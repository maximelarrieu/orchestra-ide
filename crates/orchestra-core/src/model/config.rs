use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::project_type::ProjectType;

/// Contenu de `.orchestra/config.json` : la définition complète d'un Espace de Contexte.
///
/// Volontairement agnostique — un projet Dev, Nutrition, Langue ou Immobilier partage
/// la même structure ; seuls les Skills/Agents et les intégrations diffèrent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub project_name: String,
    pub project_type: ProjectType,

    /// Chemin local ciblé (projets « Dev »). `None` pour les projets hors-dev.
    #[serde(default)]
    pub workspace_path: Option<PathBuf>,

    /// Active l'Agent Documentaliste (Phase 5).
    #[serde(default)]
    pub documentalist_enabled: bool,

    /// Identifiants des Skills activés (exécutables en Phase 3).
    #[serde(default)]
    pub skills: Vec<String>,

    /// Agents composant l'orchestre (nom, rôle, skills propres).
    #[serde(default)]
    pub agents: Vec<AgentDef>,

    /// Intégrations écosystème (Phase 4) — toutes optionnelles.
    #[serde(default)]
    pub integrations: Integrations,
}

/// Définition d'un agent : nom, rôle (qui oriente son prompt) et skills propres.
///
/// Rétro-compatible : un agent écrit comme une simple chaîne dans `config.json`
/// (`"agents": ["Agent_Tuteur"]`) est chargé comme `AgentDef { name, role: "", skills: [] }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentDef {
    pub name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub skills: Vec<String>,
}

impl AgentDef {
    /// Crée un agent depuis un nom (rôle/skills vides).
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), role: String::new(), skills: Vec::new() }
    }
}

impl<'de> Deserialize<'de> for AgentDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Name(String),
            Full {
                name: String,
                #[serde(default)]
                role: String,
                #[serde(default)]
                skills: Vec<String>,
            },
        }
        Ok(match Repr::deserialize(deserializer)? {
            Repr::Name(name) => AgentDef { name, role: String::new(), skills: Vec::new() },
            Repr::Full { name, role, skills } => AgentDef { name, role, skills },
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Integrations {
    #[serde(default)]
    pub git: Option<GitIntegration>,
    #[serde(default)]
    pub github: Option<GithubIntegration>,
    #[serde(default)]
    pub jira: Option<JiraIntegration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitIntegration {
    #[serde(default)]
    pub auto_branching: bool,
    pub main_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubIntegration {
    pub repo: String,
    /// Nom de la variable d'environnement contenant le token (jamais le token en clair).
    pub token_env_var: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JiraIntegration {
    pub project_key: String,
    pub url: String,
    pub token_env_var: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_immobilier_space() {
        let raw = r#"{
            "project_name": "Recherche_Immo_Aix",
            "project_type": "immobilier",
            "agents": ["Agent_Scraper", "Agent_Filtrage"],
            "skills": ["Scrape_Web_Page", "Extract_JSON_From_HTML", "Geocoding_Calcul"]
        }"#;

        let cfg: ProjectConfig = serde_json::from_str(raw).expect("config valide");
        assert_eq!(cfg.project_type, ProjectType::Immobilier);
        assert_eq!(cfg.skills.len(), 3);
        // Rétro-compat : agents écrits en chaînes → AgentDef (rôle/skills vides).
        assert_eq!(cfg.agents.len(), 2);
        assert_eq!(cfg.agents[0].name, "Agent_Scraper");
        assert!(cfg.agents[0].role.is_empty());
        // Champs absents → valeurs par défaut (serde(default)).
        assert!(cfg.workspace_path.is_none());
        assert!(!cfg.documentalist_enabled);
        assert!(cfg.integrations.git.is_none());
    }

    #[test]
    fn project_type_round_trips_snake_case() {
        let json = serde_json::to_string(&ProjectType::Dev).unwrap();
        assert_eq!(json, "\"dev\"");
    }
}
