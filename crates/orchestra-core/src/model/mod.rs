//! Modèle de données agnostique des « Espaces de Contexte ».

pub mod config;
pub mod project_type;
pub mod space;

pub use config::{
    AgentDef, GitIntegration, GithubIntegration, Integrations, JiraIntegration, ProjectConfig,
};
pub use project_type::ProjectType;
pub use space::{load_document, save_document, Adr, ContextSpace, DocKind, SpaceDoc};
