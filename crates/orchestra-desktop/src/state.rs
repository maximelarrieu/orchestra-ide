//! État de l'app + **pont vers le cœur** : isole la logique « parler à `orchestra-core` »
//! du rendu (les composants se contentent d'afficher les signaux mis à jour ici).

use dioxus::prelude::*;
use orchestra_core::events::AgentEvent;
use orchestra_core::model::ContextSpace;
use orchestra_core::runtime;
use tokio::sync::mpsc::UnboundedSender;

/// Objectif par défaut envoyé à l'orchestre (saisie libre à venir).
pub const DEFAULT_GOAL: &str = "Avance concrètement sur l'objectif de cet espace.";

/// Une ligne du panneau Plan.
#[derive(Clone, PartialEq)]
pub struct PlanRow {
    pub id: String,
    pub agent: String,
    pub status: String,
}

/// Lance l'orchestration et **streame les [`AgentEvent`]** dans les signaux fournis (radar,
/// plan, drapeau d'approbation, canal d'approbation). Appelé depuis un gestionnaire d'événement
/// du composant (le `spawn` Dioxus s'exécute donc dans le scope réactif courant).
pub fn drive_orchestration(
    space: ContextSpace,
    mut log: Signal<Vec<String>>,
    mut plan: Signal<Vec<PlanRow>>,
    mut pending: Signal<bool>,
    mut approve_tx: Signal<Option<UnboundedSender<bool>>>,
) {
    log.set(Vec::new());
    plan.set(Vec::new());
    pending.set(false);

    spawn(async move {
        let handle = runtime::orchestrate(&space, DEFAULT_GOAL);
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

/// Met à jour le statut d'une tâche du plan (par id).
fn set_status(plan: &mut Signal<Vec<PlanRow>>, id: &str, status: &str) {
    let mut rows = plan.write();
    if let Some(row) = rows.iter_mut().find(|r| r.id == id) {
        row.status = status.to_string();
    }
}
