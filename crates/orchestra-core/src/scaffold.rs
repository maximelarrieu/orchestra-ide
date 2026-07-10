//! Génération d'un Espace de Contexte sur disque (« orchestra init », Phase 2).
//!
//! Logique PURE : aucune interaction terminal ici. La couche UI (`orchestra-tui`)
//! collecte les choix de l'utilisateur, les empaquette dans [`InitOptions`] et
//! appelle [`scaffold_space`]. C'est la même frontière métier/affichage que le reste
//! du cœur — demain l'UI Tauri appellera ces fonctions à l'identique.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::OrchestraError;
use crate::model::config::{AgentDef, ProjectConfig};
use crate::model::project_type::ProjectType;
use crate::model::space::ContextSpace;

/// Les choix collectés auprès de l'utilisateur par l'assistant d'initialisation.
///
/// Découplé de [`ProjectConfig`] : c'est l'« intention » de l'utilisateur, traduite
/// en configuration complète (Skills/agents par défaut compris) par [`Self::into_config`].
#[derive(Debug, Clone)]
pub struct InitOptions {
    pub project_name: String,
    pub project_type: ProjectType,
    /// Chemin du code ciblé — pertinent pour les projets « Dev » uniquement.
    pub workspace_path: Option<PathBuf>,
    pub documentalist_enabled: bool,
    /// Intégrations écosystème (Git/GitHub/Jira) choisies à l'init.
    pub integrations: crate::model::config::Integrations,
    /// Objectifs / description du projet saisis à la création — injectés dans le persona.
    /// Vide → gabarit seul.
    pub objectives: String,
    /// Squad d'agents choisie. Vide → squad de départ ([`default_agents`]).
    pub agents: Vec<AgentDef>,
}

impl InitOptions {
    /// Traduit l'intention en configuration. **Aucun agent ni skill pré-défini** n'est injecté :
    /// une session démarre vierge, et l'Orchestrateur déploie sa propre équipe à la volée
    /// (`spawn_agent`). Les champs `agents`/`skills` restent vides (héritage schéma).
    pub fn into_config(self) -> ProjectConfig {
        ProjectConfig {
            project_name: self.project_name,
            project_type: self.project_type,
            workspace_path: self.workspace_path,
            documentalist_enabled: self.documentalist_enabled,
            skills: Vec::new(),
            agents: self.agents, // vide par défaut (plus de roster pré-câblé)
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

    let project_type = opts.project_type;
    let objectives = opts.objectives.clone();
    let config = opts.into_config();

    fs::create_dir_all(orchestra_dir.join("adr"))?;

    let json = serde_json::to_string_pretty(&config)?;
    fs::write(&config_path, format!("{json}\n"))?;

    fs::write(
        orchestra_dir.join("persona.md"),
        persona_template(project_type, &config.project_name, &objectives),
    )?;

    ContextSpace::load(root)
}

/// **Reprend un projet existant** : initialise `.orchestra/` dans `root` (workspace = `root`),
/// en type Dev avec Documentaliste activé. Le dossier de code devient un Espace pilotable.
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
        project_type: ProjectType::Dev,
        workspace_path: Some(root.to_path_buf()),
        documentalist_enabled: true,
        integrations: Default::default(),
        objectives: String::new(),
        agents: Vec::new(),
    };
    scaffold_space(root, opts)
}

/// Gabarit de persona propre au type de projet — point de départ que l'utilisateur
/// complète. Chaque famille de projet a des « critères » naturellement différents.
fn persona_template(kind: ProjectType, name: &str, objectives: &str) -> String {
    // Objectifs saisis à la création (le cas échéant), placés en tête.
    let objectives_section = if objectives.trim().is_empty() {
        String::new()
    } else {
        format!("## Objectifs du projet\n{}\n\n", objectives.trim())
    };
    let body = match kind {
        ProjectType::Dev => {
            "## Contexte technique\n\
             - **Langages / stack** : à compléter\n\
             - **Conventions de code** : à compléter\n\
             - **Commande de tests** : à compléter\n\n\
             ## Objectifs\n\
             - à compléter\n"
        }
        ProjectType::Langue => {
            "## Apprentissage\n\
             - **Langue cible** : à compléter\n\
             - **Niveau actuel** (CECRL) : à compléter\n\
             - **Objectif** : à compléter\n\n\
             ## Préférences\n\
             - **Rythme** : à compléter\n\
             - **Thèmes** : à compléter\n"
        }
    };

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
    fn scaffolds_and_reloads_dev_space() {
        let tmp = TempDir::new("dev");
        let opts = InitOptions {
            project_name: "Mon_App".to_string(),
            project_type: ProjectType::Dev,
            workspace_path: Some(PathBuf::from("/code/mon-app")),
            documentalist_enabled: true,
            integrations: Default::default(),
            objectives: "Construire l'API de paiement.".to_string(),
            agents: Vec::new(),
        };

        let space = scaffold_space(&tmp.0, opts).expect("scaffolding réussi");
        // Les objectifs saisis se retrouvent dans le persona.
        assert!(space.persona.as_deref().unwrap().contains("API de paiement"));

        // La config est rechargeable ; une session démarre SANS agents/skills pré-câblés.
        assert_eq!(space.config.project_name, "Mon_App");
        assert_eq!(space.config.project_type, ProjectType::Dev);
        assert!(space.config.documentalist_enabled);
        assert!(space.config.skills.is_empty(), "aucun skill pré-défini");
        assert!(space.config.agents.is_empty(), "aucun agent pré-défini");
        assert!(space.persona.is_some());
        assert!(tmp.0.join(".orchestra").join("adr").is_dir());
    }

    #[test]
    fn save_persona_persists_and_reloads() {
        let tmp = TempDir::new("persona");
        let opts = InitOptions {
            project_name: "P".to_string(),
            project_type: ProjectType::Dev,
            workspace_path: None,
            documentalist_enabled: false,
            integrations: Default::default(),
            objectives: String::new(),
            agents: Vec::new(),
        };
        let mut space = scaffold_space(&tmp.0, opts).expect("scaffolding réussi");

        space.save_persona("# Persona\n\nBudget : 350k€").expect("sauvegarde OK");
        assert_eq!(space.persona.as_deref(), Some("# Persona\n\nBudget : 350k€"));

        // Rechargé depuis le disque, le persona reflète la sauvegarde.
        let reloaded = ContextSpace::load(&tmp.0).expect("rechargement OK");
        assert_eq!(reloaded.persona.as_deref(), Some("# Persona\n\nBudget : 350k€"));
    }

    #[test]
    fn adopt_project_initializes_dev_space_in_place() {
        let tmp = TempDir::new("adopt");
        let space = adopt_project(&tmp.0).expect("reprise réussie");
        assert_eq!(space.config.project_type, ProjectType::Dev);
        // Le workspace pointe sur le dossier repris lui-même.
        assert_eq!(space.config.workspace_path.as_deref(), Some(tmp.0.as_path()));
        assert!(space.config.documentalist_enabled);
        assert!(tmp.0.join(".orchestra").join("config.json").is_file());
    }

    #[test]
    fn refuses_to_overwrite_existing_space() {
        let tmp = TempDir::new("dup");
        let opts = || InitOptions {
            project_name: "X".to_string(),
            project_type: ProjectType::Langue,
            workspace_path: None,
            documentalist_enabled: false,
            integrations: Default::default(),
            objectives: String::new(),
            agents: Vec::new(),
        };

        scaffold_space(&tmp.0, opts()).expect("1er init OK");
        let err = scaffold_space(&tmp.0, opts()).expect_err("2e init doit échouer");
        assert!(matches!(err, OrchestraError::SpaceAlreadyExists { .. }));
    }
}
