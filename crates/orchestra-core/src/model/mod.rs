//! Modèle de données agnostique des « Espaces de Contexte ».

pub mod config;
pub mod project_type;
pub mod skill_id;
pub mod space;

pub use config::{GitIntegration, GithubIntegration, Integrations, JiraIntegration, ProjectConfig};
pub use project_type::ProjectType;
pub use skill_id::{default_agents, default_skills};
pub use space::{Adr, ContextSpace};
