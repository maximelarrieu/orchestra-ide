//! The guard that keeps an agent inside its worktree.
//!
//! Agents run with permissions bypassed, so this is the only thing standing
//! between a mistaken command and the rest of the machine. It lives in the core
//! crate because two very different programs need the exact same rules: the
//! daemon, to explain them, and `orchestra-hook`, a dependency-free binary that
//! Claude Code runs before every tool call.
//!
//! The decision is deliberately local. Asking the daemon would add a socket
//! round trip to every tool call and would make an agent depend on the daemon
//! being healthy.
//!
//! The v1 of this project had no such guard, and an agent ran `mv *.md docs/`
//! on the repository itself.

use std::path::{Component, Path, PathBuf};

/// What the guard decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// Refuse, with the reason the agent will read.
    Deny(String),
}

impl Verdict {
    pub fn is_denied(&self) -> bool {
        matches!(self, Verdict::Deny(_))
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Verdict::Deny(r) => Some(r),
            Verdict::Allow => None,
        }
    }
}

/// The box an agent is allowed to touch.
#[derive(Debug, Clone)]
pub struct Boundary {
    /// The worktree; everything the agent writes must be under it.
    pub worktree: PathBuf,
    /// Extra directories it may read and write, such as a scratch space.
    pub extra: Vec<PathBuf>,
}

impl Boundary {
    pub fn new(worktree: impl Into<PathBuf>) -> Self {
        Boundary {
            worktree: worktree.into(),
            extra: Vec::new(),
        }
    }

    fn contains(&self, path: &Path) -> bool {
        let candidate = normalise(path);
        std::iter::once(&self.worktree)
            .chain(self.extra.iter())
            .any(|root| candidate.starts_with(normalise(root)))
    }
}

/// Git subcommands an agent must never run: they move work out of the box or
/// throw it away.
const FORBIDDEN_GIT: [&str; 6] = ["push", "checkout", "switch", "worktree", "remote", "clean"];

/// Decide on one tool call.
///
/// `tool` is Claude Code's tool name and `input` its arguments as the hook
/// receives them.
pub fn check(boundary: &Boundary, tool: &str, input: &serde_json::Value) -> Verdict {
    match tool {
        "Bash" | "BashOutput" => {
            let command = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            check_command(boundary, command)
        }
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            let path = input
                .get("file_path")
                .or_else(|| input.get("notebook_path"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            check_write_path(boundary, path)
        }
        // Reading outside the worktree is how an agent learns the codebase it
        // is changing; only writing is confined.
        _ => Verdict::Allow,
    }
}

/// Inspect a shell command.
pub fn check_command(boundary: &Boundary, command: &str) -> Verdict {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Verdict::Allow;
    }

    if let Some(reason) = catastrophic(trimmed) {
        return Verdict::Deny(reason);
    }

    for word in tokens(trimmed) {
        // A path written out in full is checked whatever the command is: it is
        // the one signal that does not depend on knowing every tool's flags.
        if word.starts_with('/') || word.starts_with("~/") {
            let word = word.as_str();
            let path = expand(word);
            if !boundary.contains(&path) && !readable_system_path(&path) {
                return Verdict::Deny(format!(
                    "« {} » est hors du worktree du ticket ({}). \
                     Travaille uniquement dans ton répertoire de travail.",
                    word,
                    boundary.worktree.display()
                ));
            }
        }
    }

    if let Some(sub) = git_subcommand(trimmed) {
        if FORBIDDEN_GIT.contains(&sub.as_str()) {
            return Verdict::Deny(format!(
                "« git {sub} » est interdit : ta branche est relue avant toute fusion, \
                 et changer de branche ferait perdre le travail en cours."
            ));
        }
    }

    Verdict::Allow
}

/// Inspect a path a tool wants to write.
pub fn check_write_path(boundary: &Boundary, path: &str) -> Verdict {
    if path.is_empty() {
        return Verdict::Allow;
    }
    let expanded = expand(path);
    // A relative path is resolved against the worktree, which is the agent's
    // working directory.
    let candidate = if expanded.is_absolute() {
        expanded
    } else {
        boundary.worktree.join(expanded)
    };
    if boundary.contains(&candidate) {
        Verdict::Allow
    } else {
        Verdict::Deny(format!(
            "écrire dans « {path} » sort du worktree du ticket ({}).",
            boundary.worktree.display()
        ))
    }
}

/// Commands that are wrong wherever they run.
fn catastrophic(command: &str) -> Option<String> {
    let flat = command.split_whitespace().collect::<Vec<_>>().join(" ");
    let patterns = [
        ("rm -rf /", "effacer la racine du système"),
        ("rm -fr /", "effacer la racine du système"),
        ("mkfs", "formater un système de fichiers"),
        ("dd if=", "écrire directement sur un périphérique"),
        (":(){", "une bombe de forks"),
        ("shutdown", "éteindre la machine"),
        ("reboot", "redémarrer la machine"),
    ];
    for (needle, what) in patterns {
        if flat.contains(needle) {
            return Some(format!(
                "commande refusée : elle tenterait de {what}. Rien de ce que \
                 demande ce ticket ne le justifie."
            ));
        }
    }
    None
}

/// The subcommand of a `git …` invocation, if that is what this is.
fn git_subcommand(command: &str) -> Option<String> {
    let mut words = tokens(command).into_iter().peekable();
    while let Some(word) = words.next() {
        if word != "git" {
            continue;
        }
        // Skip `-C <dir>` and other options to reach the subcommand.
        let mut rest = words.clone();
        while let Some(next) = rest.next() {
            if next == "-C" || next == "--git-dir" || next == "--work-tree" {
                rest.next();
                continue;
            }
            if next.starts_with('-') {
                continue;
            }
            return Some(next);
        }
    }
    None
}

/// Split on whitespace, dropping quotes so a quoted path is still seen.
fn tokens(command: &str) -> Vec<String> {
    command
        .split(|c: char| c.is_whitespace() || c == ';' || c == '|' || c == '&')
        .map(|w| w.trim_matches(['"', '\'', '(', ')']).to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// Resolve `.` and `..` textually. The path may not exist yet, so the
/// filesystem cannot be asked.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// System paths a command may mention without it meaning an escape: binaries,
/// devices and temporary files.
fn readable_system_path(path: &Path) -> bool {
    const ALLOWED: [&str; 8] = [
        "/usr",
        "/bin",
        "/sbin",
        "/lib",
        "/opt",
        "/etc",
        "/dev/null",
        "/proc",
    ];
    let text = path.to_string_lossy();
    ALLOWED.iter().any(|prefix| text.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn boundary() -> Boundary {
        Boundary::new("/home/u/worktrees/depot/1-cache")
    }

    fn bash(command: &str) -> Verdict {
        check(&boundary(), "Bash", &json!({ "command": command }))
    }

    fn write(path: &str) -> Verdict {
        check(&boundary(), "Write", &json!({ "file_path": path }))
    }

    #[test]
    fn ordinary_work_inside_the_worktree_is_allowed() {
        for command in [
            "cargo test --workspace",
            "ls src",
            "grep -r TODO .",
            "git status",
            "git add -A",
            "git commit -m '[backend] ajoute le cache'",
            "git diff main",
            "npm install",
            "./scripts/build.sh",
            "cat /usr/share/doc/README",
        ] {
            assert_eq!(bash(command), Verdict::Allow, "refusé à tort : {command}");
        }
        for path in [
            "src/main.rs",
            "./docs/plan.md",
            "/home/u/worktrees/depot/1-cache/src/lib.rs",
        ] {
            assert_eq!(write(path), Verdict::Allow, "refusé à tort : {path}");
        }
    }

    #[test]
    fn writing_outside_the_worktree_is_refused_with_the_path() {
        let v = write("/home/u/projets/depot/src/main.rs");
        assert!(v.is_denied());
        assert!(v.reason().unwrap().contains("sort du worktree"));

        // The v1 incident: touching the project's own checkout.
        assert!(write("/home/u/.bashrc").is_denied());
        assert!(write("../../ailleurs.txt").is_denied());
        assert!(write("/home/u/worktrees/depot/2-autre/x.rs").is_denied());
    }

    #[test]
    fn a_command_naming_a_path_outside_the_worktree_is_refused() {
        // The shape of the v1 accident, and the ways it could be spelled.
        for command in [
            "mv *.md /home/u/projets/depot/docs/",
            "cp secret.txt /home/u/ailleurs",
            "rm /home/u/projets/depot/README.md",
            "echo x > /home/u/.ssh/config",
            "cat ~/.aws/credentials",
        ] {
            let v = bash(command);
            assert!(v.is_denied(), "aurait dû être refusé : {command}");
            assert!(v.reason().unwrap().contains("hors du worktree"));
        }
    }

    #[test]
    fn git_commands_that_move_work_out_are_refused() {
        for sub in ["push", "checkout", "switch", "worktree", "remote", "clean"] {
            let v = bash(&format!("git {sub} quelque-chose"));
            assert!(v.is_denied(), "git {sub} aurait dû être refusé");
            assert!(v.reason().unwrap().contains(sub));
        }
        // Options before the subcommand do not hide it.
        assert!(bash("git -C . push origin main").is_denied());
        assert!(bash("cd src && git push").is_denied());
        // Committing and reading history stay allowed.
        assert_eq!(bash("git log --oneline -5"), Verdict::Allow);
        assert_eq!(bash("git commit -am wip"), Verdict::Allow);
    }

    #[test]
    fn catastrophic_commands_are_refused_wherever_they_point() {
        for command in [
            "rm -rf /",
            "rm  -rf  /",
            "sudo shutdown now",
            "mkfs.ext4 /dev/sda",
        ] {
            let v = bash(command);
            assert!(v.is_denied(), "aurait dû être refusé : {command}");
        }
    }

    #[test]
    fn reading_is_not_confined_only_writing_is() {
        // An agent must be able to read the wider codebase to understand it.
        assert_eq!(
            check(
                &boundary(),
                "Read",
                &json!({"file_path": "/home/u/projets/depot/src/main.rs"})
            ),
            Verdict::Allow
        );
        assert_eq!(
            check(
                &boundary(),
                "Grep",
                &json!({"pattern": "TODO", "path": "/home/u"})
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn extra_directories_widen_the_box() {
        let mut b = boundary();
        b.extra.push(PathBuf::from("/tmp/scratch"));
        assert_eq!(
            check(&b, "Write", &json!({"file_path": "/tmp/scratch/notes.md"})),
            Verdict::Allow
        );
        assert!(check(&b, "Write", &json!({"file_path": "/tmp/ailleurs"})).is_denied());
    }

    #[test]
    fn dot_dot_does_not_sneak_out() {
        assert!(write("/home/u/worktrees/depot/1-cache/../2-autre/x").is_denied());
        assert_eq!(
            write("/home/u/worktrees/depot/1-cache/src/../lib.rs"),
            Verdict::Allow
        );
    }

    #[test]
    fn a_sibling_directory_with_a_shared_prefix_is_not_inside() {
        // `/…/1-cache-bis` must not pass as `/…/1-cache`.
        assert!(write("/home/u/worktrees/depot/1-cache-bis/x.rs").is_denied());
    }

    #[test]
    fn quoted_paths_are_still_seen() {
        assert!(bash("cp x \"/home/u/ailleurs/y\"").is_denied());
        assert!(bash("cp x '/home/u/ailleurs/y'").is_denied());
    }

    #[test]
    fn an_empty_or_odd_call_does_not_panic() {
        assert_eq!(bash(""), Verdict::Allow);
        assert_eq!(bash("   "), Verdict::Allow);
        assert_eq!(check(&boundary(), "Bash", &json!({})), Verdict::Allow);
        assert_eq!(check(&boundary(), "Write", &json!({})), Verdict::Allow);
        assert_eq!(
            check(&boundary(), "Inconnu", &json!({"x": 1})),
            Verdict::Allow
        );
    }
}
