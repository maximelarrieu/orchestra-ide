//! Cœur métier d'Orchestra IDE.
//!
//! Ce crate ne dépend d'AUCUNE bibliothèque d'affichage : il expose le modèle des
//! « Espaces de Contexte », les erreurs et le contrat d'événements ([`events::AgentEvent`])
//! que l'UI (ratatui aujourd'hui, Tauri demain) consomme. C'est la garantie du
//! découplage strict logique métier / affichage exigé par la spec.

pub mod error;
pub mod events;
pub mod integrations;
pub mod llm;
pub mod markdown_skill;
pub mod model;
pub mod runtime;
pub mod scaffold;
pub mod skills;

pub use error::OrchestraError;
pub use scaffold::{scaffold_space, InitOptions};
