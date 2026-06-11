//! Contrat d'événements cœur ↔ UI.
//!
//! En Phase 1 ce type est défini mais pas encore émis : les agents qui le publieront
//! sur un canal `tokio::sync::mpsc` arrivent en Phase 3. Le figer dès maintenant fixe
//! l'interface que le TUI — puis l'UI Tauri — consommera sans connaître le cœur.

/// Une unité d'activité observable sur l'« écran radar ».
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Ligne de log d'un agent (ex. « 12 nouvelles annonces trouvées »).
    Log { agent: String, msg: String },
    /// L'agent a terminé sa tâche.
    Done { agent: String },
}
