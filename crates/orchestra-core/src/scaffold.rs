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
use crate::model::project_type::ProjectType;
use crate::model::skill_id::{default_agents, default_skills};
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
}

impl InitOptions {
    /// Traduit l'intention en configuration complète, en pré-remplissant la matrice
    /// de Skills et d'agents par défaut associée au type de projet.
    pub fn into_config(self) -> ProjectConfig {
        ProjectConfig {
            project_name: self.project_name,
            project_type: self.project_type,
            workspace_path: self.workspace_path,
            documentalist_enabled: self.documentalist_enabled,
            skills: default_skills(self.project_type),
            agents: default_agents(self.project_type),
            integrations: Default::default(),
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
    let config = opts.into_config();

    fs::create_dir_all(orchestra_dir.join("adr"))?;

    let json = serde_json::to_string_pretty(&config)?;
    fs::write(&config_path, format!("{json}\n"))?;

    fs::write(
        orchestra_dir.join("persona.md"),
        persona_template(project_type, &config.project_name),
    )?;

    ContextSpace::load(root)
}

/// Gabarit de persona propre au type de projet — point de départ que l'utilisateur
/// complète. Chaque famille de projet a des « critères » naturellement différents.
fn persona_template(kind: ProjectType, name: &str) -> String {
    let body = match kind {
        ProjectType::Dev => {
            "## Contexte technique\n\
             - **Langages / stack** : à compléter\n\
             - **Conventions de code** : à compléter\n\
             - **Commande de tests** : à compléter\n\n\
             ## Objectifs\n\
             - à compléter\n"
        }
        ProjectType::Nutrition => {
            "## Objectifs\n\
             - **But** (perte/maintien/prise) : à compléter\n\
             - **Calories cibles / jour** : à compléter\n\n\
             ## Contraintes\n\
             - **Allergies / intolérances** : à compléter\n\
             - **Régime** (végé, sans gluten…) : à compléter\n"
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
        ProjectType::Immobilier => {
            "## Critères stricts\n\
             - **Budget max** : à compléter\n\
             - **Surface min (m²)** : à compléter\n\
             - **Quartiers cibles** : à compléter\n\
             - **Diagnostics minimum (DPE)** : à compléter\n\n\
             ## Sources\n\
             - à compléter\n"
        }
    };

    format!("# Persona — {name}\n\n{body}")
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
        };

        let space = scaffold_space(&tmp.0, opts).expect("scaffolding réussi");

        // La config est rechargeable et reflète les choix + les défauts injectés.
        assert_eq!(space.config.project_name, "Mon_App");
        assert_eq!(space.config.project_type, ProjectType::Dev);
        assert!(space.config.documentalist_enabled);
        assert_eq!(space.config.skills, default_skills(ProjectType::Dev));
        assert_eq!(space.config.agents, default_agents(ProjectType::Dev));
        assert!(space.persona.is_some());
        assert!(tmp.0.join(".orchestra").join("adr").is_dir());
    }

    #[test]
    fn refuses_to_overwrite_existing_space() {
        let tmp = TempDir::new("dup");
        let opts = || InitOptions {
            project_name: "X".to_string(),
            project_type: ProjectType::Nutrition,
            workspace_path: None,
            documentalist_enabled: false,
        };

        scaffold_space(&tmp.0, opts()).expect("1er init OK");
        let err = scaffold_space(&tmp.0, opts()).expect_err("2e init doit échouer");
        assert!(matches!(err, OrchestraError::SpaceAlreadyExists { .. }));
    }
}
