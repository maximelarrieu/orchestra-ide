//! État du tableau de bord, indépendant du rendu.
//!
//! Sépare la *logique* d'agrégation du flux (compteurs, historique) du *dessin*
//! (`dashboard`). On peut ainsi tester l'agrégation sans terminal ni tokio.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use orchestra_core::events::AgentEvent;
use orchestra_core::model::{AgentDef, ContextSpace, DocKind, ProjectType, SpaceDoc};

use crate::editor::Editor;

/// Statistiques de session d'un agent (cumulées tant que l'appli tourne).
#[derive(Default)]
pub struct AgentStat {
    pub invocations: usize,
    pub thinking: Duration,
    thinking_start: Option<Instant>,
}

/// Champ d'agent en cours d'édition (saisie dans le menu Agents).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentField {
    Name,
    Role,
    Skills,
    Add,
}

impl AgentField {
    pub fn label(self) -> &'static str {
        match self {
            AgentField::Name => "Nouveau nom",
            AgentField::Role => "Rôle",
            AgentField::Skills => "Skills (séparés par des virgules)",
            AgentField::Add => "Nom du nouvel agent",
        }
    }
}

/// Nombre d'événements conservés dans l'historique du radar (les plus anciens sont
/// oubliés). Largement au-delà de ce qu'un écran affiche.
const HISTORY_CAP: usize = 500;

/// Phase de l'orchestre, déduite du flux d'événements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Rien n'a encore été lancé (état initial).
    Idle,
    /// Au moins un agent tourne.
    Running,
    /// L'orchestre a été lancé puis le canal s'est fermé (tous terminés).
    Finished,
}

/// Vue affichée dans la zone centrale du dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Flux d'activité des agents.
    Radar,
    /// Navigateur des documents de l'espace (persona, ADRs, docs).
    Docs,
    /// Gestionnaire d'agents (rôle, skills, stats, édition).
    Agents,
}

/// Visualiseur Markdown ouvert sur un document.
pub struct Viewer {
    pub title: String,
    pub text: String,
    pub scroll: usize,
    /// Vrai si le document affiché est le persona (→ raccourci d'édition).
    pub is_persona: bool,
}

pub struct App {
    pub space: Option<ContextSpace>,
    pub events: Vec<AgentEvent>,
    pub started: usize,
    pub done: usize,
    pub phase: Phase,
    /// Modèle Claude actif si `ANTHROPIC_API_KEY` est présente, sinon `None` (mode simulé).
    pub llm_model: Option<String>,
    /// Vue centrale courante (radar / ADRs).
    pub view: View,
    /// Saisie en cours d'un chemin d'espace (`Some(tampon)`), ou `None` hors saisie.
    pub input: Option<String>,
    /// Saisie en cours d'une intention pour `[1]` (`Some(tampon)`), ou `None`.
    pub intention: Option<String>,
    /// Éditeur de persona ouvert (`Some`) ou fermé (`None`).
    pub editor: Option<Editor>,
    /// Documents de l'espace (rafraîchis à l'ouverture du navigateur).
    pub docs: Vec<SpaceDoc>,
    /// Index du document sélectionné dans le navigateur.
    pub doc_sel: usize,
    /// Visualiseur Markdown ouvert (`Some`) ou fermé (`None`).
    pub viewer: Option<Viewer>,
    /// Conversation avec le coordinateur en cours : `Some(tampon de saisie)`.
    pub chat: Option<String>,
    /// Index de l'agent sélectionné dans le gestionnaire d'agents.
    pub agent_sel: usize,
    /// Édition d'un champ d'agent en cours : `Some((champ, tampon))`.
    pub agent_prompt: Option<(AgentField, String)>,
    /// Stats de session par agent (clé = nom).
    pub agent_stats: HashMap<String, AgentStat>,
    /// Défilement du radar : nombre de lignes remontées depuis le bas (0 = suit le bas).
    pub radar_scroll: usize,
    /// Agent dont un appel LLM est en cours (`Some`) — pilote l'indicateur « réfléchit… ».
    pub busy: Option<String>,
    /// Début de l'attente courante (pour afficher le temps écoulé).
    pub busy_since: Option<Instant>,
    /// Compteur d'animation du spinner (incrémenté à chaque tick).
    pub spinner: usize,
    /// Message transitoire affiché à l'utilisateur (succès/erreur d'une action).
    pub notice: Option<String>,
}

/// Cadres du spinner d'activité (braille).
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Exemple d'intention proposé par défaut selon le type de projet (`[1]`).
fn default_intention(kind: ProjectType) -> &'static str {
    match kind {
        ProjectType::Dev => "Lis le README et propose 3 améliorations prioritaires du code.",
        ProjectType::Nutrition => "Propose un plan de repas équilibré pour aujourd'hui selon mes critères.",
        ProjectType::Langue => "Donne-moi une leçon de 15 min avec quelques exercices, puis corrige mes réponses.",
        ProjectType::Immobilier => "Liste les annonces correspondant à mes critères et classe-les par pertinence.",
    }
}

impl App {
    pub fn new(space: Option<ContextSpace>) -> Self {
        Self {
            space,
            events: Vec::new(),
            started: 0,
            done: 0,
            phase: Phase::Idle,
            llm_model: orchestra_core::llm::LlmClient::from_env().map(|c| c.model().to_string()),
            view: View::Radar,
            input: None,
            intention: None,
            editor: None,
            docs: Vec::new(),
            doc_sel: 0,
            viewer: None,
            chat: None,
            agent_sel: 0,
            agent_prompt: None,
            agent_stats: HashMap::new(),
            radar_scroll: 0,
            busy: None,
            busy_since: None,
            spinner: 0,
            notice: None,
        }
    }

    /// Secondes écoulées depuis le début de l'attente courante (si un agent réfléchit).
    pub fn busy_elapsed_secs(&self) -> Option<u64> {
        self.busy_since.map(|t| t.elapsed().as_secs())
    }

    /// Avance l'animation du spinner (appelé au tick de rafraîchissement).
    pub fn tick(&mut self) {
        self.spinner = self.spinner.wrapping_add(1);
    }

    /// Cadre courant du spinner.
    pub fn spinner_frame(&self) -> &'static str {
        SPINNER[self.spinner % SPINNER.len()]
    }

    /// Fait défiler le radar (delta>0 = remonter dans l'historique). Borné en bas à 0 ; la
    /// borne haute est appliquée au rendu (selon le contenu et la hauteur).
    pub fn radar_scroll_by(&mut self, delta: isize) {
        self.radar_scroll = (self.radar_scroll as isize + delta).max(0) as usize;
    }

    /// `[5]` — démarre une conversation : radar remis à zéro, saisie de chat ouverte.
    pub fn start_chat(&mut self) {
        self.view = View::Radar;
        self.editor = None;
        self.viewer = None;
        self.begin_run(); // efface l'historique et passe en Running
        self.chat = Some(String::new());
    }

    pub fn chat_push(&mut self, c: char) {
        if let Some(buf) = self.chat.as_mut() {
            buf.push(c);
        }
    }

    pub fn chat_backspace(&mut self) {
        if let Some(buf) = self.chat.as_mut() {
            buf.pop();
        }
    }

    /// Valide le message courant : le renvoie (s'il est non vide) et vide le tampon, en
    /// restant en mode conversation.
    pub fn chat_submit(&mut self) -> Option<String> {
        let buf = self.chat.as_mut()?;
        let msg = buf.trim().to_string();
        buf.clear();
        if msg.is_empty() {
            None
        } else {
            self.radar_scroll = 0; // revient au bas pour suivre la réponse
            Some(msg)
        }
    }

    pub fn end_chat(&mut self) {
        self.chat = None;
    }

    /// `[4]` — ouvre l'éditeur de persona sur le contenu courant de l'espace.
    pub fn open_persona_editor(&mut self) {
        if let Some(space) = &self.space {
            let text = space.persona.clone().unwrap_or_default();
            self.editor = Some(Editor::from_str(&text));
            self.notice = None;
        } else {
            self.notice = Some("Aucun espace chargé : impossible d'éditer le persona.".into());
        }
    }

    /// `[2]` — bascule entre le radar et le navigateur de documents (rafraîchit la liste).
    pub fn toggle_docs(&mut self) {
        if self.view == View::Docs {
            self.view = View::Radar;
            self.viewer = None;
            return;
        }
        self.docs = self.space.as_ref().map(ContextSpace::documents).unwrap_or_default();
        self.doc_sel = 0;
        self.viewer = None;
        self.notice = None;
        self.view = View::Docs;
    }

    /// Déplace la sélection dans la liste de documents (bornée).
    pub fn docs_move(&mut self, delta: isize) {
        if self.docs.is_empty() {
            return;
        }
        let last = self.docs.len() - 1;
        let next = (self.doc_sel as isize + delta).clamp(0, last as isize);
        self.doc_sel = next as usize;
    }

    /// Ouvre le document sélectionné dans le visualiseur Markdown (lecture via le cœur).
    pub fn open_selected_doc(&mut self) {
        let Some(doc) = self.docs.get(self.doc_sel) else { return };
        match orchestra_core::model::load_document(&doc.path) {
            Ok(text) => {
                self.viewer = Some(Viewer {
                    title: doc.label.clone(),
                    text,
                    scroll: 0,
                    is_persona: doc.kind == DocKind::Persona,
                });
            }
            Err(e) => self.notice = Some(format!("Lecture impossible : {e}")),
        }
    }

    pub fn close_viewer(&mut self) {
        self.viewer = None;
    }

    /// Fait défiler le visualiseur (clamp bas géré au rendu d'après la hauteur dispo).
    pub fn viewer_scroll(&mut self, delta: isize) {
        if let Some(v) = self.viewer.as_mut() {
            v.scroll = (v.scroll as isize + delta).max(0) as usize;
        }
    }

    /// Vrai si le visualiseur affiche le persona (→ raccourci `e` pour l'éditer).
    pub fn viewer_is_persona(&self) -> bool {
        self.viewer.as_ref().is_some_and(|v| v.is_persona)
    }

    /// `[3]` — entre en saisie d'un chemin d'espace.
    pub fn start_space_input(&mut self) {
        self.input = Some(String::new());
        self.notice = None;
    }

    pub fn input_push(&mut self, c: char) {
        if let Some(buf) = self.input.as_mut() {
            buf.push(c);
        }
    }

    pub fn input_backspace(&mut self) {
        if let Some(buf) = self.input.as_mut() {
            buf.pop();
        }
    }

    pub fn cancel_input(&mut self) {
        self.input = None;
    }

    /// Termine la saisie et renvoie le chemin saisi (vidé si vide).
    pub fn take_input(&mut self) -> Option<String> {
        self.input.take().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }

    /// `[1]` — entre en saisie d'une intention, pré-remplie d'un exemple selon le type de
    /// projet (l'utilisateur l'édite ou la valide telle quelle).
    pub fn start_intention(&mut self) {
        let example = self
            .space
            .as_ref()
            .map(|s| default_intention(s.config.project_type))
            .unwrap_or("");
        self.intention = Some(example.to_string());
        self.notice = None;
    }

    pub fn intention_push(&mut self, c: char) {
        if let Some(buf) = self.intention.as_mut() {
            buf.push(c);
        }
    }

    pub fn intention_backspace(&mut self) {
        if let Some(buf) = self.intention.as_mut() {
            buf.pop();
        }
    }

    pub fn cancel_intention(&mut self) {
        self.intention = None;
    }

    /// Termine la saisie d'intention et renvoie l'objectif (vidé si vide).
    pub fn take_intention(&mut self) -> Option<String> {
        self.intention.take().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }

    /// Vrai si un Espace valide est chargé : sans lui, pas d'agents à lancer.
    pub fn can_launch(&self) -> bool {
        self.space
            .as_ref()
            .is_some_and(|s| !s.config.agents.is_empty())
    }

    /// Vrai si le persona contient encore des placeholders « à compléter » : lancer un
    /// vrai LLM dans ce cas gaspille un appel (l'agent dira qu'il manque de contexte).
    pub fn persona_incomplete(&self) -> bool {
        self.space
            .as_ref()
            .and_then(|s| s.persona.as_deref())
            .is_some_and(|p| p.contains("à compléter"))
    }

    /// Remet le radar à zéro et passe en `Running` (appelé quand l'utilisateur lance
    /// l'orchestre). L'historique précédent est effacé pour une lecture nette.
    pub fn begin_run(&mut self) {
        self.events.clear();
        self.started = 0;
        self.done = 0;
        self.phase = Phase::Running;
        self.radar_scroll = 0;
        self.busy = None;
        self.busy_since = None;
    }

    /// Intègre un événement du runtime dans l'état (compteurs + historique + stats agents).
    pub fn on_event(&mut self, ev: AgentEvent) {
        // « Thinking » ne va pas dans l'historique : il pilote l'indicateur + chronomètre.
        if let AgentEvent::Thinking { agent } = &ev {
            self.busy = Some(agent.clone());
            self.busy_since = Some(Instant::now());
            self.agent_stats.entry(agent.clone()).or_default().thinking_start = Some(Instant::now());
            return;
        }
        self.busy = None; // une sortie est apparue → plus en attente
        self.busy_since = None;

        match &ev {
            AgentEvent::Started { agent } => {
                self.agent_stats.entry(agent.clone()).or_default().invocations += 1;
            }
            AgentEvent::Done { agent } | AgentEvent::Log { agent, .. } => {
                let st = self.agent_stats.entry(agent.clone()).or_default();
                if let Some(start) = st.thinking_start.take() {
                    st.thinking += start.elapsed();
                }
            }
            AgentEvent::Thinking { .. } => {}
        }
        match &ev {
            AgentEvent::Started { .. } => self.started += 1,
            AgentEvent::Done { .. } => self.done += 1,
            _ => {}
        }
        self.events.push(ev);
        if self.events.len() > HISTORY_CAP {
            self.events.remove(0);
        }
    }

    // --- Gestionnaire d'agents (`[6]`) ---------------------------------------------

    /// `[6]` — bascule entre le radar et le gestionnaire d'agents.
    pub fn toggle_agents(&mut self) {
        if self.view == View::Agents {
            self.view = View::Radar;
            self.agent_prompt = None;
            return;
        }
        self.agent_sel = 0;
        self.agent_prompt = None;
        self.notice = None;
        self.view = View::Agents;
    }

    fn agent_count(&self) -> usize {
        self.space.as_ref().map(|s| s.config.agents.len()).unwrap_or(0)
    }

    pub fn agents_move(&mut self, delta: isize) {
        let n = self.agent_count();
        if n == 0 {
            return;
        }
        let next = (self.agent_sel as isize + delta).clamp(0, n as isize - 1);
        self.agent_sel = next as usize;
    }

    /// Agent sélectionné (lecture).
    pub fn selected_agent(&self) -> Option<&AgentDef> {
        self.space.as_ref()?.config.agents.get(self.agent_sel)
    }

    pub fn start_agent_rename(&mut self) {
        if let Some(a) = self.selected_agent() {
            self.agent_prompt = Some((AgentField::Name, a.name.clone()));
        }
    }
    pub fn start_agent_role(&mut self) {
        if let Some(a) = self.selected_agent() {
            self.agent_prompt = Some((AgentField::Role, a.role.clone()));
        }
    }
    pub fn start_agent_skills(&mut self) {
        if let Some(a) = self.selected_agent() {
            self.agent_prompt = Some((AgentField::Skills, a.skills.join(", ")));
        }
    }
    pub fn start_agent_add(&mut self) {
        if self.space.is_some() {
            self.agent_prompt = Some((AgentField::Add, String::new()));
        }
    }

    pub fn agent_prompt_push(&mut self, c: char) {
        if let Some((_, buf)) = self.agent_prompt.as_mut() {
            buf.push(c);
        }
    }
    pub fn agent_prompt_backspace(&mut self) {
        if let Some((_, buf)) = self.agent_prompt.as_mut() {
            buf.pop();
        }
    }
    pub fn cancel_agent_prompt(&mut self) {
        self.agent_prompt = None;
    }

    /// Valide la saisie en cours et persiste les agents dans `config.json` (via le cœur).
    pub fn submit_agent_prompt(&mut self) {
        let Some((field, buf)) = self.agent_prompt.take() else { return };
        let value = buf.trim().to_string();
        let sel = self.agent_sel;
        let Some(space) = self.space.as_mut() else { return };
        match field {
            AgentField::Name => {
                if !value.is_empty() {
                    if let Some(a) = space.config.agents.get_mut(sel) {
                        a.name = value;
                    }
                }
            }
            AgentField::Role => {
                if let Some(a) = space.config.agents.get_mut(sel) {
                    a.role = value;
                }
            }
            AgentField::Skills => {
                if let Some(a) = space.config.agents.get_mut(sel) {
                    a.skills = value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
            AgentField::Add => {
                if !value.is_empty() {
                    space.config.agents.push(AgentDef::new(value));
                    self.agent_sel = space.config.agents.len() - 1;
                }
            }
        }
        self.persist_config();
    }

    /// Supprime l'agent sélectionné et persiste.
    pub fn delete_selected_agent(&mut self) {
        let sel = self.agent_sel;
        let Some(space) = self.space.as_mut() else { return };
        if sel < space.config.agents.len() {
            let removed = space.config.agents.remove(sel);
            if self.agent_sel >= space.config.agents.len() {
                self.agent_sel = space.config.agents.len().saturating_sub(1);
            }
            self.persist_config();
            if self.notice.is_none() {
                self.notice = Some(format!("Agent « {} » supprimé.", removed.name));
            }
        }
    }

    fn persist_config(&mut self) {
        if let Some(space) = self.space.as_ref() {
            match space.save_config() {
                Ok(()) => self.notice = Some("Agents enregistrés.".into()),
                Err(e) => self.notice = Some(format!("Échec enregistrement : {e}")),
            }
        }
    }

    /// Signalé par la boucle principale quand le canal se ferme (tous les agents finis).
    pub fn mark_finished(&mut self) {
        self.busy = None;
        self.busy_since = None;
        if self.phase == Phase::Running {
            self.phase = Phase::Finished;
        }
    }

    /// Agents encore en cours (démarrés mais pas terminés).
    pub fn running_count(&self) -> usize {
        self.started.saturating_sub(self.done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_started_and_done() {
        let mut app = App::new(None);
        app.begin_run();
        app.on_event(AgentEvent::Started { agent: "A".into() });
        app.on_event(AgentEvent::Log { agent: "A".into(), msg: "x".into() });
        app.on_event(AgentEvent::Started { agent: "B".into() });
        app.on_event(AgentEvent::Done { agent: "A".into() });

        assert_eq!(app.started, 2);
        assert_eq!(app.done, 1);
        assert_eq!(app.running_count(), 1);
        assert_eq!(app.events.len(), 4);

        app.mark_finished();
        assert_eq!(app.phase, Phase::Finished);
    }

    #[test]
    fn history_is_capped() {
        let mut app = App::new(None);
        for _ in 0..(HISTORY_CAP + 50) {
            app.on_event(AgentEvent::Log { agent: "A".into(), msg: "x".into() });
        }
        assert_eq!(app.events.len(), HISTORY_CAP);
    }

    #[test]
    fn toggle_docs_switches_view() {
        let mut app = App::new(None);
        assert_eq!(app.view, View::Radar);
        app.toggle_docs();
        assert_eq!(app.view, View::Docs);
        app.toggle_docs();
        assert_eq!(app.view, View::Radar);
    }

    #[test]
    fn docs_move_is_bounded() {
        let mut app = App::new(None);
        app.docs_move(5); // liste vide → reste à 0
        assert_eq!(app.doc_sel, 0);
    }

    #[test]
    fn thinking_sets_busy_without_polluting_history() {
        let mut app = App::new(None);
        app.on_event(AgentEvent::Thinking { agent: "Coordinateur".into() });
        assert_eq!(app.busy.as_deref(), Some("Coordinateur"));
        assert!(app.events.is_empty(), "Thinking ne va pas dans l'historique");
        // Une sortie efface l'indicateur d'attente.
        app.on_event(AgentEvent::Log { agent: "Coordinateur".into(), msg: "ok".into() });
        assert!(app.busy.is_none());
        assert_eq!(app.events.len(), 1);
    }

    #[test]
    fn agent_add_persists_to_config() {
        use orchestra_core::model::project_type::ProjectType;
        use orchestra_core::{scaffold_space, InitOptions};

        let dir = std::env::temp_dir().join(format!("orch-agents-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        scaffold_space(
            &dir,
            InitOptions {
                project_name: "T".into(),
                project_type: ProjectType::Dev,
                workspace_path: None,
                documentalist_enabled: false,
                integrations: Default::default(),
            },
        )
        .unwrap();

        let mut app = App::new(Some(ContextSpace::load(&dir).unwrap()));
        app.toggle_agents();
        app.start_agent_add();
        for c in "Agent_X".chars() {
            app.agent_prompt_push(c);
        }
        app.submit_agent_prompt();

        let reloaded = ContextSpace::load(&dir).unwrap();
        assert!(reloaded.config.agents.iter().any(|a| a.name == "Agent_X"),
            "le nouvel agent doit être persisté dans config.json");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn radar_scroll_clamps_at_zero() {
        let mut app = App::new(None);
        app.radar_scroll_by(5);
        assert_eq!(app.radar_scroll, 5);
        app.radar_scroll_by(-100); // ne descend pas sous 0
        assert_eq!(app.radar_scroll, 0);
        app.radar_scroll_by(3);
        app.begin_run(); // un nouveau lancement revient en bas
        assert_eq!(app.radar_scroll, 0);
    }

    #[test]
    fn chat_input_submit_and_end() {
        let mut app = App::new(None);
        app.start_chat();
        assert!(app.chat.is_some());
        app.chat_push('h');
        app.chat_push('i');
        app.chat_backspace();
        assert_eq!(app.chat_submit().as_deref(), Some("h"));
        assert_eq!(app.chat.as_deref(), Some(""), "le tampon est vidé mais le chat reste actif");
        assert_eq!(app.chat_submit(), None, "message vide → rien envoyé");
        app.end_chat();
        assert!(app.chat.is_none());
    }

    #[test]
    fn chat_message_can_be_multiline() {
        let mut app = App::new(None);
        app.start_chat();
        app.chat_push('a');
        app.chat_push('\n'); // Maj+Entrée
        app.chat_push('b');
        assert_eq!(app.chat_submit().as_deref(), Some("a\nb"));
    }

    #[test]
    fn space_input_buffer_edit_and_take() {
        let mut app = App::new(None);
        app.start_space_input();
        app.input_push('a');
        app.input_push('b');
        app.input_backspace();
        app.input_push('c');
        assert_eq!(app.input.as_deref(), Some("ac"));
        assert_eq!(app.take_input().as_deref(), Some("ac"));
        assert!(app.input.is_none(), "la saisie est consommée");
    }

    #[test]
    fn intention_input_edit_and_take() {
        let mut app = App::new(None);
        app.start_intention();
        app.intention_push('g');
        app.intention_push('o');
        app.intention_backspace();
        app.intention_push('o');
        assert_eq!(app.take_intention().as_deref(), Some("go"));
        assert!(app.intention.is_none(), "la saisie est consommée");
    }

    #[test]
    fn empty_space_input_yields_none() {
        let mut app = App::new(None);
        app.start_space_input();
        app.input_push(' ');
        assert_eq!(app.take_input(), None, "saisie vide → aucun chemin");
    }

    #[test]
    fn detects_incomplete_persona() {
        use orchestra_core::model::config::{AgentDef, ProjectConfig};
        use orchestra_core::model::project_type::ProjectType;
        let mk = |persona: Option<&str>| {
            let space = ContextSpace {
                root: std::path::PathBuf::from("."),
                config: ProjectConfig {
                    project_name: "T".into(),
                    project_type: ProjectType::Immobilier,
                    workspace_path: None,
                    documentalist_enabled: false,
                    skills: vec![],
                    agents: vec![AgentDef::new("A")],
                    integrations: Default::default(),
                },
                persona: persona.map(str::to_string),
                adrs: vec![],
            };
            App::new(Some(space))
        };
        assert!(mk(Some("Budget : à compléter")).persona_incomplete());
        assert!(!mk(Some("Budget : 350k€")).persona_incomplete());
        assert!(!mk(None).persona_incomplete(), "pas de persona → pas bloquant");
    }

    /// L'espace d'exemple livré doit se charger ET être lançable (sinon `[1]` ne fait
    /// rien). Ce test reproduit ce que voit l'utilisateur qui ouvre cet espace.
    #[test]
    fn bundled_example_space_can_launch() {
        let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/recherche-immo-aix");
        let space = ContextSpace::load(&example).expect("l'exemple doit se charger");
        let app = App::new(Some(space));
        assert!(app.can_launch(), "l'exemple doit avoir des agents → [1] actif");
    }
}
