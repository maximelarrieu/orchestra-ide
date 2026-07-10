//! Registre **global** des Espaces connus, indépendant d'un espace donné : permet aux UIs de
//! lister et rouvrir les espaces déjà utilisés (récents d'abord) **sans retaper leur chemin**.
//! Partagé TUI ⇄ GUI (cf. `CLAUDE.md`). Stocké dans `<config>/orchestra/spaces.json`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::OrchestraError;
use crate::model::ContextSpace;

/// Un espace connu : nom (lu dans sa config) + chemin sur disque.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownSpace {
    pub name: String,
    pub path: PathBuf,
}

/// Borne du nombre d'espaces mémorisés (les plus anciens sont oubliés).
const MAX_ENTRIES: usize = 30;

/// Dossier de configuration global (multi-plateforme, sans dépendance externe) :
/// `%APPDATA%\orchestra` (Windows) · `$XDG_CONFIG_HOME/orchestra` · `$HOME/.config/orchestra`.
fn config_dir() -> PathBuf {
    for var in ["APPDATA", "XDG_CONFIG_HOME"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return PathBuf::from(v).join("orchestra");
            }
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".config").join("orchestra");
        }
    }
    PathBuf::from(".orchestra-registry")
}

/// Chemin du fichier de registre.
pub fn registry_path() -> PathBuf {
    config_dir().join("spaces.json")
}

/// Forme normalisée d'un chemin (pour dédoublonner) ; repli sur le chemin brut si introuvable.
fn normalize(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn read(file: &Path) -> Vec<KnownSpace> {
    fs::read_to_string(file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write(file: &Path, list: &[KnownSpace]) -> Result<(), OrchestraError> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(file, serde_json::to_string_pretty(list)?)?;
    Ok(())
}

/// Espaces connus (récents d'abord). Fichier absent/illisible → liste vide.
pub fn known_spaces() -> Vec<KnownSpace> {
    read(&registry_path())
}

/// Mémorise un espace (ou le remonte en tête) après ouverture réussie. **Valide** qu'il s'agit
/// bien d'un espace (chargeable via le cœur) et lit son nom. Renvoie l'entrée mémorisée.
pub fn remember_space(path: &Path) -> Result<KnownSpace, OrchestraError> {
    remember_in(&registry_path(), path)
}

/// Retire un espace du registre (« ne plus suivre »).
pub fn forget_space(path: &Path) {
    forget_in(&registry_path(), path);
}

fn remember_in(file: &Path, path: &Path) -> Result<KnownSpace, OrchestraError> {
    let space = ContextSpace::load(path)?; // valide l'espace + lit son nom
    let entry = KnownSpace { name: space.config.project_name.clone(), path: normalize(path) };
    let mut list = read(file);
    list.retain(|k| normalize(&k.path) != entry.path);
    list.insert(0, entry.clone());
    list.truncate(MAX_ENTRIES);
    write(file, &list)?;
    Ok(entry)
}

fn forget_in(file: &Path, path: &Path) {
    let target = normalize(path);
    let mut list = read(file);
    list.retain(|k| normalize(&k.path) != target);
    let _ = write(file, &list);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProjectConfig;

    fn make_space(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "orch-reg-space-{}-{:?}-{tag}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".orchestra")).unwrap();
        let cfg = ProjectConfig {
            project_name: format!("Space_{tag}"),
            workspace_path: None,
            integrations: Default::default(),
        };
        fs::write(
            root.join(".orchestra").join("config.json"),
            serde_json::to_string(&cfg).unwrap(),
        )
        .unwrap();
        root
    }

    fn tmp_registry() -> PathBuf {
        std::env::temp_dir().join(format!(
            "orch-reg-{}-{:?}.json",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn remember_dedups_and_orders_recent_first() {
        let reg = tmp_registry();
        let _ = fs::remove_file(&reg);
        let a = make_space("a");
        let b = make_space("b");

        remember_in(&reg, &a).unwrap();
        remember_in(&reg, &b).unwrap();
        remember_in(&reg, &a).unwrap(); // remonte « a » en tête, sans doublon

        let list = read(&reg);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Space_a");
        assert_eq!(list[1].name, "Space_b");

        forget_in(&reg, &a);
        let list = read(&reg);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Space_b");

        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
        let _ = fs::remove_file(&reg);
    }

    #[test]
    fn remember_rejects_non_space() {
        let reg = tmp_registry();
        let bogus = std::env::temp_dir().join("orch-reg-nope-xyz-zzz");
        let _ = fs::remove_dir_all(&bogus);
        assert!(remember_in(&reg, &bogus).is_err());
    }
}
