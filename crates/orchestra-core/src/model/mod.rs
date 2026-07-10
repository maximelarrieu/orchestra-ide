//! Modèle de données agnostique des « Espaces de Contexte ».

pub mod config;
pub mod space;

pub use config::{
    GitIntegration, GithubIntegration, Integrations, JiraIntegration, ProjectConfig,
};
pub use space::{load_document, save_document, Adr, ContextSpace, DocKind, SpaceDoc};
