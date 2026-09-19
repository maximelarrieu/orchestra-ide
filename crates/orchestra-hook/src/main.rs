//! The guard Claude Code runs before every tool call of an Orchestra agent.
//!
//! It reads the hook payload on standard input, decides, and says so through
//! its exit code: 0 lets the call through, 2 with a reason on standard error
//! refuses it and shows the agent why.
//!
//! **The decision is local.** An earlier design had this binary ask the daemon
//! over a socket; that would put a round trip on the critical path of every
//! tool call and make an agent's safety depend on the daemon being healthy.
//! Everything needed is in the environment and the payload, so nothing is
//! asked of anyone. The rules themselves live in `orchestra_core::guard`, so
//! the daemon and this binary cannot disagree about them.
//!
//! Three properties matter more than features here: it starts immediately, it
//! never blocks, and an Orchestra problem never breaks the agent. Anything
//! unexpected therefore allows the call.

use std::io::Read;
use std::path::PathBuf;

use orchestra_core::guard::{check, Boundary, GitPolicy, Verdict};

/// Set by the daemon on every agent it spawns.
const WORKTREE_VAR: &str = "ORCHESTRA_WORKTREE";
/// Extra directories the agent may also write to, separated by `:`.
const EXTRA_VAR: &str = "ORCHESTRA_EXTRA_DIRS";
/// `full` for the one role whose job is git. Absent or unknown means confined.
const GIT_VAR: &str = "ORCHESTRA_GIT";

fn main() {
    // Whatever happens, a failure here must not fail the agent.
    let verdict = std::panic::catch_unwind(run).unwrap_or(Verdict::Allow);
    match verdict {
        Verdict::Allow => std::process::exit(0),
        Verdict::Deny(reason) => {
            eprintln!("{reason}");
            std::process::exit(2);
        }
    }
}

fn run() -> Verdict {
    // No worktree in the environment means this is not one of our agents.
    let Some(worktree) = std::env::var_os(WORKTREE_VAR).map(PathBuf::from) else {
        return Verdict::Allow;
    };
    if worktree.as_os_str().is_empty() {
        return Verdict::Allow;
    }

    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() {
        return Verdict::Allow;
    }
    let payload = payload.trim();
    if payload.is_empty() {
        return Verdict::Allow;
    }

    let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
        return Verdict::Allow;
    };

    // Only tool calls are guarded; other hook events are observations.
    let event = value
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if event != "PreToolUse" {
        return Verdict::Allow;
    }

    let tool = value
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let input = value
        .get("tool_input")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let mut boundary = Boundary::new(worktree);
    boundary.git = std::env::var(GIT_VAR)
        .map(|v| GitPolicy::parse(&v))
        .unwrap_or_default();
    if let Some(extra) = std::env::var_os(EXTRA_VAR) {
        boundary.extra = std::env::split_paths(&extra)
            .filter(|p| !p.as_os_str().is_empty())
            .collect();
    }

    check(&boundary, tool, &input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The decision as `run` makes it, without touching the process
    /// environment or standard input.
    fn decide(worktree: &str, payload: serde_json::Value) -> Verdict {
        let event = payload
            .get("hook_event_name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if event != "PreToolUse" {
            return Verdict::Allow;
        }
        let tool = payload
            .get("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let input = payload
            .get("tool_input")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        check(&Boundary::new(worktree), tool, &input)
    }

    /// A payload shaped exactly like the one Claude Code sends, captured from
    /// a real run.
    fn payload(tool: &str, input: serde_json::Value) -> serde_json::Value {
        json!({
            "session_id": "51d313a4-c9f5-45f5-a731-9a8aaeacdaad",
            "transcript_path": "/home/u/.claude/projects/-tmp/51d3.jsonl",
            "cwd": "/home/u/wt/1-cache",
            "permission_mode": "bypassPermissions",
            "hook_event_name": "PreToolUse",
            "tool_name": tool,
            "tool_input": input,
            "tool_use_id": "toolu_01NSnfK3fnkQ2xQi8wWuwZNr"
        })
    }

    #[test]
    fn a_real_payload_for_ordinary_work_is_allowed() {
        let v = decide(
            "/home/u/wt/1-cache",
            payload(
                "Bash",
                json!({"command": "cargo test", "description": "run the tests"}),
            ),
        );
        assert_eq!(v, Verdict::Allow);
    }

    #[test]
    fn a_command_leaving_the_worktree_is_refused_with_a_readable_reason() {
        let v = decide(
            "/home/u/wt/1-cache",
            payload("Bash", json!({"command": "mv *.md /home/u/projet/docs/"})),
        );
        let reason = v.reason().expect("refus attendu");
        assert!(reason.contains("hors du worktree"), "{reason}");
        assert!(
            reason.contains("/home/u/wt/1-cache"),
            "le périmètre est rappelé"
        );
    }

    #[test]
    fn hook_events_that_are_not_tool_calls_pass_through() {
        for event in ["Stop", "SessionStart", "PostToolUse", "Notification"] {
            let mut p = payload("Bash", json!({"command": "rm -rf /"}));
            p["hook_event_name"] = json!(event);
            assert_eq!(decide("/home/u/wt/1-cache", p), Verdict::Allow);
        }
    }

    #[test]
    fn a_malformed_payload_never_blocks_the_agent() {
        assert_eq!(decide("/home/u/wt/1", json!({})), Verdict::Allow);
        assert_eq!(decide("/home/u/wt/1", json!(null)), Verdict::Allow);
        assert_eq!(
            decide("/home/u/wt/1", json!({"hook_event_name": "PreToolUse"})),
            Verdict::Allow
        );
    }

    #[test]
    fn the_guard_agrees_with_the_core_rules() {
        // The binary must add no rule of its own: same input, same verdict.
        let boundary = Boundary::new("/home/u/wt/1-cache");
        let input = json!({"command": "git push origin main"});
        assert_eq!(
            decide("/home/u/wt/1-cache", payload("Bash", input.clone())),
            check(&boundary, "Bash", &input)
        );
    }
}
