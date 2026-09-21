//! Panes: a window onto each agent, in the multiplexer the user already runs.
//!
//! Everything here is best-effort and nothing here can fail a ticket. A pane
//! is a convenience; the work happens whether or not a window shows it, and a
//! zellij that is absent, older, or simply not listening must cost a log line
//! and nothing else.
//!
//! Every call therefore has a deadline. This is not theoretical: a `zellij
//! action` whose client cannot reach its server does not fail, it *waits* —
//! which, without a deadline, would hang the task walking a ticket's stages.
//!
//! What the CLI gives us (zellij 0.45, `zellij action --help`):
//!
//! - `new-pane` prints the new pane's id on stdout, `terminal_<n>`. That is the
//!   whole reason panes can be addressed later at all.
//! - `focus-pane-id <id>`, `rename-pane --pane-id <id> <name>` and
//!   `close-pane --pane-id <id>` take that id.
//!
//! Facts about the command line live in `docs/ZELLIJ_NOTES.md`.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

/// How long any one zellij call is given before it is abandoned.
///
/// These are local IPC calls that answer in milliseconds; three seconds is a
/// deadline nothing healthy ever reaches.
const DEADLINE: Duration = Duration::from_secs(3);

/// True when this process lives inside a zellij session.
///
/// Zellij exports `ZELLIJ` in every pane it starts. Asking the environment
/// rather than trying a call is what makes the whole module free for someone
/// who does not use zellij: no process spawned, nothing to wait for.
pub fn inside() -> bool {
    std::env::var_os("ZELLIJ").is_some()
}

/// Open a pane running `command`, and return the id zellij gave it.
///
/// `None` when there is no zellij, when it refused, or when it did not answer
/// in time — the caller carries on either way.
pub async fn new_pane(name: &str, cwd: &Path, command: &[String]) -> Option<String> {
    if !inside() || command.is_empty() {
        return None;
    }
    let mut args: Vec<String> = vec![
        "action".into(),
        "new-pane".into(),
        "--name".into(),
        name.into(),
        "--cwd".into(),
        cwd.display().to_string(),
        "--".into(),
    ];
    args.extend(command.iter().cloned());
    let out = call(&args).await?;
    pane_id(&out)
}

/// Bring a pane to the front. False when it could not be done.
pub async fn focus(pane_id: &str) -> bool {
    valid_id(pane_id)
        && call(&["action".into(), "focus-pane-id".into(), pane_id.to_string()])
            .await
            .is_some()
}

/// Rename a pane, which is how a finished agent stops looking like a running
/// one from across the screen.
pub async fn rename(pane_id: &str, name: &str) -> bool {
    valid_id(pane_id)
        && call(&[
            "action".into(),
            "rename-pane".into(),
            "--pane-id".into(),
            pane_id.to_string(),
            name.to_string(),
        ])
        .await
        .is_some()
}

/// Close a pane. Used when a ticket is cleaned up, never on its own.
pub async fn close(pane_id: &str) -> bool {
    valid_id(pane_id)
        && call(&[
            "action".into(),
            "close-pane".into(),
            "--pane-id".into(),
            pane_id.to_string(),
        ])
        .await
        .is_some()
}

/// Run one zellij call under a deadline, and give back its standard output.
async fn call(args: &[String]) -> Option<String> {
    let mut command = tokio::process::Command::new("zellij");
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Abandoning the call must also stop the client, or a hung one would
        // outlive the daemon that asked for it.
        .kill_on_drop(true);

    let child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            tracing::debug!("zellij introuvable : {e}");
            return None;
        }
    };
    match tokio::time::timeout(DEADLINE, child.wait_with_output()).await {
        Ok(Ok(out)) if out.status.success() => {
            Some(String::from_utf8_lossy(&out.stdout).into_owned())
        }
        Ok(Ok(out)) => {
            tracing::debug!(
                "zellij {:?} a refusé : {}",
                args.first(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
            None
        }
        Ok(Err(e)) => {
            tracing::debug!("zellij illisible : {e}");
            None
        }
        Err(_) => {
            tracing::warn!("zellij n'a pas répondu en {:?} : pane abandonné", DEADLINE);
            None
        }
    }
}

/// The pane id inside whatever `new-pane` printed.
///
/// Parsed rather than trusted: the day a version prints a banner first, an
/// unparsable line must mean "no pane to address", not a pane id made of
/// someone's release notes.
fn pane_id(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| valid_id(line))
        .map(str::to_string)
}

/// `terminal_12`, `plugin_3`: the two shapes zellij hands out.
fn valid_id(id: &str) -> bool {
    let Some((kind, number)) = id.split_once('_') else {
        return false;
    };
    matches!(kind, "terminal" | "plugin")
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

/// The name a pane carries while its agent works.
pub fn pane_name(project: &str, role: &str, ticket: i64) -> String {
    format!("[{project}] {role} #{ticket}")
}

/// The same name once the agent is done, prefixed by how it went.
///
/// A symbol, never a colour on its own: a wall of panes is read from a metre
/// away, and that is exactly where hue stops being legible.
pub fn finished_name(base: &str, status: orchestra_core::model::AgentStatus) -> String {
    let mark = match status {
        orchestra_core::model::AgentStatus::Done => "✓",
        orchestra_core::model::AgentStatus::Cancelled => "⊘",
        orchestra_core::model::AgentStatus::Manual => "☰",
        _ => "✗",
    };
    format!("{mark} {base}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::model::AgentStatus;

    #[test]
    fn the_pane_id_is_read_out_of_what_zellij_printed() {
        assert_eq!(pane_id("terminal_12\n"), Some("terminal_12".into()));
        assert_eq!(pane_id("  plugin_3  \n"), Some("plugin_3".into()));
        // A version that says something else first must not cost us the id.
        assert_eq!(
            pane_id("note: something\nterminal_7\n"),
            Some("terminal_7".into())
        );
    }

    #[test]
    fn anything_that_is_not_an_id_is_not_taken_for_one() {
        assert_eq!(pane_id(""), None);
        assert_eq!(pane_id("ok\n"), None);
        assert_eq!(pane_id("terminal_\n"), None);
        assert_eq!(pane_id("terminal_abc\n"), None);
        assert_eq!(pane_id("window_3\n"), None);
    }

    #[tokio::test]
    async fn nothing_is_attempted_outside_zellij() {
        // The guard is the environment, so a machine without zellij never
        // spawns a process — and these calls answer instantly.
        if inside() {
            return;
        }
        assert_eq!(
            new_pane("x", Path::new("/tmp"), &["true".to_string()]).await,
            None
        );
        assert!(!focus("terminal_1").await);
    }

    #[tokio::test]
    async fn an_id_that_is_not_one_is_never_sent_to_zellij() {
        assert!(!focus("; rm -rf /").await);
        assert!(!rename("terminal_abc", "x").await);
        assert!(!close("").await);
    }

    #[test]
    fn a_pane_says_which_ticket_it_belongs_to() {
        assert_eq!(
            pane_name("orchestra", "backend", 12),
            "[orchestra] backend #12"
        );
        assert_eq!(
            finished_name("[orchestra] backend #12", AgentStatus::Done),
            "✓ [orchestra] backend #12"
        );
        assert!(finished_name("x", AgentStatus::Failed).starts_with('✗'));
        assert!(finished_name("x", AgentStatus::Cancelled).starts_with('⊘'));
    }
}
