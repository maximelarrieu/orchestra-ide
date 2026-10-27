//! Which Claude account a project's agents use.
//!
//! The account follows the project's folder: what its main repository's
//! `.envrc` exports as `CLAUDE_CONFIG_DIR`, unless `.orchestra/account.toml`
//! says otherwise (`orchestra_core::config::project_claude_config_dir`).
//! Worktrees live under the daemon's data dir, out of reach of any `.envrc`,
//! so the question is always asked of the main repository.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use orchestra_core::config::{project_claude_config_dir, DaemonConfig, ProjectAccount};
use tokio::process::Command;

/// A `.envrc` that takes longer than this is not waited for.
const DIRENV_TIMEOUT: Duration = Duration::from_secs(10);

/// The `CLAUDE_CONFIG_DIR` for `repo`'s agents; `None` is `~/.claude`.
///
/// Read at each use, like the project's other `.orchestra` files. Best effort:
/// no direnv, a blocked `.envrc` or an unreadable file each fall through to
/// the next source, with a line in the log.
pub async fn claude_config_dir(daemon: &DaemonConfig, repo: &Path) -> Option<PathBuf> {
    let account = match std::fs::read_to_string(repo.join(".orchestra/account.toml")) {
        Ok(src) => ProjectAccount::parse(&src).unwrap_or_else(|e| {
            tracing::warn!("{e} — le compte vient du .envrc ou de la configuration");
            ProjectAccount::default()
        }),
        Err(_) => ProjectAccount::default(),
    };
    let from_envrc = if account.claude_config_dir.is_some() {
        None
    } else {
        from_envrc(repo).await
    };
    project_claude_config_dir(&account, from_envrc, daemon)
}

/// What `repo`'s `.envrc` exports as `CLAUDE_CONFIG_DIR`, if anything.
async fn from_envrc(repo: &Path) -> Option<PathBuf> {
    if !has_envrc(repo) {
        return None;
    }
    let mut cmd = Command::new("direnv");
    cmd.arg("exec")
        .arg(repo)
        .args(["printenv", "CLAUDE_CONFIG_DIR"])
        .current_dir(repo)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    // Only the `.envrc` may answer: not the daemon's own environment, nor a
    // direnv state it inherited from the shell it was started in.
    for key in ["CLAUDE_CONFIG_DIR", "DIRENV_DIR", "DIRENV_FILE", "DIRENV_DIFF", "DIRENV_WATCHES"] {
        cmd.env_remove(key);
    }
    let out = match tokio::time::timeout(DIRENV_TIMEOUT, cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            tracing::warn!("direnv injoignable pour {} : {e}", repo.display());
            return None;
        }
        Err(_) => {
            tracing::warn!("direnv sans réponse pour {} après {DIRENV_TIMEOUT:?}", repo.display());
            return None;
        }
    };
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("is blocked") {
        tracing::warn!(
            "le .envrc de {} n'est pas autorisé (direnv allow) : il ne choisit pas le compte",
            repo.display()
        );
    }
    let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !value.is_empty()).then(|| PathBuf::from(value))
}

/// direnv reads the nearest `.envrc` up the tree, so the folder the project
/// sits in counts as much as the project itself.
fn has_envrc(repo: &Path) -> bool {
    repo.ancestors().any(|dir| dir.join(".envrc").is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_project_without_envrc_takes_the_daemon_default() {
        let dir = tempfile::tempdir().unwrap();
        let mut daemon = DaemonConfig::default();
        assert_eq!(claude_config_dir(&daemon, dir.path()).await, None);
        daemon.claude_config_dir = Some(PathBuf::from("/d/.claude-pro"));
        assert_eq!(
            claude_config_dir(&daemon, dir.path()).await,
            Some(PathBuf::from("/d/.claude-pro"))
        );
    }

    #[tokio::test]
    async fn account_toml_overrides_without_asking_direnv() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".orchestra")).unwrap();
        std::fs::write(
            dir.path().join(".orchestra/account.toml"),
            "claude_config_dir = \"/p/.claude-perso\"\n",
        )
        .unwrap();
        // An .envrc that would say otherwise, and that direnv has not allowed.
        std::fs::write(dir.path().join(".envrc"), "export CLAUDE_CONFIG_DIR=/e/autre\n").unwrap();
        assert_eq!(
            claude_config_dir(&DaemonConfig::default(), dir.path()).await,
            Some(PathBuf::from("/p/.claude-perso"))
        );
    }

    #[tokio::test]
    async fn a_broken_account_toml_falls_through() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".orchestra")).unwrap();
        std::fs::write(dir.path().join(".orchestra/account.toml"), "compte = 1\n").unwrap();
        assert_eq!(claude_config_dir(&DaemonConfig::default(), dir.path()).await, None);
    }
}
