//! Navigateur de dossiers minimal pour **découvrir des espaces sans taper de chemin**. Partagé
//! TUI ⇄ GUI (cf. `CLAUDE.md`) : il liste les sous-dossiers d'un répertoire et marque ceux qui
//! sont des Espaces de Contexte (présence de `.orchestra/config.json`).

use std::fs;
use std::path::{Path, PathBuf};

/// Une entrée de dossier dans le navigateur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    /// Vrai si le dossier est un Espace de Contexte (contient `.orchestra/config.json`).
    pub is_space: bool,
}

/// Vrai si `dir` est un Espace de Contexte.
pub fn is_space(dir: &Path) -> bool {
    dir.join(".orchestra").join("config.json").is_file()
}

/// Sous-dossiers de `dir` (triés par nom, dossiers cachés ignorés), chacun marqué « espace »
/// ou non. Répertoire illisible → liste vide.
pub fn browse(dir: &Path) -> Vec<DirEntry> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else { return out };
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
        if name.starts_with('.') {
            continue; // on masque les dossiers cachés (dont .orchestra lui-même)
        }
        let is_space = is_space(&path);
        out.push(DirEntry { name: name.to_string(), path, is_space });
    }
    // Espaces d'abord, puis ordre alphabétique (insensible à la casse).
    out.sort_by(|a, b| b.is_space.cmp(&a.is_space).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/// Dossier parent (pour remonter dans l'arborescence).
pub fn parent(dir: &Path) -> Option<PathBuf> {
    dir.parent().map(Path::to_path_buf)
}

/// Point de départ raisonnable du navigateur : répertoire personnel, sinon répertoire courant.
pub fn home_dir() -> PathBuf {
    for var in ["HOME", "USERPROFILE"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return PathBuf::from(v);
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_marks_spaces_first() {
        let base = std::env::temp_dir().join(format!(
            "orch-browse-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("zzz_plain")).unwrap();
        fs::create_dir_all(base.join("aaa_space").join(".orchestra")).unwrap();
        fs::write(base.join("aaa_space").join(".orchestra").join("config.json"), "{}").unwrap();
        fs::create_dir_all(base.join(".cache")).unwrap(); // caché → ignoré

        let entries = browse(&base);
        assert_eq!(entries.len(), 2); // .cache ignoré
        assert!(entries[0].is_space); // l'espace passe en tête
        assert_eq!(entries[0].name, "aaa_space");
        assert!(!entries.iter().find(|e| e.name == "zzz_plain").unwrap().is_space);

        let _ = fs::remove_dir_all(&base);
    }
}
