//! What a ticket's worktree needs from the main checkout that git does not carry.
//!
//! A worktree is a fresh checkout: everything the repository ignores is missing
//! from it. The virtualenv, the local `.env`, a developer's own `CLAUDE.md` or
//! launch script — without them the app does not even import, and an agent can
//! neither run the tests nor show its work running. The project says which of
//! those files its worktrees get, in `.orchestra/worktree.toml`:
//!
//! ```toml
//! copy = [".env.local", "dev.sh", "CLAUDE.md"]
//! link = [".venv"]
//! ```
//!
//! **Copy by default, link only on request.** The guard compares paths, it does
//! not follow links: an agent editing `CLAUDE.md` through a link would rewrite
//! the main checkout's file while the guard sees a path inside the worktree. A
//! copy keeps whatever the agent does in its box. A link is for what is too big
//! to copy and is not meant to be edited, such as a virtualenv or
//! `node_modules` — the price being that a `pip install` there is shared.
//!
//! Like `.orchestra/checks.toml`, the file is read from the main repository,
//! never from the ticket's branch. This module holds no I/O: it parses the file
//! and says which entries are acceptable; the daemon copies and links.

use std::path::{Component, Path, PathBuf};

/// `.orchestra/worktree.toml`, as written.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorktreeFiles {
    /// Copied into the worktree: files or whole directories.
    pub copy: Vec<String>,
    /// Linked from the worktree to the main checkout.
    pub link: Vec<String>,
}

/// How one entry reaches the worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Copy,
    Link,
}

/// One entry, checked: a path relative to the repository root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub mode: Mode,
}

impl WorktreeFiles {
    pub fn parse(src: &str) -> crate::error::Result<Self> {
        toml::from_str(src)
            .map_err(|e| crate::error::CoreError::Config(format!(".orchestra/worktree.toml : {e}")))
    }

    /// The entries to provide, and the ones refused with why.
    ///
    /// Refused rather than dropped in silence: a line that does nothing is a
    /// worktree that lacks a file, and the user should learn it from the
    /// ticket, not from a failing test.
    pub fn entries(&self) -> (Vec<Entry>, Vec<String>) {
        let mut ok = Vec::new();
        let mut refused = Vec::new();
        let all = self
            .copy
            .iter()
            .map(|p| (p, Mode::Copy))
            .chain(self.link.iter().map(|p| (p, Mode::Link)));
        for (raw, mode) in all {
            match relative_entry(raw) {
                Some(path) if ok.iter().any(|e: &Entry| e.path == path) => {
                    refused.push(format!("« {raw} » est listé deux fois"));
                }
                Some(path) => ok.push(Entry { path, mode }),
                None => refused.push(format!(
                    "« {raw} » refusé : un chemin relatif à la racine du dépôt, sans « .. » ni .git"
                )),
            }
        }
        (ok, refused)
    }
}

/// A path that stays inside the repository and out of its `.git`.
///
/// Absolute paths and `..` would let the file name a place outside both the
/// main checkout and the worktree; `.git` is git's, and a worktree's `.git` is a
/// file that must not be replaced.
pub fn relative_entry(raw: &str) -> Option<PathBuf> {
    let path = Path::new(raw.trim());
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    let first = clean.components().next()?;
    if first.as_os_str() == ".git" {
        return None;
    }
    Some(clean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_and_link_are_read_and_a_typo_is_not_silent() {
        let f = WorktreeFiles::parse("copy = [\".env.local\"]\nlink = [\".venv\"]\n").unwrap();
        let (entries, refused) = f.entries();
        assert!(refused.is_empty());
        assert_eq!(
            entries,
            vec![
                Entry { path: ".env.local".into(), mode: Mode::Copy },
                Entry { path: ".venv".into(), mode: Mode::Link },
            ]
        );
        assert!(WorktreeFiles::parse("cpy = []").is_err());
    }

    #[test]
    fn only_paths_inside_the_repository_are_accepted() {
        assert_eq!(relative_entry("./svc/.env"), Some(PathBuf::from("svc/.env")));
        assert_eq!(relative_entry("node_modules/"), Some(PathBuf::from("node_modules")));
        for bad in ["", ".", "/etc/passwd", "../voisin/.env", "a/../../b", ".git", ".git/hooks"] {
            assert_eq!(relative_entry(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_refused_entry_says_why_and_the_others_still_go() {
        let f = WorktreeFiles {
            copy: vec!["../dehors".into(), "dev.sh".into()],
            link: vec!["dev.sh".into()],
        };
        let (entries, refused) = f.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(refused.len(), 2, "{refused:?}");
        assert!(refused[0].contains("../dehors"));
        assert!(refused[1].contains("deux fois"));
    }
}
