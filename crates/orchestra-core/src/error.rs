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

    #[error("Un Espace existe déjà ici : {path} — refus d'écraser la configuration")]
    SpaceAlreadyExists { path: PathBuf },

    #[error("Configuration invalide : {0}")]
    InvalidConfig(#[from] serde_json::Error),

    #[error("Un skill « {0} » existe déjà — refus de l'écraser")]
    SkillAlreadyExists(String),

    #[error("Nom de skill invalide : « {0} » (lettres, chiffres, « _ » ou « - »)")]
    InvalidSkillName(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
