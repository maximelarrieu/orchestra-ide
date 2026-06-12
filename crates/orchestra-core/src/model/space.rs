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

/// Nature d'un document consultable de l'espace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    Persona,
    Adr,
    Doc,
}

/// Un document consultable de l'espace (persona, ADR, ou Markdown du workspace).
#[derive(Debug, Clone)]
pub struct SpaceDoc {
    pub label: String,
    pub path: PathBuf,
    pub kind: DocKind,
}

/// Lit le contenu texte d'un document. Centralise l'accès disque dans le cœur (l'UI ne
/// touche jamais le système de fichiers directement).
pub fn load_document(path: &Path) -> Result<String, OrchestraError> {
    Ok(fs::read_to_string(path)?)
}

const SCAN_MAX_DEPTH: usize = 4;
const SCAN_MAX_FILES: usize = 200;
/// Dossiers ignorés lors du balayage des Markdown du workspace.
const SCAN_SKIP: &[&str] = &["target", "node_modules", "dist", "build", "vendor"];

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

    /// Racine de travail des agents : `workspace_path` si défini, sinon la racine de l'espace.
    pub fn workspace(&self) -> PathBuf {
        self.config
            .workspace_path
            .clone()
            .unwrap_or_else(|| self.root.clone())
    }

    /// Documents consultables de l'espace : persona, ADRs, puis les fichiers Markdown
    /// trouvés dans le workspace (ex. ceux produits par l'Agent Documentaliste).
    pub fn documents(&self) -> Vec<SpaceDoc> {
        let mut docs = Vec::new();

        let persona = self.persona_path();
        if persona.is_file() {
            docs.push(SpaceDoc { label: "persona.md".into(), path: persona, kind: DocKind::Persona });
        }
        for adr in &self.adrs {
            let name = adr
                .path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("adr.md");
            docs.push(SpaceDoc { label: format!("adr/{name}"), path: adr.path.clone(), kind: DocKind::Adr });
        }

        let ws = self.workspace();
        let mut found = Vec::new();
        scan_markdown(&ws, 0, &mut found);
        found.sort();
        for path in found {
            let label = path
                .strip_prefix(&ws)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            docs.push(SpaceDoc { label, path, kind: DocKind::Doc });
        }

        docs
    }

    /// Écrit le persona sur disque et met à jour la copie en mémoire. C'est par ici que
    /// l'UI persiste les modifications — elle ne touche jamais le système de fichiers
    /// directement (frontière métier/affichage).
    pub fn save_persona(&mut self, content: &str) -> Result<(), OrchestraError> {
        fs::write(self.persona_path(), content)?;
        self.persona = Some(content.to_string());
        Ok(())
    }

    /// Réécrit `.orchestra/config.json` depuis la configuration en mémoire (après édition
    /// des agents, par exemple). L'écriture reste dans le cœur.
    pub fn save_config(&self) -> Result<(), OrchestraError> {
        let path = self.root.join(".orchestra").join("config.json");
        let json = serde_json::to_string_pretty(&self.config)?;
        fs::write(path, format!("{json}\n"))?;
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

/// Balaye récursivement `dir` (profondeur bornée) pour collecter les fichiers `.md`,
/// en ignorant les dossiers cachés (dont `.orchestra`) et le bruit de build.
fn scan_markdown(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > SCAN_MAX_DEPTH || out.len() >= SCAN_MAX_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if path.is_dir() {
            if name.starts_with('.') || SCAN_SKIP.contains(&name) {
                continue; // .orchestra, .git, target, node_modules…
            }
            scan_markdown(&path, depth + 1, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_lists_persona_adr_and_workspace_markdown() {
        let dir = std::env::temp_dir().join(format!("orch-docs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".orchestra/adr")).unwrap();
        fs::create_dir_all(dir.join("docs")).unwrap();
        fs::write(dir.join(".orchestra/persona.md"), "# Persona").unwrap();
        fs::write(dir.join(".orchestra/adr/0001-choix.md"), "# ADR").unwrap();
        fs::write(dir.join("docs/lecons.md"), "# Leçons").unwrap();
        fs::write(dir.join("notes.txt"), "ignore").unwrap(); // non-markdown ignoré

        let config = ProjectConfig {
            project_name: "T".into(),
            project_type: crate::model::project_type::ProjectType::Langue,
            workspace_path: None,
            documentalist_enabled: false,
            skills: vec![],
            agents: vec![],
            integrations: Default::default(),
        };
        let space = ContextSpace {
            root: dir.clone(),
            config,
            persona: Some("# Persona".into()),
            adrs: ContextSpace::load_adrs(&dir.join(".orchestra/adr")),
        };

        let docs = space.documents();
        let kinds: Vec<_> = docs.iter().map(|d| d.kind).collect();
        assert!(kinds.contains(&DocKind::Persona));
        assert!(kinds.contains(&DocKind::Adr));
        // Le markdown du workspace est trouvé, le .txt non, le .orchestra non re-listé.
        assert!(docs.iter().any(|d| d.label == "docs/lecons.md" && d.kind == DocKind::Doc));
        assert!(!docs.iter().any(|d| d.label.contains("notes")));
        assert_eq!(docs.iter().filter(|d| d.kind == DocKind::Doc).count(), 1);

        let _ = fs::remove_dir_all(&dir);
    }
}
