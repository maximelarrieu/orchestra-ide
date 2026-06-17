//! État de l'app + **ponts vers le cœur** : isole la logique « parler à `orchestra-core` »
//! du rendu. Les composants ([`crate::components`]) se contentent d'afficher / déclencher.

use dioxus::prelude::*;
use orchestra_core::events::AgentEvent;
use orchestra_core::model::ContextSpace;
use orchestra_core::runtime;
use tokio::sync::mpsc::UnboundedSender;

/// Objectif par défaut proposé dans la zone de saisie.
pub const DEFAULT_GOAL: &str = "Avance concrètement sur l'objectif de cet espace.";

/// Vue centrale courante (équivalent des touches du TUI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Orchestrate,
    Documents,
    Agents,
}

/// Une ligne du panneau Plan.
#[derive(Clone, PartialEq)]
pub struct PlanRow {
    pub id: String,
    pub agent: String,
    pub status: String,
}

/// Nature d'un skill dans le sélecteur.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SkillKind {
    Primitive,
    Fiche,
    Label,
}

/// Une entrée du sélecteur de skills (catalogue à cocher).
#[derive(Clone, PartialEq)]
pub struct SkillEntry {
    pub id: String,
    pub kind: SkillKind,
    pub description: String,
    pub selected: bool,
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

/// Catalogue des skills pour un agent : primitives (code) + fiches (`SKILL.md`) + skills
/// assignés mais non branchés (`Label`), chacun marqué assigné ou non.
pub fn skill_entries(space: &ContextSpace, agent_idx: usize) -> Vec<SkillEntry> {
    let assigned = space
        .config
        .agents
        .get(agent_idx)
        .map(|a| a.skills.clone())
        .unwrap_or_default();

    let mut out: Vec<SkillEntry> = Vec::new();
    for m in orchestra_core::skills::catalog() {
        let selected = assigned.iter().any(|s| s == m.id);
        out.push(SkillEntry { id: m.id.to_string(), kind: SkillKind::Primitive, description: m.description, selected });
    }
    for f in orchestra_core::markdown_skill::load_all(&space.root) {
        if out.iter().any(|e| e.id == f.id) {
            continue;
        }
        let selected = assigned.iter().any(|s| s == &f.id || s == &f.name);
        out.push(SkillEntry { id: f.id, kind: SkillKind::Fiche, description: f.description, selected });
    }
    for s in &assigned {
        if !out.iter().any(|e| &e.id == s) {
            out.push(SkillEntry { id: s.clone(), kind: SkillKind::Label, description: "(non branché)".into(), selected: true });
        }
    }
    out
}

/// (Dé)coche un skill pour un agent et **persiste** la config via le cœur.
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
    let _ = sp.save_config(); // écriture disque centralisée dans le cœur
}
