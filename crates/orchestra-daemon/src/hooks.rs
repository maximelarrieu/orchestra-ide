//! The per-session settings handed to an agent.
//!
//! Hooks are passed with `--settings`, never written to the user's own
//! `~/.claude/settings.json`: Orchestra must not change how the user's normal
//! Claude Code sessions behave.

use std::path::Path;

use serde_json::{json, Value};

/// Name of the guard binary. Installed next to `orchestra`, so the same
/// version ships with the daemon.
pub const HOOK_BIN: &str = "orchestra-hook";

/// Build the `--settings` payload for an agent.
///
/// Only `PreToolUse` is wired: it is the one that can refuse. Observing the
/// rest would duplicate what the output stream already says.
pub fn settings_json(hook_bin: &str) -> String {
    json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": "Bash|Edit|Write|MultiEdit|NotebookEdit",
                "hooks": [{
                    "type": "command",
                    "command": hook_bin,
                    "timeout": 5
                }]
            }]
        }
    })
    .to_string()
}

/// Where the guard lives: next to the running binary, else on the PATH.
pub fn hook_binary() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(HOOK_BIN)))
        .filter(|path| path.exists())
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| HOOK_BIN.to_string())
}

/// Environment an agent runs with.
///
/// The guard reads the worktree from here rather than being told by the
/// daemon, so it needs nothing at the moment it decides.
pub fn agent_env(
    worktree: &Path,
    agent_id: &str,
    ticket_id: &str,
    extra_dirs: &[String],
) -> Vec<(String, String)> {
    let mut env = vec![
        (
            "ORCHESTRA_WORKTREE".to_string(),
            worktree.to_string_lossy().to_string(),
        ),
        ("ORCHESTRA_AGENT_ID".to_string(), agent_id.to_string()),
        ("ORCHESTRA_TICKET_ID".to_string(), ticket_id.to_string()),
    ];
    if !extra_dirs.is_empty() {
        env.push(("ORCHESTRA_EXTRA_DIRS".to_string(), extra_dirs.join(":")));
    }
    env
}

/// Recognise a refusal in the text of a failed tool result.
///
/// Claude Code reports it as `PreToolUse:Bash hook error: [<cmd>]: <reason>`,
/// so the refusal is visible in the stream without asking for hook events.
pub fn blocked_reason(tool_result_text: &str) -> Option<String> {
    let marker = "hook error:";
    if !tool_result_text.contains("PreToolUse") || !tool_result_text.contains(marker) {
        return None;
    }
    let after = tool_result_text.split_once(marker)?.1;
    // The command is echoed in brackets before the reason.
    let reason = match after.split_once("]:") {
        Some((_, r)) => r,
        None => after,
    };
    Some(reason.trim().to_string())
}

/// Tools an agent may never use, whatever its role asks for.
pub fn always_disallowed() -> Vec<String> {
    vec![
        // The guard already refuses these, but saying so up front saves the
        // agent a wasted turn discovering it.
        "Bash(git push:*)".to_string(),
        "Bash(git checkout:*)".to_string(),
        "Bash(git switch:*)".to_string(),
    ]
}

/// Parse a settings payload back, for tests and for `orchestra doctor`.
pub fn parse(settings: &str) -> Option<Value> {
    serde_json::from_str(settings).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn the_settings_wire_the_guard_to_the_writing_tools() {
        let settings = settings_json("/usr/local/bin/orchestra-hook");
        let v = parse(&settings).unwrap();
        let entry = &v["hooks"]["PreToolUse"][0];
        let matcher = entry["matcher"].as_str().unwrap();
        for tool in ["Bash", "Edit", "Write", "MultiEdit"] {
            assert!(matcher.contains(tool), "{tool} doit être surveillé");
        }
        assert_eq!(
            entry["hooks"][0]["command"],
            "/usr/local/bin/orchestra-hook"
        );
        assert_eq!(entry["hooks"][0]["type"], "command");
        assert!(entry["hooks"][0]["timeout"].as_u64().unwrap() <= 10);
        // One line, since it travels as a command-line argument.
        assert!(!settings.contains('\n'));
    }

    #[test]
    fn the_environment_tells_the_guard_its_boundary() {
        let env = agent_env(
            &PathBuf::from("/home/u/wt/1-cache"),
            "agent-1",
            "ticket-1",
            &[],
        );
        let get = |k: &str| {
            env.iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.as_str())
                .unwrap_or_default()
        };
        assert_eq!(get("ORCHESTRA_WORKTREE"), "/home/u/wt/1-cache");
        assert_eq!(get("ORCHESTRA_AGENT_ID"), "agent-1");
        assert!(!env.iter().any(|(n, _)| n == "ORCHESTRA_EXTRA_DIRS"));

        let with_extra = agent_env(
            &PathBuf::from("/w"),
            "a",
            "t",
            &["/tmp/scratch".into(), "/tmp/autre".into()],
        );
        assert!(with_extra
            .iter()
            .any(|(n, v)| n == "ORCHESTRA_EXTRA_DIRS" && v == "/tmp/scratch:/tmp/autre"));
    }

    #[test]
    fn a_refusal_is_recognised_in_the_stream() {
        // Captured from a real run.
        let text = "PreToolUse:Bash hook error: [/usr/local/bin/orchestra-hook]: \
                    « /home/u/ailleurs » est hors du worktree du ticket.";
        let reason = blocked_reason(text).expect("refus attendu");
        assert!(reason.starts_with("« /home/u/ailleurs »"));
        assert!(!reason.contains("hook error"));
    }

    #[test]
    fn an_ordinary_tool_failure_is_not_a_refusal() {
        for text in [
            "error: could not compile `orchestra-core`",
            "cat: fichier.txt: No such file or directory",
            "",
        ] {
            assert_eq!(blocked_reason(text), None, "faux positif sur : {text}");
        }
    }

    #[test]
    fn the_guard_is_looked_for_next_to_the_binary() {
        // Falls back to the bare name rather than failing when absent.
        let found = hook_binary();
        assert!(found.ends_with(HOOK_BIN), "{found}");
    }
}
