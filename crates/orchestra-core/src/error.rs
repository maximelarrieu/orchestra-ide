use std::path::PathBuf;

use thiserror::Error;

/// Erreur unique du cœur d'Orchestra. Les couches supérieures (CLI, futurs agents)
/// remontent toutes leurs erreurs via ce type.
#[derive(Debug, Error)]
pub enum OrchestraError {
    #[error("Espace introuvable : impossible de lire {path} ({source})")]
    SpaceNotFound {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Configuration invalide : {0}")]
    InvalidConfig(#[from] serde_json::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
