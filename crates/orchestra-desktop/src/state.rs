//! État de l'app + **ponts vers le cœur** : isole la logique « parler à `orchestra-core` »
//! du rendu. Les composants ([`crate::components`]) se contentent d'afficher / déclencher.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use dioxus::prelude::*;
use orchestra_core::events::AgentEvent;
use orchestra_core::model::ContextSpace;
use orchestra_core::runtime;
use orchestra_core::session::{Sessions, Tabbed};
use tokio::sync::mpsc::UnboundedSender;

// Registre des espaces connus (récents) — partagé avec le TUI.
pub use orchestra_core::registry::KnownSpace;

/// Activité d'un agent sur un fichier (« orchestre en verre ») : qui l'a touché en dernier, et
/// si c'était une écriture (sinon une lecture). Sert à annoter l'explorateur en temps réel.
#[derive(Clone, PartialEq, Eq)]
pub struct FileActivity {
    pub agent: String,
    pub write: bool,
}

/// Une commande exécutée par un agent (pour le panneau Terminal).
#[derive(Clone, PartialEq, Eq)]
pub struct TermEntry {
    pub agent: String,
    pub command: String,
    pub output: String,
    pub ok: bool,
}

/// État vivant d'une **session** (un onglet) : l'Espace + toute sa conversation. Chaque session
/// garde **son propre contexte et son historique** ; on passe de l'une à l'autre sans rien
/// perdre, et une session en arrière-plan continue de recevoir ses événements.
pub struct DesktopSession {
    pub space: ContextSpace,
    pub messages: Vec<ChatMsg>,
    pub thinking: bool,
    pub plan: Vec<PlanRow>,
    pub pending: bool,
    pub changes: Vec<FileChange>,
    pub status: HashMap<String, AgStatus>,
    /// Activité live des agents par fichier (chemin relatif → dernier accès). Annote l'explorateur.
    pub activity: HashMap<String, FileActivity>,
    /// Historique des commandes exécutées (panneau Terminal).
    pub terminal: Vec<TermEntry>,
    /// Canal d'envoi des messages au coordinateur (présent une fois la conversation démarrée).
    pub user_tx: Option<UnboundedSender<String>>,
    /// Canal d'approbation de plan.
    pub approve_tx: Option<UnboundedSender<bool>>,
    /// Vrai une fois la conversation lancée (évite de la relancer à chaque rendu).
    pub started: bool,
}

impl DesktopSession {
    pub fn new(space: ContextSpace) -> Self {
        Self {
            space,
            messages: Vec::new(),
            thinking: false,
            plan: Vec::new(),
            pending: false,
            changes: Vec::new(),
            status: HashMap::new(),
            activity: HashMap::new(),
            terminal: Vec::new(),
            user_tx: None,
            approve_tx: None,
            started: false,
        }
    }

    /// Racine de travail (workspace de code si défini, sinon la racine de l'espace) — base de
    /// l'arborescence de l'explorateur.
    pub fn workspace_root(&self) -> &std::path::Path {
        self.space.config.workspace_path.as_deref().unwrap_or(&self.space.root)
    }
}

impl Tabbed for DesktopSession {
    fn title(&self) -> String {
        self.space.config.project_name.clone()
    }
    fn root(&self) -> &Path {
        &self.space.root
    }
}

/// Nature d'un message de chat (pour le style de bulle).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MsgKind {
    User,
    Coordinator,
    Agent,
    System,
}

/// Un message affiché dans la conversation.
#[derive(Clone, PartialEq)]
pub struct ChatMsg {
    pub who: String,
    pub text: String,
    pub kind: MsgKind,
}

/// Une ligne du panneau Plan.
#[derive(Clone, PartialEq)]
pub struct PlanRow {
    pub id: String,
    pub agent: String,
    pub status: String,
    pub objective: String,
}

/// Un fichier modifié par un agent (pour la vue Modifications).
#[derive(Clone, PartialEq)]
pub struct FileChange {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    pub diff: String,
}

/// Statut live d'un agent (pour l'encart « squad »).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AgStatus {
    Idle,
    Thinking,
    Working,
    Done,
}

impl AgStatus {
    pub fn icon(self) -> &'static str {
        match self {
            AgStatus::Idle => "○",
            AgStatus::Thinking => "⏳",
            AgStatus::Working => "▸",
            AgStatus::Done => "✔",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            AgStatus::Idle => "en attente",
            AgStatus::Thinking => "réfléchit…",
            AgStatus::Working => "actif",
            AgStatus::Done => "terminé",
        }
    }
}

/// Met à jour le statut live d'un agent d'une session (ignore l'écho utilisateur « Vous »).
fn mark(sess: &mut DesktopSession, name: &str, st: AgStatus) {
    if name != "Vous" {
        sess.status.insert(name.to_string(), st);
    }
}

/// Convertit une tâche planifiée (cœur) en ligne de plan affichable.
fn plan_row(t: orchestra_core::events::PlannedTask) -> PlanRow {
    PlanRow {
        id: t.id,
        agent: t.agent,
        status: "en attente".into(),
        objective: t.objective,
    }
}

fn set_status(sess: &mut DesktopSession, id: &str, status: &str) {
    if let Some(row) = sess.plan.iter_mut().find(|r| r.id == id) {
        row.status = status.to_string();
    }
}

/// Applique un événement du runtime à l'état d'**une** session (celle qui l'a émis).
fn apply_event(sess: &mut DesktopSession, ev: AgentEvent) {
    match ev {
        AgentEvent::Thinking { agent } => {
            sess.thinking = true;
            mark(sess, &agent, AgStatus::Thinking);
        }
        AgentEvent::Log { agent, msg } => {
            sess.thinking = false;
            mark(sess, &agent, AgStatus::Working);
            let kind = if agent == "Vous" {
                MsgKind::User
            } else if agent == runtime::COORDINATOR {
                MsgKind::Coordinator
            } else {
                MsgKind::Agent
            };
            sess.messages.push(ChatMsg { who: agent, text: msg, kind });
        }
        AgentEvent::Started { agent } => {
            mark(sess, &agent, AgStatus::Working);
            sess.messages.push(ChatMsg {
                who: agent,
                text: "rejoint la conversation".into(),
                kind: MsgKind::System,
            });
        }
        AgentEvent::Done { agent } => {
            sess.thinking = false;
            mark(sess, &agent, AgStatus::Done);
        }
        AgentEvent::PlanReady { tasks } => {
            sess.plan = tasks.into_iter().map(plan_row).collect();
            sess.pending = true;
        }
        AgentEvent::TaskStarted { id, agent } => {
            set_status(sess, &id, "en cours");
            mark(sess, &agent, AgStatus::Working);
        }
        AgentEvent::TaskDone { id } => set_status(sess, &id, "fait ✓"),
        AgentEvent::TaskFailed { id, .. } => set_status(sess, &id, "échec ✗"),
        AgentEvent::Terminal { agent, command, output, ok } => {
            mark(sess, &agent, AgStatus::Working);
            sess.terminal.push(TermEntry { agent, command, output, ok });
        }
        AgentEvent::FileRead { agent, path } => {
            mark(sess, &agent, AgStatus::Working);
            sess.activity.insert(path, FileActivity { agent, write: false });
        }
        AgentEvent::FileChanged { agent, path, added, removed, diff } => {
            mark(sess, &agent, AgStatus::Working);
            sess.activity.insert(path.clone(), FileActivity { agent, write: true });
            sess.changes.push(FileChange { path, added, removed, diff });
        }
    }
}

/// Démarre la **conversation avec le coordinateur** (équivalent `[5]` du TUI) pour la session
/// d'index `index` : ouvre le canal du cœur, mémorise les `Sender` **dans la session**, et
/// **streame les événements dans cette session** (repérée par sa racine). Les autres onglets ne
/// sont pas touchés ; une session en arrière-plan continue donc de vivre.
pub fn start_session_chat(mut sessions: Signal<Sessions<DesktopSession>>, index: usize) {
    // Espace de la session ciblée (clone pour lancer le runtime hors du verrou).
    let space = {
        let s = sessions.read();
        match s.get(index) {
            Some(sess) => sess.space.clone(),
            None => return,
        }
    };
    let root = space.root.clone();
    let handle = runtime::start_conversation(&space);

    // (Re)démarre proprement le contexte de la session + mémorise ses canaux.
    {
        let mut s = sessions.write();
        if let Some(sess) = s.get_mut(index) {
            sess.messages.clear();
            sess.plan.clear();
            sess.pending = false;
            sess.thinking = false;
            sess.status.clear();
            sess.changes.clear();
            sess.activity.clear();
            sess.terminal.clear();
            sess.user_tx = Some(handle.user);
            sess.approve_tx = Some(handle.approve);
            sess.started = true;
        }
    }

    let mut events = handle.events;
    spawn(async move {
        while let Some(ev) = events.recv().await {
            let mut s = sessions.write();
            // La session peut avoir été fermée entre-temps : on la retrouve par sa racine.
            match s.index_of(&root) {
                Some(i) => {
                    if let Some(sess) = s.get_mut(i) {
                        apply_event(sess, ev);
                    }
                }
                None => break,
            }
        }
    });
}

/// Enregistre un document quelconque de l'espace (persona, memory, ADR, `.md`) via le cœur.
pub fn save_document(path: &Path, content: &str) -> bool {
    orchestra_core::model::save_document(path, content).is_ok()
}

/// Lit un fichier du workspace (`root` + chemin relatif) en UTF-8. `None` si illisible ou binaire.
/// Alimente le panneau central de l'explorateur.
pub fn read_file_rel(root: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(root.join(rel)).ok()
}

/// Notes de la mémoire partagée de l'espace (`.orchestra/memory.md`), plus récentes d'abord.
/// Alimente le panneau « Memory ».
pub fn memory_entries(root: &Path) -> Vec<String> {
    orchestra_core::memory::entries(root)
}

// --- Espaces : registre des espaces connus (récents) ---------------------------------------

/// Espaces connus (récents d'abord), depuis le registre global.
pub fn known_spaces() -> Vec<KnownSpace> {
    orchestra_core::registry::known_spaces()
}

/// Ouvre un espace depuis un chemin **dans un onglet** (ou réactive le sien s'il est déjà
/// ouvert) : charge, ouvre la session, **mémorise** dans le registre et rafraîchit la liste des
/// espaces connus. Renvoie `true` si chargé.
pub fn open_space(
    mut sessions: Signal<Sessions<DesktopSession>>,
    mut known: Signal<Vec<KnownSpace>>,
    path: &str,
) -> bool {
    let pb = PathBuf::from(path);
    match ContextSpace::load(&pb) {
        Ok(sp) => {
            sessions.write().open(DesktopSession::new(sp));
            let _ = orchestra_core::registry::remember_space(&pb);
            known.set(orchestra_core::registry::known_spaces());
            true
        }
        Err(_) => false,
    }
}

/// Retire un espace du registre (« ne plus suivre ») et rafraîchit la liste.
pub fn forget_space_entry(mut known: Signal<Vec<KnownSpace>>, path: &Path) {
    orchestra_core::registry::forget_space(path);
    known.set(orchestra_core::registry::known_spaces());
}

/// **Reprend un projet existant** (`path`) : initialise `.orchestra` dedans (workspace = `path`),
/// l'ouvre **dans un onglet** et le mémorise. `true` si réussi.
pub fn adopt_project(
    mut sessions: Signal<Sessions<DesktopSession>>,
    mut known: Signal<Vec<KnownSpace>>,
    path: &Path,
) -> bool {
    match orchestra_core::scaffold::adopt_project(path) {
        Ok(sp) => {
            let _ = orchestra_core::registry::remember_space(path);
            sessions.write().open(DesktopSession::new(sp));
            known.set(orchestra_core::registry::known_spaces());
            true
        }
        Err(_) => false,
    }
}

/// Slug de dossier sûr (minuscules ; non-alphanumérique → `-`).
fn slug(name: &str) -> String {
    let s: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "espace".to_string() } else { s }
}

/// Crée un nouvel espace (`parent/<slug(nom)>`) via le cœur, l'ouvre **dans un onglet** et le
/// mémorise.
pub fn create_space(
    mut sessions: Signal<Sessions<DesktopSession>>,
    mut known: Signal<Vec<KnownSpace>>,
    parent: &str,
    name: &str,
    workspace: &str,
    objectives: &str,
) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Le nom du projet est obligatoire.".into());
    }
    let root = PathBuf::from(parent.trim()).join(slug(name));
    let workspace_path = if workspace.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(workspace.trim()))
    };
    let opts = orchestra_core::InitOptions {
        project_name: name.to_string(),
        workspace_path,
        integrations: Default::default(),
        objectives: objectives.to_string(),
    };
    match orchestra_core::scaffold_space(&root, opts) {
        Ok(sp) => {
            let _ = orchestra_core::registry::remember_space(&root);
            sessions.write().open(DesktopSession::new(sp));
            known.set(orchestra_core::registry::known_spaces());
            Ok(())
        }
        Err(e) => Err(format!("Échec de la création : {e}")),
    }
}

/// Convertit du Markdown en **HTML** pour le visualiseur de documents (titres, listes, code,
/// tableaux…). Les blocs ` ```mermaid ` sont transformés en `<pre class="mermaid">` afin d'être
/// rendus visuellement par mermaid.js dans la webview (cf. `DocumentsView`).
pub fn render_markdown_html(md: &str) -> String {
    use pulldown_cmark::{html, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let mut events: Vec<Event> = Vec::new();
    let mut in_mermaid = false;
    let mut buf = String::new();
    for ev in Parser::new_ext(md, options) {
        match ev {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) if lang.as_ref() == "mermaid" => {
                in_mermaid = true;
                buf.clear();
            }
            Event::Text(t) if in_mermaid => buf.push_str(&t),
            Event::End(TagEnd::CodeBlock) if in_mermaid => {
                in_mermaid = false;
                let escaped = buf.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                events.push(Event::Html(format!("<pre class=\"mermaid\">{escaped}</pre>").into()));
            }
            other => events.push(other),
        }
    }

    let mut out = String::new();
    html::push_html(&mut out, events.into_iter());
    out
}
