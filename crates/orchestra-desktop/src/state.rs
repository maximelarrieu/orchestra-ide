//! État de l'app + **ponts vers le cœur** : isole la logique « parler à `orchestra-core` »
//! du rendu. Les composants ([`crate::components`]) se contentent d'afficher / déclencher.

use std::path::{Path, PathBuf};

use dioxus::prelude::*;
use orchestra_core::events::AgentEvent;
use orchestra_core::model::{AgentDef, ContextSpace};
use orchestra_core::runtime;
use tokio::sync::mpsc::UnboundedSender;

// Types et catalogue de skills : **partagés avec le TUI** via `orchestra-core::catalog`, pour
// garantir un comportement identique (cf. CLAUDE.md, règle de parité TUI ⇄ GUI).
pub use orchestra_core::catalog::{skill_entries, SkillEntry, SkillKind};
// Registre des espaces connus (récents) — partagé avec le TUI.
pub use orchestra_core::registry::KnownSpace;

/// Objectif par défaut proposé dans la zone de saisie.
pub const DEFAULT_GOAL: &str = "Avance concrètement sur l'objectif de cet espace.";

/// Vue centrale courante (équivalent des touches du TUI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Orchestrate,
    Chat,
    Documents,
    Agents,
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
}

/// Lance l'orchestration et **streame les [`AgentEvent`]** dans les signaux fournis. Appelé
/// depuis un gestionnaire d'événement (le `spawn` Dioxus tourne dans le scope réactif courant).
pub fn drive_orchestration(
    space: ContextSpace,
    objective: String,
    mut log: Signal<Vec<String>>,
    mut plan: Signal<Vec<PlanRow>>,
    mut pending: Signal<bool>,
    mut approve_tx: Signal<Option<UnboundedSender<bool>>>,
) {
    log.set(Vec::new());
    plan.set(Vec::new());
    pending.set(false);

    spawn(async move {
        let handle = runtime::orchestrate(&space, &objective);
        approve_tx.set(Some(handle.approve));
        let mut events = handle.events;
        while let Some(ev) = events.recv().await {
            match ev {
                AgentEvent::PlanReady { tasks } => {
                    plan.set(
                        tasks
                            .into_iter()
                            .map(|t| PlanRow { id: t.id, agent: t.agent, status: "en attente".into() })
                            .collect(),
                    );
                    pending.set(true);
                }
                AgentEvent::TaskStarted { id, .. } => set_status(&mut plan, &id, "en cours"),
                AgentEvent::TaskDone { id } => set_status(&mut plan, &id, "fait ✓"),
                AgentEvent::TaskFailed { id, .. } => set_status(&mut plan, &id, "échec ✗"),
                AgentEvent::Started { agent } => log.write().push(format!("▶ {agent}")),
                AgentEvent::Done { agent } => log.write().push(format!("✔ {agent}")),
                AgentEvent::Log { agent, msg } => log.write().push(format!("{agent} : {msg}")),
                AgentEvent::Thinking { .. } => {}
            }
        }
    });
}

fn set_status(plan: &mut Signal<Vec<PlanRow>>, id: &str, status: &str) {
    let mut rows = plan.write();
    if let Some(row) = rows.iter_mut().find(|r| r.id == id) {
        row.status = status.to_string();
    }
}

/// Démarre une **conversation avec le coordinateur** (équivalent `[5]` du TUI) : ouvre le
/// canal bidirectionnel du cœur, mémorise les `Sender` (messages utilisateur + approbation de
/// plan) et **streame les événements** dans les signaux du chat. Un plan proposé en cours de
/// conversation passe par `plan`/`pending` (mêmes signaux que l'orchestration directe).
#[allow(clippy::too_many_arguments)]
pub fn start_chat(
    space: ContextSpace,
    mut user_tx: Signal<Option<UnboundedSender<String>>>,
    mut messages: Signal<Vec<ChatMsg>>,
    mut thinking: Signal<bool>,
    mut plan: Signal<Vec<PlanRow>>,
    mut pending: Signal<bool>,
    mut approve_tx: Signal<Option<UnboundedSender<bool>>>,
) {
    messages.set(Vec::new());
    plan.set(Vec::new());
    pending.set(false);
    thinking.set(false);

    let handle = runtime::start_conversation(&space);
    user_tx.set(Some(handle.user));
    approve_tx.set(Some(handle.approve));
    let mut events = handle.events;

    spawn(async move {
        while let Some(ev) = events.recv().await {
            match ev {
                AgentEvent::Thinking { .. } => thinking.set(true),
                AgentEvent::Log { agent, msg } => {
                    thinking.set(false);
                    let kind = if agent == "Vous" {
                        MsgKind::User
                    } else if agent == runtime::COORDINATOR {
                        MsgKind::Coordinator
                    } else {
                        MsgKind::Agent
                    };
                    messages.write().push(ChatMsg { who: agent, text: msg, kind });
                }
                AgentEvent::Started { agent } => messages
                    .write()
                    .push(ChatMsg { who: agent, text: "rejoint la conversation".into(), kind: MsgKind::System }),
                AgentEvent::Done { .. } => thinking.set(false),
                AgentEvent::PlanReady { tasks } => {
                    plan.set(
                        tasks
                            .into_iter()
                            .map(|t| PlanRow { id: t.id, agent: t.agent, status: "en attente".into() })
                            .collect(),
                    );
                    pending.set(true);
                }
                AgentEvent::TaskStarted { id, .. } => set_status(&mut plan, &id, "en cours"),
                AgentEvent::TaskDone { id } => set_status(&mut plan, &id, "fait ✓"),
                AgentEvent::TaskFailed { id, .. } => set_status(&mut plan, &id, "échec ✗"),
            }
        }
    });
}

// --- Gestion des agents et skills (toutes les opérations persistent via le cœur) -----------
// La logique « catalogue / branchement » vit dans `orchestra-core::catalog` (partagée avec le
// TUI) ; ici on ne fait qu'appeler le cœur depuis des signaux Dioxus.

/// (Dé)coche un skill pour un agent et **persiste** la config.
pub fn toggle_skill(mut space: Signal<Option<ContextSpace>>, agent_idx: usize, skill_id: &str) {
    let mut guard = space.write();
    let Some(sp) = guard.as_mut() else { return };
    if let Some(agent) = sp.config.agents.get_mut(agent_idx) {
        if let Some(pos) = agent.skills.iter().position(|s| s == skill_id) {
            agent.skills.remove(pos);
        } else {
            agent.skills.push(skill_id.to_string());
        }
    }
    let _ = sp.save_config();
}

/// Renomme l'agent `idx` (ignoré si vide) et persiste.
pub fn set_agent_name(mut space: Signal<Option<ContextSpace>>, idx: usize, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let mut g = space.write();
    if let Some(sp) = g.as_mut() {
        if let Some(a) = sp.config.agents.get_mut(idx) {
            a.name = name.to_string();
        }
        let _ = sp.save_config();
    }
}

/// Change le rôle de l'agent `idx` et persiste.
pub fn set_agent_role(mut space: Signal<Option<ContextSpace>>, idx: usize, role: &str) {
    let mut g = space.write();
    if let Some(sp) = g.as_mut() {
        if let Some(a) = sp.config.agents.get_mut(idx) {
            a.role = role.trim().to_string();
        }
        let _ = sp.save_config();
    }
}

/// Ajoute un agent personnalisé (nom seul) et persiste.
pub fn add_agent(mut space: Signal<Option<ContextSpace>>, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let mut g = space.write();
    if let Some(sp) = g.as_mut() {
        sp.config.agents.push(AgentDef::new(name));
        let _ = sp.save_config();
    }
}

/// Ajoute le prochain agent **suggéré** (catalogue du type de projet) non encore présent.
pub fn add_suggested_agent(mut space: Signal<Option<ContextSpace>>) {
    let mut g = space.write();
    if let Some(sp) = g.as_mut() {
        let mut suggestions = orchestra_core::catalog::inactive_agent_templates(sp);
        if !suggestions.is_empty() {
            sp.config.agents.push(suggestions.remove(0));
            let _ = sp.save_config();
        }
    }
}

/// Supprime l'agent `idx` et persiste.
pub fn delete_agent(mut space: Signal<Option<ContextSpace>>, idx: usize) {
    let mut g = space.write();
    if let Some(sp) = g.as_mut() {
        if idx < sp.config.agents.len() {
            sp.config.agents.remove(idx);
            let _ = sp.save_config();
        }
    }
}

/// Active/désactive l'Agent Documentaliste pour l'espace et persiste.
pub fn toggle_documentalist(mut space: Signal<Option<ContextSpace>>) {
    let mut g = space.write();
    if let Some(sp) = g.as_mut() {
        sp.config.documentalist_enabled = !sp.config.documentalist_enabled;
        let _ = sp.save_config();
    }
}

/// Vrai si le Documentaliste est activé.
pub fn documentalist_enabled(space: Signal<Option<ContextSpace>>) -> bool {
    space.read().as_ref().is_some_and(|sp| sp.config.documentalist_enabled)
}

/// **Branche** un skill non branché : crée sa fiche `SKILL.md` (via le cœur) et renvoie son chemin.
pub fn wire_skill(space: Signal<Option<ContextSpace>>, id: &str) -> Option<PathBuf> {
    let g = space.read();
    let sp = g.as_ref()?;
    orchestra_core::catalog::wire_skill(sp, id).ok()
}

/// Crée une nouvelle fiche de skill `<name>` et renvoie son chemin `SKILL.md`.
pub fn create_fiche(space: Signal<Option<ContextSpace>>, name: &str) -> Option<PathBuf> {
    let g = space.read();
    let sp = g.as_ref()?;
    orchestra_core::markdown_skill::create(&sp.root, name, "").ok()
}

/// Charge le contenu de la fiche `id` (chemin + texte) pour l'éditeur.
pub fn load_fiche(space: Signal<Option<ContextSpace>>, id: &str) -> Option<(PathBuf, String)> {
    let g = space.read();
    let sp = g.as_ref()?;
    let path = orchestra_core::markdown_skill::skills_dir(&sp.root).join(id).join("SKILL.md");
    let text = orchestra_core::model::load_document(&path).ok()?;
    Some((path, text))
}

/// Enregistre le contenu d'une fiche `SKILL.md` (via le cœur).
pub fn save_fiche(path: &Path, content: &str) -> bool {
    orchestra_core::markdown_skill::save(path, content).is_ok()
}

/// Enregistre un document quelconque de l'espace (persona, memory, ADR, `.md`) via le cœur.
pub fn save_document(path: &Path, content: &str) -> bool {
    orchestra_core::model::save_document(path, content).is_ok()
}

// --- Espaces : registre des espaces connus (récents) ---------------------------------------

/// Espaces connus (récents d'abord), depuis le registre global.
pub fn known_spaces() -> Vec<KnownSpace> {
    orchestra_core::registry::known_spaces()
}

/// Ouvre un espace depuis un chemin : charge, met à jour les signaux, **mémorise** dans le
/// registre et rafraîchit la liste des espaces connus. Renvoie `true` si chargé.
pub fn open_space(
    mut space: Signal<Option<ContextSpace>>,
    mut space_path: Signal<String>,
    mut known: Signal<Vec<KnownSpace>>,
    path: &str,
) -> bool {
    let pb = PathBuf::from(path);
    match ContextSpace::load(&pb) {
        Ok(sp) => {
            space.set(Some(sp));
            space_path.set(path.to_string());
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
