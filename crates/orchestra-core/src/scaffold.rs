//! Génération d'un Espace de Contexte sur disque (« orchestra init », Phase 2).
//!
//! Logique PURE : aucune interaction terminal ici. La couche UI (`orchestra-tui`)
//! collecte les choix de l'utilisateur, les empaquette dans [`InitOptions`] et
//! appelle [`scaffold_space`]. C'est la même frontière métier/affichage que le reste
//! du cœur — demain l'UI Tauri appellera ces fonctions à l'identique.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::OrchestraError;
use crate::model::config::ProjectConfig;
use crate::model::space::ContextSpace;

/// Les choix collectés auprès de l'utilisateur par l'assistant d'initialisation.
///
/// Découplé de [`ProjectConfig`] : c'est l'« intention » de l'utilisateur, traduite
/// en configuration par [`Self::into_config`]. Aucun agent ni skill n'est pré-câblé —
/// l'Orchestrateur déploie sa propre équipe à la volée.
#[derive(Debug, Clone)]
pub struct InitOptions {
    pub project_name: String,
    /// Chemin du code à piloter. `None` si l'espace n'est pas adossé à un dossier de code.
    pub workspace_path: Option<PathBuf>,
    /// Intégrations écosystème (Git/GitHub/Jira) choisies à l'init.
    pub integrations: crate::model::config::Integrations,
    /// Objectifs / description du projet saisis à la création — injectés dans le persona.
    /// Vide → gabarit seul.
    pub objectives: String,
}

impl InitOptions {
    /// Traduit l'intention en configuration.
    pub fn into_config(self) -> ProjectConfig {
        ProjectConfig {
            project_name: self.project_name,
            workspace_path: self.workspace_path,
            integrations: self.integrations,
        }
    }
}

/// Crée l'arborescence `.orchestra/` dans `root` à partir des choix fournis, puis
/// renvoie l'[`ContextSpace`] fraîchement chargé.
///
/// Produit :
/// - `.orchestra/config.json` (formaté, lisible),
/// - `.orchestra/persona.md` (gabarit propre au type de projet),
/// - `.orchestra/adr/` (dossier vide, prêt à recevoir les décisions d'architecture).
///
/// Refuse d'écraser un espace déjà initialisé ([`OrchestraError::SpaceAlreadyExists`]).
pub fn scaffold_space(root: &Path, opts: InitOptions) -> Result<ContextSpace, OrchestraError> {
    let orchestra_dir = root.join(".orchestra");
    let config_path = orchestra_dir.join("config.json");

    if config_path.exists() {
        return Err(OrchestraError::SpaceAlreadyExists { path: config_path });
    }

    let objectives = opts.objectives.clone();
    let config = opts.into_config();

    fs::create_dir_all(orchestra_dir.join("adr"))?;

    let json = serde_json::to_string_pretty(&config)?;
    fs::write(&config_path, format!("{json}\n"))?;

    fs::write(
        orchestra_dir.join("persona.md"),
        persona_template(&config.project_name, &objectives),
    )?;

    ContextSpace::load(root)
}

/// **Reprend un projet existant** : initialise `.orchestra/` dans `root` (workspace = `root`).
/// Le dossier de code devient un Espace pilotable.
/// Refuse d'écraser une configuration déjà présente ([`OrchestraError::SpaceAlreadyExists`]).
pub fn adopt_project(root: &Path) -> Result<ContextSpace, OrchestraError> {
    let name = root
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("projet")
        .to_string();
    let opts = InitOptions {
        project_name: name,
        workspace_path: Some(root.to_path_buf()),
        integrations: Default::default(),
        objectives: String::new(),
    };
    scaffold_space(root, opts)
}

/// Gabarit de persona — point de départ que l'utilisateur complète.
fn persona_template(name: &str, objectives: &str) -> String {
    // Objectifs saisis à la création (le cas échéant), placés en tête.
    let objectives_section = if objectives.trim().is_empty() {
        String::new()
    } else {
        format!("## Objectifs du projet\n{}\n\n", objectives.trim())
    };
    let body = "## Contexte technique\n\
         - **Langages / stack** : à compléter\n\
         - **Conventions de code** : à compléter\n\
         - **Commande de tests** : à compléter\n\n\
         ## Objectifs\n\
         - à compléter\n";

    format!("# Persona — {name}\n\n{objectives_section}{body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Répertoire temporaire unique, nettoyé en fin de test (sans dépendance externe).
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let mut p = std::env::temp_dir();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            p.push(format!("orchestra-test-{tag}-{nanos}"));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn scaffolds_and_reloads_space() {
        let tmp = TempDir::new("dev");
        let opts = InitOptions {
            project_name: "Mon_App".to_string(),
            workspace_path: Some(PathBuf::from("/code/mon-app")),
            integrations: Default::default(),
            objectives: "Construire l'API de paiement.".to_string(),
        };

        let space = scaffold_space(&tmp.0, opts).expect("scaffolding réussi");
        // Les objectifs saisis se retrouvent dans le persona.
        assert!(space.persona.as_deref().unwrap().contains("API de paiement"));

        // La config est rechargeable.
        assert_eq!(space.config.project_name, "Mon_App");
        assert_eq!(space.config.workspace_path.as_deref(), Some(Path::new("/code/mon-app")));
        assert!(space.persona.is_some());
        assert!(tmp.0.join(".orchestra").join("adr").is_dir());
    }

    #[test]
    fn save_persona_persists_and_reloads() {
        let tmp = TempDir::new("persona");
        let opts = InitOptions {
            project_name: "P".to_string(),
            workspace_path: None,
            integrations: Default::default(),
            objectives: String::new(),
        };
        let mut space = scaffold_space(&tmp.0, opts).expect("scaffolding réussi");

        space.save_persona("# Persona\n\nBudget : 350k€").expect("sauvegarde OK");
        assert_eq!(space.persona.as_deref(), Some("# Persona\n\nBudget : 350k€"));

        // Rechargé depuis le disque, le persona reflète la sauvegarde.
        let reloaded = ContextSpace::load(&tmp.0).expect("rechargement OK");
        assert_eq!(reloaded.persona.as_deref(), Some("# Persona\n\nBudget : 350k€"));
    }

    #[test]
    fn adopt_project_initializes_space_in_place() {
        let tmp = TempDir::new("adopt");
        let space = adopt_project(&tmp.0).expect("reprise réussie");
        // Le workspace pointe sur le dossier repris lui-même.
        assert_eq!(space.config.workspace_path.as_deref(), Some(tmp.0.as_path()));
        assert!(tmp.0.join(".orchestra").join("config.json").is_file());
    }

    #[test]
    fn refuses_to_overwrite_existing_space() {
        let tmp = TempDir::new("dup");
        let opts = || InitOptions {
            project_name: "X".to_string(),
            workspace_path: None,
            integrations: Default::default(),
            objectives: String::new(),
        };

        scaffold_space(&tmp.0, opts()).expect("1er init OK");
        let err = scaffold_space(&tmp.0, opts()).expect_err("2e init doit échouer");
        assert!(matches!(err, OrchestraError::SpaceAlreadyExists { .. }));
    }
}
