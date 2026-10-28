//! Turns `[sandbox]` into the paths one agent gets (`orchestra_core::sandbox`).
//! The only I/O of the feature lives here: does `bwrap` exist, which of the
//! configured paths exist, where is the repository's own `.git`.

use std::path::{Path, PathBuf};

use orchestra_core::config::Config;
use orchestra_core::guard::GitPolicy;
use orchestra_core::sandbox::{Sandbox, SandboxMode};

/// `None` when the sandbox is off, or when `bwrap` cannot be found (the second
/// case is the caller's to report: it is `Err`).
pub fn for_agent(
    cfg: &Config,
    worktree: &Path,
    config_dir: Option<&Path>,
    git: GitPolicy,
) -> Result<Option<Sandbox>, String> {
    if cfg.sandbox.mode == SandboxMode::Off {
        return Ok(None);
    }
    if which("bwrap").is_none() {
        return Err("« bwrap » est introuvable : l'agent tourne sans bac à sable".into());
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let expand = |s: &str| -> Option<PathBuf> {
        match s.strip_prefix("~/") {
            Some(rest) => home.as_ref().map(|h| h.join(rest)),
            None => Some(PathBuf::from(s)),
        }
    };

    let mut writable = vec![worktree.to_path_buf()];
    match config_dir {
        Some(dir) => writable.push(dir.to_path_buf()),
        None => {
            if let Some(h) = &home {
                writable.push(h.join(".claude"));
            }
        }
    }
    if let Some(h) = &home {
        writable.push(h.join(".claude.json"));
        writable.push(h.join(".cache"));
        writable.push(h.join(".local/share/claude"));
        writable.push(h.join(".local/state/claude"));
    }
    writable.extend(cfg.sandbox.writable.iter().filter_map(|s| expand(s)));
    // A role with full git commits and merges through the repository's own
    // `.git`, which the worktree only points to.
    if git == GitPolicy::Full {
        writable.extend(common_git_dir(worktree));
    }
    writable.retain(|p| p.exists());

    let hidden = match git {
        GitPolicy::Full => Vec::new(),
        GitPolicy::Confined => cfg
            .sandbox
            .hide
            .iter()
            .filter_map(|s| expand(s))
            .filter(|p| p.is_dir())
            .collect(),
    };
    Ok(Some(Sandbox { writable, hidden }))
}

/// The repository's `.git` directory, read from the worktree's `.git` file
/// (`gitdir: <repo>/.git/worktrees/<name>`).
pub fn common_git_dir(worktree: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(worktree.join(".git")).ok()?;
    let gitdir = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
    // <repo>/.git/worktrees/<name> -> <repo>/.git
    gitdir.parent()?.parent().map(Path::to_path_buf)
}

fn which(bin: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(bin))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_repository_git_dir_is_found_from_a_worktree_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".git"),
            "gitdir: /repo/.git/worktrees/orch-12\n",
        )
        .unwrap();
        assert_eq!(common_git_dir(dir.path()), Some(PathBuf::from("/repo/.git")));
        let none = tempfile::tempdir().unwrap();
        assert_eq!(common_git_dir(none.path()), None);
    }

    #[test]
    fn off_gives_no_sandbox() {
        let cfg = Config::default();
        let got = for_agent(&cfg, Path::new("/tmp"), None, GitPolicy::Confined);
        assert_eq!(got, Ok(None));
    }

    #[test]
    fn a_confined_role_hides_credentials_and_a_full_one_does_not() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join(".ssh")).unwrap();
        let wt = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.sandbox.mode = SandboxMode::Bwrap;
        cfg.sandbox.hide = vec![home.path().join(".ssh").display().to_string()];
        // `bwrap` may be missing on the machine running the tests.
        if which("bwrap").is_none() {
            return;
        }
        let confined = for_agent(&cfg, wt.path(), None, GitPolicy::Confined).unwrap().unwrap();
        assert_eq!(confined.hidden, vec![home.path().join(".ssh")]);
        assert!(confined.writable.contains(&wt.path().to_path_buf()));
        let full = for_agent(&cfg, wt.path(), None, GitPolicy::Full).unwrap().unwrap();
        assert!(full.hidden.is_empty());
    }
}
