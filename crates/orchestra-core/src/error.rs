use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("configuration invalide : {0}")]
    Config(String),
    #[error("transition de statut interdite : {from} → {to}")]
    IllegalTransition { from: String, to: String },
    #[error("rôle inconnu : {0}")]
    UnknownRole(String),
    #[error("dépendances cycliques entre rôles : {0}")]
    CyclicDependencies(String),
    #[error("parse : {0}")]
    Parse(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
    #[error(transparent)]
    Yaml(#[from] serde_yaml_ng::Error),
}

pub type Result<T> = std::result::Result<T, CoreError>;
