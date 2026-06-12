//! État du tableau de bord, indépendant du rendu.
//!
//! Sépare la *logique* d'agrégation du flux (compteurs, historique) du *dessin*
//! (`dashboard`). On peut ainsi tester l'agrégation sans terminal ni tokio.

use orchestra_core::events::AgentEvent;
use orchestra_core::model::ContextSpace;

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

/// Vue affichée dans la zone centrale du dashboard (Phase 5, finitions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Flux d'activité des agents.
    Radar,
    /// Liste des ADRs de l'espace.
    Adrs,
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
    /// Message transitoire affiché à l'utilisateur (succès/erreur d'une action).
    pub notice: Option<String>,
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
            notice: None,
        }
    }

    /// `[2]` — bascule entre le radar et la liste des ADRs.
    pub fn toggle_adrs(&mut self) {
        self.view = match self.view {
            View::Adrs => View::Radar,
            _ => View::Adrs,
        };
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
    }

    /// Intègre un événement du runtime dans l'état (compteurs + historique borné).
    pub fn on_event(&mut self, ev: AgentEvent) {
        match &ev {
            AgentEvent::Started { .. } => self.started += 1,
            AgentEvent::Done { .. } => self.done += 1,
            AgentEvent::Log { .. } => {}
        }
        self.events.push(ev);
        if self.events.len() > HISTORY_CAP {
            self.events.remove(0);
        }
    }

    /// Signalé par la boucle principale quand le canal se ferme (tous les agents finis).
    pub fn mark_finished(&mut self) {
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
    fn toggle_adrs_switches_view() {
        let mut app = App::new(None);
        assert_eq!(app.view, View::Radar);
        app.toggle_adrs();
        assert_eq!(app.view, View::Adrs);
        app.toggle_adrs();
        assert_eq!(app.view, View::Radar);
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
    fn empty_space_input_yields_none() {
        let mut app = App::new(None);
        app.start_space_input();
        app.input_push(' ');
        assert_eq!(app.take_input(), None, "saisie vide → aucun chemin");
    }

    #[test]
    fn detects_incomplete_persona() {
        use orchestra_core::model::config::ProjectConfig;
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
                    agents: vec!["A".into()],
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
