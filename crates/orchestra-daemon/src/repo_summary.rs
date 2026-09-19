//! A compact picture of a repository, for the orchestrator's prompt.
//!
//! The orchestrator can read files itself, but it needs a map first. This
//! builds one that stays under a few thousand tokens whatever the repository
//! size: a folded tree, the top of the README, the project's own instructions,
//! and recent history.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// Ceilings, chosen so a large repository still fits in a prompt.
const MAX_TREE_ENTRIES: usize = 300;
const MAX_DEPTH: usize = 3;
const MAX_README_LINES: usize = 120;
const MAX_INSTRUCTIONS_CHARS: usize = 6_000;
const MAX_COMMITS: usize = 20;

pub struct RepoSummary {
    pub text: String,
}

impl RepoSummary {
    /// Build the summary. Every part is best effort: a repository without git,
    /// without a README or empty still yields something usable.
    pub fn build(root: &Path) -> Self {
        let mut out = String::new();

        if let Some(branch) = git(root, &["symbolic-ref", "--short", "HEAD"]) {
            out.push_str(&format!("Branche courante : {branch}\n"));
        }
        if let Some(langs) = language_histogram(root) {
            out.push_str(&format!("Langages : {langs}\n"));
        }
        out.push('\n');

        if let Some(tree) = tree(root) {
            out.push_str("## Arborescence\n\n```\n");
            out.push_str(&tree);
            out.push_str("```\n\n");
        }

        for name in ["CLAUDE.md", "AGENTS.md"] {
            if let Some(text) = read_capped(&root.join(name), MAX_INSTRUCTIONS_CHARS) {
                out.push_str(&format!("## {name}\n\n{text}\n\n"));
                break;
            }
        }

        if let Some((name, text)) = read_readme(root) {
            out.push_str(&format!("## {name} (début)\n\n{text}\n\n"));
        }

        if let Some(log) = git(
            root,
            &[
                "log",
                "--oneline",
                "--no-decorate",
                &format!("-{MAX_COMMITS}"),
            ],
        ) {
            if !log.is_empty() {
                out.push_str("## Derniers commits\n\n```\n");
                out.push_str(&log);
                out.push_str("\n```\n");
            }
        }

        RepoSummary { text: out }
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// Files git knows about, folded into a tree capped in depth and width.
fn tree(root: &Path) -> Option<String> {
    let listing = git(root, &["ls-files"])?;
    let mut dirs: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;

    for path in listing.lines() {
        total += 1;
        let parts: Vec<&str> = path.split('/').collect();
        // A file at depth <= MAX_DEPTH is listed under its directory; deeper
        // ones are folded into their ancestor at MAX_DEPTH.
        let dir = if parts.len() <= 1 {
            ".".to_string()
        } else {
            parts[..parts.len().saturating_sub(1).min(MAX_DEPTH)].join("/")
        };
        *dirs.entry(dir).or_insert(0) += 1;
    }
    if total == 0 {
        return None;
    }

    let mut out = String::new();
    let shown = dirs.len().min(MAX_TREE_ENTRIES);
    for (dir, count) in dirs.iter().take(shown) {
        let label = if dir == "." { "(racine)" } else { dir };
        out.push_str(&format!("{label}/  {count} fichier(s)\n"));
    }
    if dirs.len() > shown {
        out.push_str(&format!("… et {} dossiers de plus\n", dirs.len() - shown));
    }
    out.push_str(&format!("total : {total} fichiers suivis\n"));
    Some(out)
}

/// Extension counts, so the orchestrator knows what it is looking at.
fn language_histogram(root: &Path) -> Option<String> {
    let listing = git(root, &["ls-files"])?;
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for path in listing.lines() {
        if let Some(ext) = path.rsplit_once('.').map(|(_, e)| e) {
            if ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
                *counts.entry(ext).or_insert(0) += 1;
            }
        }
    }
    if counts.is_empty() {
        return None;
    }
    let mut top: Vec<(&str, usize)> = counts.into_iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    Some(
        top.into_iter()
            .take(6)
            .map(|(ext, n)| format!("{ext} ({n})"))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn read_readme(root: &Path) -> Option<(String, String)> {
    for name in ["README.md", "README.rst", "README.txt", "README"] {
        let path = root.join(name);
        if let Ok(text) = std::fs::read_to_string(&path) {
            let head: Vec<&str> = text.lines().take(MAX_README_LINES).collect();
            return Some((name.to_string(), head.join("\n")));
        }
    }
    None
}

fn read_capped(path: &Path, max_chars: usize) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    if text.chars().count() <= max_chars {
        return Some(text);
    }
    let head: String = text.chars().take(max_chars).collect();
    Some(format!("{head}\n… (tronqué)"))
}

/// Run git in `root`, returning trimmed stdout on success.
fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "t"],
        ] {
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(&args)
                .output()
                .unwrap();
        }
        dir
    }

    fn commit(root: &Path, message: &str) {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["add", "-A"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["commit", "-q", "-m", message])
            .output()
            .unwrap();
    }

    #[test]
    fn a_real_repository_is_summarised() {
        let dir = repo();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/deep/deeper/deepest")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn a() {}").unwrap();
        std::fs::write(root.join("src/deep/deeper/deepest/x.rs"), "//").unwrap();
        std::fs::write(root.join("README.md"), "# Mon projet\n\nFait des choses.").unwrap();
        std::fs::write(root.join("CLAUDE.md"), "Règle : ne pas pousser.").unwrap();
        commit(root, "premier commit");

        let s = RepoSummary::build(root);
        assert!(!s.is_empty());
        assert!(s.text.contains("Arborescence"));
        assert!(s.text.contains("src/"));
        assert!(s.text.contains("total : 5 fichiers suivis"));
        assert!(s.text.contains("Mon projet"), "le README est repris");
        assert!(
            s.text.contains("ne pas pousser"),
            "les consignes du projet aussi"
        );
        assert!(s.text.contains("premier commit"));
        assert!(s.text.contains("rs (3)"), "les langages sont comptés");
    }

    #[test]
    fn deep_paths_are_folded_rather_than_listed() {
        let dir = repo();
        let root = dir.path();
        std::fs::create_dir_all(root.join("a/b/c/d/e")).unwrap();
        std::fs::write(root.join("a/b/c/d/e/f.rs"), "//").unwrap();
        commit(root, "c");
        let s = RepoSummary::build(root);
        assert!(s.text.contains("a/b/c/"), "replié à la profondeur trois");
        assert!(!s.text.contains("a/b/c/d/e"), "pas de chemin plus profond");
    }

    #[test]
    fn a_directory_without_git_still_yields_something() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "# Sans git").unwrap();
        let s = RepoSummary::build(dir.path());
        assert!(s.text.contains("Sans git"));
        assert!(!s.text.contains("Arborescence"));
    }

    #[test]
    fn an_empty_repository_is_not_a_failure() {
        let dir = repo();
        let s = RepoSummary::build(dir.path());
        // Nothing tracked yet: no tree, but no panic either.
        assert!(!s.text.contains("Arborescence"));
    }

    #[test]
    fn long_files_are_capped() {
        let dir = repo();
        let root = dir.path();
        let huge = "x".repeat(MAX_INSTRUCTIONS_CHARS * 3);
        std::fs::write(root.join("CLAUDE.md"), &huge).unwrap();
        std::fs::write(root.join("README.md"), "ligne\n".repeat(500)).unwrap();
        commit(root, "c");
        let s = RepoSummary::build(root);
        assert!(s.text.contains("(tronqué)"));
        // The README contributes at most its first lines.
        let readme_lines = s.text.lines().filter(|l| l.trim() == "ligne").count();
        assert!(
            readme_lines <= MAX_README_LINES,
            "{readme_lines} lignes reprises"
        );
    }
}
