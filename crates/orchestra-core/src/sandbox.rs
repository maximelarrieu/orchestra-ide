//! The filesystem a sandboxed agent sees, and the `bwrap` arguments that make
//! it so. Pure: the daemon decides which paths exist and hands them in.
//!
//! The guard hook refuses what it recognises; this takes away what it cannot
//! foresee. The whole disk is read-only, the paths an agent legitimately
//! writes are bound back writable, and the places holding credentials are
//! covered with an empty directory. The network stays open: `claude` needs it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SandboxMode {
    /// No wrapper: the guard hook alone.
    #[default]
    Off,
    /// Wrap each agent in `bwrap` (bubblewrap).
    Bwrap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    pub mode: SandboxMode,
    /// Directories covered with an empty one for roles whose git is
    /// `confined`. `~` is the home directory. A role with `git: full` (the
    /// integrator) keeps them: it needs the credentials to push.
    pub hide: Vec<String>,
    /// Directories writable on top of the worktree and the account directory.
    pub writable: Vec<String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        SandboxConfig {
            mode: SandboxMode::Off,
            hide: ["~/.ssh", "~/.aws", "~/.gnupg", "~/.config/gcloud", "~/.config/gh"]
                .map(String::from)
                .to_vec(),
            writable: Vec::new(),
        }
    }
}

/// What one agent may write and may not see.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sandbox {
    pub writable: Vec<PathBuf>,
    pub hidden: Vec<PathBuf>,
}

impl Sandbox {
    /// The arguments to put between `bwrap` and the command.
    ///
    /// Order matters: hidden paths are covered after the read-only root and
    /// before the writable binds, so a writable path inside a hidden one
    /// still shows through.
    pub fn bwrap_args(&self) -> Vec<String> {
        let mut args: Vec<String> = ["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc"]
            .map(String::from)
            .to_vec();
        args.extend(["--tmpfs", "/tmp"].map(String::from));
        for path in &self.hidden {
            args.extend(["--tmpfs".into(), path.display().to_string()]);
        }
        for path in &self.writable {
            let p = path.display().to_string();
            args.extend(["--bind".into(), p.clone(), p]);
        }
        args.extend(["--die-with-parent", "--new-session"].map(String::from));
        args.push("--".into());
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_root_is_read_only_and_the_worktree_bound_back_writable() {
        let s = Sandbox {
            writable: vec!["/work/t1".into()],
            hidden: vec!["/home/u/.ssh".into()],
        };
        let args = s.bwrap_args();
        assert_eq!(&args[..3], ["--ro-bind", "/", "/"]);
        let pos = |needle: &[&str]| {
            args.windows(needle.len())
                .position(|w| w == needle)
                .unwrap_or_else(|| panic!("{needle:?} absent de {args:?}"))
        };
        assert!(pos(&["--tmpfs", "/home/u/.ssh"]) < pos(&["--bind", "/work/t1", "/work/t1"]));
        assert_eq!(args.last().map(String::as_str), Some("--"));
    }

    #[test]
    fn off_is_the_default() {
        assert_eq!(SandboxConfig::default().mode, SandboxMode::Off);
    }
}
