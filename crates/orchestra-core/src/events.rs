//! Contrat d'événements cœur ↔ UI.
//!
//! Émis dès la Phase 3 par le [runtime](crate::runtime) sur un canal `tokio::sync::mpsc`,
//! puis consommés par le TUI (et demain l'UI Tauri) sans que l'affichage ne connaisse le
//! cœur. C'est la frontière métier/affichage de la spec, matérialisée par ce type.

/// Une unité d'activité observable sur l'« écran radar ».
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// L'agent vient de démarrer sa tâche.
    Started { agent: String },
    /// Ligne de log d'un agent (ex. « 12 nouvelles annonces trouvées »).
    Log { agent: String, msg: String },
    /// L'agent a terminé sa tâche.
    Done { agent: String },
}

impl AgentEvent {
    /// Nom de l'agent à l'origine de l'événement (commun aux trois variantes).
    pub fn agent(&self) -> &str {
        match self {
            AgentEvent::Started { agent }
            | AgentEvent::Log { agent, .. }
            | AgentEvent::Done { agent } => agent,
        }
    }
}
