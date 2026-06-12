use std::fs;
use std::path::{Path, PathBuf};

use crate::error::OrchestraError;

use super::config::ProjectConfig;

/// Un Architecture Decision Record référencé dans l'espace.
#[derive(Debug, Clone)]
pub struct Adr {
    pub title: String,
    pub path: PathBuf,
}

/// Un Espace de Contexte chargé en mémoire : configuration + persona + ADRs.
///
/// Point d'entrée du cœur. L'UI ne manipule que cette structure (et les événements),
/// jamais le système de fichiers directement.
#[derive(Debug, Clone)]
pub struct ContextSpace {
    pub root: PathBuf,
    pub config: ProjectConfig,
    pub persona: Option<String>,
    pub adrs: Vec<Adr>,
}

impl ContextSpace {
    /// Chemin du fichier persona de cet espace (`.orchestra/persona.md`).
    pub fn persona_path(&self) -> PathBuf {
        self.root.join(".orchestra").join("persona.md")
    }

    /// Écrit le persona sur disque et met à jour la copie en mémoire. C'est par ici que
    /// l'UI persiste les modifications — elle ne touche jamais le système de fichiers
    /// directement (frontière métier/affichage).
    pub fn save_persona(&mut self, content: &str) -> Result<(), OrchestraError> {
        fs::write(self.persona_path(), content)?;
        self.persona = Some(content.to_string());
        Ok(())
    }

    /// Charge l'espace situé dans `root` (qui doit contenir `.orchestra/config.json`).
    pub fn load(root: &Path) -> Result<Self, OrchestraError> {
        let config_path = root.join(".orchestra").join("config.json");
        let raw = fs::read_to_string(&config_path).map_err(|source| {
            OrchestraError::SpaceNotFound {
                path: config_path.clone(),
                source,
            }
        })?;
        let config: ProjectConfig = serde_json::from_str(&raw)?;

        let persona = fs::read_to_string(root.join(".orchestra").join("persona.md")).ok();
        let adrs = Self::load_adrs(&root.join(".orchestra").join("adr"));

        Ok(Self {
            root: root.to_path_buf(),
            config,
            persona,
            adrs,
        })
    }

    fn load_adrs(dir: &Path) -> Vec<Adr> {
        let mut adrs = Vec::new();
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    let title = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("ADR")
                        .to_string();
                    adrs.push(Adr { title, path });
                }
            }
        }
        adrs.sort_by(|a, b| a.title.cmp(&b.title));
        adrs
    }
}
