//! Contrat d'événements cœur ↔ UI.
//!
//! Émis dès la Phase 3 par le [runtime](crate::runtime) sur un canal `tokio::sync::mpsc`,
//! puis consommés par le TUI (et demain l'UI Tauri) sans que l'affichage ne connaisse le
//! cœur. C'est la frontière métier/affichage de la spec, matérialisée par ce type.

/// Aperçu d'une tâche planifiée, pour l'affichage du plan côté UI (sans dépendre du modèle
/// d'orchestration interne).
#[derive(Debug, Clone)]
pub struct PlannedTask {
    pub id: String,
    pub agent: String,
    pub objective: String,
    pub depends_on: Vec<String>,
}

/// Une unité d'activité observable sur l'« écran radar ».
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// L'agent vient de démarrer sa tâche.
    Started { agent: String },
    /// L'agent attend une réponse du modèle (appel LLM en cours) — pilote le spinner.
    Thinking { agent: String },
    /// Ligne de log d'un agent (ex. « 12 nouvelles annonces trouvées »).
    Log { agent: String, msg: String },
    /// L'agent a terminé sa tâche.
    Done { agent: String },
    /// Un plan d'orchestration a été établi et attend (selon le mode) validation/exécution.
    PlanReady { tasks: Vec<PlannedTask> },
    /// Une tâche du plan démarre (exécutée par `agent`).
    TaskStarted { id: String, agent: String },
    /// Une tâche du plan s'est terminée avec succès.
    TaskDone { id: String },
    /// Une tâche du plan a échoué.
    TaskFailed { id: String, error: String },
}

impl AgentEvent {
    /// Nom de l'agent à l'origine de l'événement, s'il y en a un.
    pub fn agent(&self) -> Option<&str> {
        match self {
            AgentEvent::Started { agent }
            | AgentEvent::Thinking { agent }
            | AgentEvent::Log { agent, .. }
            | AgentEvent::Done { agent }
            | AgentEvent::TaskStarted { agent, .. } => Some(agent),
            AgentEvent::PlanReady { .. }
            | AgentEvent::TaskDone { .. }
            | AgentEvent::TaskFailed { .. } => None,
        }
    }
}
