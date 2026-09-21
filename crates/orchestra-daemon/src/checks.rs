//! Running the repository's own checks, in the ticket's worktree.
//!
//! The rules are in [`orchestra_core::checks`]; what is here is the process:
//! spawn it, hold it to a deadline, keep the end of what it said.
//!
//! No shell. The command is already split, so nothing in it can chain a second
//! one, and the only thing that runs is the program the configuration names.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use orchestra_core::checks::{self, Check, CheckRun, ChecksOutcome, RepoFacts};
use orchestra_core::events::{Event, EventKind};

/// Files worth reading at a repository's root to work out how it checks itself.
const LOOKED_FOR: [&str; 8] = [
    "justfile",
    ".justfile",
    "Makefile",
    "GNUmakefile",
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "pytest.ini",
];

/// Which commands this ticket's gate runs.
///
/// The configuration wins. Failing that, the *main* repository is asked — never
/// the worktree: the worktree is the branch, the branch is what the agents
/// wrote, and a branch that picks its own examiner is not an examination.
pub fn commands(cfg: &orchestra_core::config::ChecksConfig, repo: &Path) -> Vec<Check> {
    if !cfg.enabled {
        return Vec::new();
    }
    if !cfg.commands.is_empty() {
        return cfg
            .commands
            .iter()
            .filter_map(|c| Check::parse(c))
            .collect();
    }
    checks::detect(&facts(repo))
        .and_then(|line| Check::parse(&line))
        .into_iter()
        .collect()
}

/// Read what little of a repository's root says how it is verified.
fn facts(repo: &Path) -> RepoFacts {
    let mut files = Vec::new();
    for name in LOOKED_FOR {
        // Only the head of the file: a `Cargo.toml` is small, a lock-like
        // `package.json` need not be read whole to find its scripts.
        if let Ok(content) = std::fs::read_to_string(repo.join(name)) {
            let capped: String = content.chars().take(64_000).collect();
            files.push((name.to_string(), capped));
        }
    }
    RepoFacts { files }
}

/// The last pass of the gate, rebuilt from a ticket's `CheckFinished` events.
///
/// One pass is one `round`, so the last pass is every run carrying the highest
/// number. Rebuilt rather than stored: the events are the record, and a second
/// copy in a column is a second copy to keep in step.
pub fn last_pass(events: &[Event]) -> Option<ChecksOutcome> {
    let mut last: Option<ChecksOutcome> = None;
    for event in events {
        let EventKind::CheckFinished { round, run } = &event.kind else {
            continue;
        };
        match last.as_mut() {
            Some(outcome) if outcome.round == *round => outcome.runs.push((**run).clone()),
            Some(outcome) if outcome.round > *round => {}
            _ => {
                last = Some(ChecksOutcome {
                    round: *round,
                    runs: vec![(**run).clone()],
                })
            }
        }
    }
    last
}

/// Run one check and report what it did, whatever happened.
///
/// A command that cannot even start is a failed check, not an error to bubble
/// up: the ticket has to hear "this did not pass", and why, exactly as it
/// would for a non-zero exit.
pub async fn run(check: &Check, worktree: &Path, timeout: Duration) -> CheckRun {
    let started = Instant::now();
    let label = check.label();

    let spawned = tokio::process::Command::new(&check.program)
        .args(&check.args)
        .current_dir(worktree)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // The daemon going down must not leave a test suite running.
        .kill_on_drop(true)
        .spawn();

    let child = match spawned {
        Ok(child) => child,
        Err(e) => {
            return CheckRun {
                command: label,
                ok: false,
                code: None,
                duration_ms: started.elapsed().as_millis() as u64,
                tail: format!("« {} » n'a pas pu démarrer : {e}", check.program),
            };
        }
    };

    // Dropping the future drops the child, and `kill_on_drop` then kills it:
    // a suite that hangs costs the deadline, not the afternoon.
    let out = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            return CheckRun {
                command: label,
                ok: false,
                code: None,
                duration_ms: started.elapsed().as_millis() as u64,
                tail: format!("lecture de la sortie impossible : {e}"),
            };
        }
        Err(_) => {
            return CheckRun {
                command: label,
                ok: false,
                code: None,
                duration_ms: started.elapsed().as_millis() as u64,
                tail: format!(
                    "arrêté après {} s sans avoir rendu la main",
                    timeout.as_secs()
                ),
            };
        }
    };

    let ok = out.status.success();
    // Both streams, in that order: a compiler writes its errors on stderr and
    // a test runner its failures on stdout, and we do not know which we have.
    let tail = if ok {
        String::new()
    } else {
        let mut merged = String::from_utf8_lossy(&out.stdout).into_owned();
        merged.push_str(&String::from_utf8_lossy(&out.stderr));
        crate::worker::translate::redact(&checks::tail(&merged, checks::TAIL_LINES))
    };
    CheckRun {
        command: label,
        ok,
        code: out.status.code(),
        duration_ms: started.elapsed().as_millis() as u64,
        tail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::config::ChecksConfig;

    fn temp() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("orchestra-checks-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn a_command_that_passes_keeps_no_output() {
        let dir = temp();
        let check = Check::parse("true").unwrap();
        let run = run(&check, &dir, Duration::from_secs(10)).await;
        assert!(run.ok);
        assert_eq!(run.code, Some(0));
        assert!(run.tail.is_empty(), "un vert n'a rien à raconter");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_command_that_fails_keeps_the_end_of_what_it_said() {
        let dir = temp();
        let check = Check::parse("sh -c 'echo cassé >&2; exit 3'").unwrap();
        let run = run(&check, &dir, Duration::from_secs(10)).await;
        assert!(!run.ok);
        assert_eq!(run.code, Some(3));
        assert!(run.tail.contains("cassé"), "{}", run.tail);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_command_that_hangs_is_cut_short() {
        let dir = temp();
        let check = Check::parse("sleep 30").unwrap();
        let run = run(&check, &dir, Duration::from_millis(200)).await;
        assert!(!run.ok);
        assert_eq!(run.code, None, "tué, donc sans code");
        assert!(run.tail.contains("rendu la main"), "{}", run.tail);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_command_that_does_not_exist_is_a_failed_check() {
        let dir = temp();
        let check = Check::parse("orchestra-commande-qui-nexiste-pas").unwrap();
        let run = run(&check, &dir, Duration::from_secs(10)).await;
        assert!(!run.ok);
        assert!(run.tail.contains("n'a pas pu démarrer"), "{}", run.tail);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_configuration_wins_over_the_guess() {
        let dir = temp();
        std::fs::write(dir.join("Cargo.toml"), "[workspace]").unwrap();
        let cfg = ChecksConfig {
            commands: vec!["just verifier".into()],
            ..Default::default()
        };
        assert_eq!(commands(&cfg, &dir)[0].label(), "just verifier");

        let guessed = commands(&ChecksConfig::default(), &dir);
        assert_eq!(guessed[0].label(), "cargo test --workspace");

        let off = ChecksConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(commands(&off, &dir).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    fn finished(round: u32, command: &str, ok: bool) -> Event {
        Event::from_new(
            1,
            orchestra_core::events::NewEvent::new(EventKind::CheckFinished {
                round,
                run: Box::new(CheckRun {
                    command: command.into(),
                    ok,
                    code: Some(if ok { 0 } else { 1 }),
                    duration_ms: 10,
                    tail: String::new(),
                }),
            }),
        )
    }

    #[test]
    fn only_the_last_pass_counts() {
        // A first pass that failed and a second that passed: what decides is
        // the state the branch is in now, not the one it came from.
        let events = vec![
            finished(1, "cargo test", false),
            finished(2, "cargo build", true),
            finished(2, "cargo test", true),
        ];
        let last = last_pass(&events).unwrap();
        assert_eq!(last.round, 2);
        assert_eq!(last.runs.len(), 2);
        assert!(last.passed());

        assert!(last_pass(&[]).is_none(), "aucune passe, aucun verdict");
    }

    #[test]
    fn a_repository_that_says_nothing_gets_no_gate() {
        let dir = temp();
        assert!(commands(&ChecksConfig::default(), &dir).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
