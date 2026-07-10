//! Arborescence de fichiers d'un workspace — la donnée de **l'explorateur** (panneau gauche,
//! façon Cursor). Logique pure et partagée TUI ⇄ GUI : chaque UI dessine la liste retournée, et
//! l'annote avec l'activité live des agents (« orchestre en verre ») via les chemins relatifs.
//!
//! On produit une liste **à plat** (avec profondeur) plutôt qu'un arbre imbriqué : c'est trivial
//! à rendre des deux côtés (une ligne = une entrée, indentée selon `depth`).

use std::path::Path;

/// Dossiers ignorés (bruit / volumineux) — jamais parcourus.
const SKIP_DIRS: &[&str] = &[
    ".git", ".orchestra", "target", "node_modules", "dist", "build", ".next", ".venv", "__pycache__",
];

/// Nombre maximum d'entrées retournées (garde-fou sur les gros dépôts). Au-delà, la liste est
/// tronquée et [`Tree::truncated`] passe à vrai.
const MAX_ENTRIES: usize = 800;

/// Une entrée de l'arborescence (fichier ou dossier).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileNode {
    /// Nom court (dernier segment).
    pub name: String,
    /// Chemin **relatif** à la racine du workspace, séparateurs `/` (correspond aux chemins des
    /// événements `FileRead`/`FileChanged`).
    pub rel: String,
    /// Vrai si c'est un dossier.
    pub is_dir: bool,
    /// Profondeur d'indentation (0 = racine).
    pub depth: usize,
}

/// Résultat d'un parcours : les entrées + si la liste a été tronquée (dépôt trop grand).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tree {
    pub nodes: Vec<FileNode>,
    pub truncated: bool,
}

/// Construit l'arborescence de `root` (dossiers d'abord, ordre alphabétique insensible à la
/// casse), en ignorant les dossiers cachés et volumineux ([`SKIP_DIRS`]). Bornée à
/// [`MAX_ENTRIES`] entrées.
pub fn tree(root: &Path) -> Tree {
    let mut out = Tree::default();
    walk(root, "", 0, &mut out);
    out
}

fn walk(dir: &Path, prefix: &str, depth: usize, out: &mut Tree) {
    if out.nodes.len() >= MAX_ENTRIES {
        out.truncated = true;
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };

    // On matérialise et trie : dossiers d'abord, puis alphabétique (insensible à la casse).
    let mut entries: Vec<(String, std::path::PathBuf, bool)> = Vec::new();
    for e in rd.flatten() {
        let path = e.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
        if name.starts_with('.') {
            continue; // fichiers/dossiers cachés masqués
        }
        let is_dir = path.is_dir();
        if is_dir && SKIP_DIRS.contains(&name) {
            continue;
        }
        entries.push((name.to_string(), path, is_dir));
    }
    entries.sort_by(|a, b| {
        b.2.cmp(&a.2).then(a.0.to_lowercase().cmp(&b.0.to_lowercase()))
    });

    for (name, path, is_dir) in entries {
        if out.nodes.len() >= MAX_ENTRIES {
            out.truncated = true;
            return;
        }
        let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        out.nodes.push(FileNode { name: name.clone(), rel: rel.clone(), is_dir, depth });
        if is_dir {
            walk(&path, &rel, depth + 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn lists_files_dirs_first_and_skips_noise() {
        let base = std::env::temp_dir().join(format!(
            "orch-tree-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("src")).unwrap();
        fs::create_dir_all(base.join("target/debug")).unwrap(); // ignoré
        fs::create_dir_all(base.join(".git")).unwrap(); // ignoré
        fs::write(base.join("README.md"), "x").unwrap();
        fs::write(base.join("src/main.rs"), "x").unwrap();
        fs::write(base.join(".hidden"), "x").unwrap(); // caché → ignoré

        let t = tree(&base);
        let rels: Vec<&str> = t.nodes.iter().map(|n| n.rel.as_str()).collect();
        // Dossier `src` en tête (avant le fichier README), son contenu indenté juste après.
        assert_eq!(rels, vec!["src", "src/main.rs", "README.md"]);
        assert!(!rels.iter().any(|r| r.contains("target") || r.contains(".git") || r.contains("hidden")));
        // Profondeurs cohérentes.
        let main = t.nodes.iter().find(|n| n.rel == "src/main.rs").unwrap();
        assert_eq!(main.depth, 1);
        assert!(!main.is_dir);
        assert!(!t.truncated);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn empty_or_missing_dir_is_empty() {
        assert!(tree(Path::new("/does/not/exist/orchestra")).nodes.is_empty());
    }
}
