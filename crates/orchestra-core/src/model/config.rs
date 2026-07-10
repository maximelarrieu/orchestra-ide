use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Contenu de `.orchestra/config.json` : la définition d'un Espace de Contexte.
///
/// Volontairement minimal et agnostique du domaine : l'Orchestrateur déploie lui-même
/// son équipe et choisit ses outils à la volée — rien n'est pré-câblé côté config.
/// Les anciens champs (`project_type`, `agents`, `skills`, `documentalist_enabled`) présents
/// dans d'anciens `config.json` sont simplement ignorés au chargement (serde tolère les
/// champs inconnus).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub project_name: String,

    /// Chemin local du code à piloter. `None` si l'espace n'est pas adossé à un dossier de code.
    #[serde(default)]
    pub workspace_path: Option<PathBuf>,

    /// Intégrations écosystème (Git/GitHub/Jira) — toutes optionnelles.
    #[serde(default)]
    pub integrations: Integrations,
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
    fn parse_minimal_space() {
        let raw = r#"{ "project_name": "Mon_App" }"#;

        let cfg: ProjectConfig = serde_json::from_str(raw).expect("config valide");
        assert_eq!(cfg.project_name, "Mon_App");
        // Champs absents → valeurs par défaut (serde(default)).
        assert!(cfg.workspace_path.is_none());
        assert!(cfg.integrations.git.is_none());
    }

    #[test]
    fn ignores_legacy_fields() {
        // D'anciens config.json portent encore project_type/agents/skills/documentalist :
        // ils doivent être ignorés silencieusement (pas d'erreur de désérialisation).
        let raw = r#"{
            "project_name": "Legacy",
            "project_type": "dev",
            "documentalist_enabled": true,
            "agents": ["Agent_Architecte", "Agent_Codeur"],
            "skills": ["Read_File", "Write_File_Validated"]
        }"#;

        let cfg: ProjectConfig = serde_json::from_str(raw).expect("config valide");
        assert_eq!(cfg.project_name, "Legacy");
        assert!(cfg.workspace_path.is_none());
    }
}
