//! The repository's own checks — the ones that are not a matter of opinion.
//!
//! The relecture is an agent: it reads, it judges, and what it says about the
//! tests is a report. A report can be wrong, and nothing here can tell an
//! honest one from a hurried one. An exit code can. So the daemon runs the
//! project's own verification itself, in the ticket's worktree, and that
//! result — not a sentence in a handoff — is what opens the integration.
//!
//! This module holds no I/O, like the rest of `orchestra-core`: it says which
//! command to run and how to read what came back. The daemon reads the files
//! and spawns the processes.
//!
//! **Where the command comes from matters.** It is taken from the daemon's
//! configuration, or failing that from the *main* repository — never from the
//! ticket's branch, which agents write to. What it then runs is of course the
//! branch's own code: tests are code, and an agent that writes a test writes
//! something that will execute. That is already true today — the reviewer runs
//! `cargo test` through its shell — so the gate adds no exposure. What it
//! removes is the possibility of claiming a green suite without one.

use serde::{Deserialize, Serialize};

/// How many lines of a failed command are kept, and how wide each one may be.
///
/// A compiler is happy to print a thousand lines; what says why it failed is
/// at the end, and what an agent is asked to repair has to fit in a prompt.
pub const TAIL_LINES: usize = 40;
const TAIL_COLUMNS: usize = 300;

/// A verification command, already split so it can run without a shell.
///
/// No `sh -c`: a shell would turn the configuration into a place where `;` and
/// `$(…)` mean something, for no gain — these are program names and flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub program: String,
    pub args: Vec<String>,
}

impl Check {
    /// Split a command line on whitespace, honouring single and double quotes.
    ///
    /// `None` when there is nothing to run, so an empty line in the
    /// configuration is ignored rather than becoming a command named "".
    pub fn parse(line: &str) -> Option<Check> {
        let mut words: Vec<String> = Vec::new();
        let mut word = String::new();
        let mut quote: Option<char> = None;
        let mut started = false;
        for c in line.chars() {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), c) => word.push(c),
                (None, '\'') | (None, '"') => {
                    quote = Some(c);
                    // `""` is an argument, even an empty one.
                    started = true;
                }
                (None, c) if c.is_whitespace() => {
                    if !word.is_empty() || started {
                        words.push(std::mem::take(&mut word));
                        started = false;
                    }
                }
                (None, c) => word.push(c),
            }
        }
        if !word.is_empty() || started {
            words.push(word);
        }
        let mut words = words.into_iter();
        let program = words.next()?;
        if program.is_empty() {
            return None;
        }
        Some(Check {
            program,
            args: words.collect(),
        })
    }

    /// The command as the user wrote it, for the event and the screen.
    pub fn label(&self) -> String {
        let mut out = self.program.clone();
        for arg in &self.args {
            out.push(' ');
            out.push_str(arg);
        }
        out
    }
}

/// What one command gave back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckRun {
    pub command: String,
    pub ok: bool,
    /// `None` when the process was killed — a timeout, or a signal.
    #[serde(default)]
    pub code: Option<i32>,
    pub duration_ms: u64,
    /// The last lines of its output, redacted and capped. Empty when it passed:
    /// nobody reads the output of a green run, and storing it would put every
    /// test suite in the event log.
    #[serde(default)]
    pub tail: String,
}

impl CheckRun {
    /// `cargo test — code 101 en 12 s`, for one line of activity.
    pub fn label_fr(&self) -> String {
        let seconds = self.duration_ms / 1000;
        if self.ok {
            return format!("{} passe en {} s", self.command, seconds);
        }
        match self.code {
            Some(code) => format!("{} échoue (code {code}) en {} s", self.command, seconds),
            None => format!("{} ne rend pas la main ({} s)", self.command, seconds),
        }
    }
}

/// What a whole pass of the gate concluded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChecksOutcome {
    /// 1 for the first pass, 2 after one repair round, …
    pub round: u32,
    pub runs: Vec<CheckRun>,
}

impl ChecksOutcome {
    /// True when every command ran and came back zero.
    ///
    /// An empty pass passes: a project that declares no check is not a project
    /// that failed one.
    pub fn passed(&self) -> bool {
        self.runs.iter().all(|r| r.ok)
    }

    /// The first command that refused, which is the one to repair.
    pub fn failed(&self) -> Option<&CheckRun> {
        self.runs.iter().find(|r| !r.ok)
    }

    /// What the roles sent back to work are told, one line each.
    pub fn blocking_lines(&self) -> Vec<String> {
        let Some(run) = self.failed() else {
            return Vec::new();
        };
        let mut lines = vec![format!("`{}` échoue.", run.command)];
        lines.extend(run.tail.lines().map(|l| l.to_string()));
        lines
    }
}

/// Keep the end of an output, capped in lines and in width.
pub fn tail(output: &str, lines: usize) -> String {
    let all: Vec<&str> = output.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..]
        .iter()
        .map(|line| {
            if line.chars().count() <= TAIL_COLUMNS {
                (*line).to_string()
            } else {
                let cut: String = line.chars().take(TAIL_COLUMNS).collect();
                format!("{cut}…")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What was read at the root of a repository, to work out how it checks itself.
///
/// The daemon fills this in; the guessing stays here, where it is testable
/// without a disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoFacts {
    /// Contents of `Cargo.toml`, `package.json`, `justfile`, `Makefile`,
    /// `pyproject.toml`… whichever were there, keyed by file name.
    pub files: Vec<(String, String)>,
}

impl RepoFacts {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.files
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, c)| c.as_str())
    }

    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
}

/// The command a repository gives itself, guessed from its root.
///
/// One command, not a battery: the point is the entry point the project
/// already uses, the one a contributor would type. Anything more belongs in
/// `checks.commands`, where the user writes it out and nobody has to guess.
///
/// The order is deliberate. A `justfile` or a `Makefile` is a choice someone
/// made about how this repository is run; a `Cargo.toml` is only what it is
/// written in. So a declared recipe wins over the language's default.
pub fn detect(facts: &RepoFacts) -> Option<String> {
    if facts
        .get("justfile")
        .or_else(|| facts.get(".justfile"))
        .is_some_and(|c| has_recipe(c, "test"))
    {
        return Some("just test".into());
    }
    if facts
        .get("Makefile")
        .or_else(|| facts.get("GNUmakefile"))
        .is_some_and(|c| has_recipe(c, "test"))
    {
        return Some("make test".into());
    }
    if let Some(cargo) = facts.get("Cargo.toml") {
        return Some(if cargo.contains("[workspace]") {
            "cargo test --workspace".into()
        } else {
            "cargo test".into()
        });
    }
    if facts.get("package.json").is_some_and(has_npm_test) {
        return Some("npm test".into());
    }
    if facts.has("pytest.ini") || facts.has("tox.ini") {
        return Some("pytest".into());
    }
    if facts
        .get("pyproject.toml")
        .is_some_and(|c| c.contains("pytest"))
    {
        return Some("pytest".into());
    }
    None
}

/// The quality gates a repository already gives itself, then its tests:
/// what a pass runs when nothing is configured.
///
/// Only what the project and the machine both have. `cargo fmt` runs when
/// `cargo-fmt` is installed, `npm run lint` when the script exists, `ruff`
/// when the project uses it and the tool is there: a gate that cannot run
/// would fail every ticket for a reason no agent can fix. Cheap and strict
/// first, the suite last, so a pass stops on a formatting error in seconds
/// rather than after the tests.
pub fn detect_gates(facts: &RepoFacts, installed: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    // A declared recipe is the project's own choice, as for the tests.
    let recipes = facts
        .get("justfile")
        .or_else(|| facts.get(".justfile"))
        .map(|c| ("just", c))
        .or_else(|| facts.get("Makefile").or_else(|| facts.get("GNUmakefile")).map(|c| ("make", c)));
    if let Some((runner, content)) = recipes {
        for recipe in ["fmt-check", "lint", "check"] {
            if has_recipe(content, recipe) {
                out.push(format!("{runner} {recipe}"));
            }
        }
    }
    if out.is_empty() {
        if let Some(cargo) = facts.get("Cargo.toml") {
            let workspace = cargo.contains("[workspace]");
            if installed("cargo-fmt") {
                out.push("cargo fmt --all --check".into());
            }
            if installed("cargo-clippy") {
                out.push(if workspace {
                    "cargo clippy --workspace --all-targets -- -D warnings".into()
                } else {
                    "cargo clippy --all-targets -- -D warnings".into()
                });
            }
        } else if let Some(package) = facts.get("package.json") {
            for script in ["lint", "typecheck"] {
                if has_npm_script(package, script) {
                    out.push(format!("npm run {script}"));
                }
            }
        } else if facts.get("pyproject.toml").is_some_and(|c| c.contains("ruff")) && installed("ruff") {
            out.push("ruff check .".into());
        }
    }
    out.extend(detect(facts));
    out
}

/// A script of that name in `package.json`'s `scripts`.
fn has_npm_script(package: &str, name: &str) -> bool {
    package
        .split("\"scripts\"")
        .nth(1)
        .is_some_and(|scripts| scripts.contains(&format!("\"{name}\"")))
}

/// `.orchestra/checks.toml`, versioned with the repository: its gates travel
/// with the code, and a change to them is reviewed like code.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RepoChecks {
    /// The gate before every relecture.
    pub commands: Vec<String>,
    /// Quick commands after each implementation step.
    pub after_stage: Vec<String>,
}

impl RepoChecks {
    pub fn parse(src: &str) -> crate::error::Result<Self> {
        toml::from_str(src).map_err(|e| crate::error::CoreError::Config(format!(".orchestra/checks.toml : {e}")))
    }
}

/// A `test:` target at the start of a line, comments left out.
fn has_recipe(content: &str, name: &str) -> bool {
    content.lines().any(|line| {
        let line = line.trim_end();
        !line.starts_with('#')
            && line.starts_with(name)
            && line[name.len()..].trim_start().starts_with(':')
    })
}

/// `npm init` writes a `test` script that only prints an error. Running it
/// would fail every ticket of a project that simply never wrote a test.
fn has_npm_test(package: &str) -> bool {
    let Some(scripts) = package.split("\"scripts\"").nth(1) else {
        return false;
    };
    let Some(after) = scripts.split("\"test\"").nth(1) else {
        return false;
    };
    let mut chars = after
        .trim_start()
        .trim_start_matches(':')
        .trim_start()
        .chars();
    if chars.next() != Some('"') {
        return false;
    }
    // The value is JSON, so an escaped quote inside it is part of the script —
    // and the placeholder npm writes is precisely a string full of them.
    let mut value = String::new();
    let mut escaped = false;
    for c in chars {
        match (escaped, c) {
            (true, c) => {
                value.push(c);
                escaped = false;
            }
            (false, '\\') => escaped = true,
            (false, '"') => break,
            (false, c) => value.push(c),
        }
    }
    !value.trim().is_empty() && !value.contains("no test specified")
}

#[cfg(test)]
mod gates_tests {
    use super::*;

    fn facts(files: &[(&str, &str)]) -> RepoFacts {
        RepoFacts { files: files.iter().map(|(n, c)| (n.to_string(), c.to_string())).collect() }
    }

    #[test]
    fn a_rust_workspace_gets_fmt_and_clippy_before_its_tests_when_installed() {
        let f = facts(&[("Cargo.toml", "[workspace]\nmembers = []")]);
        assert_eq!(
            detect_gates(&f, |_| true),
            vec![
                "cargo fmt --all --check",
                "cargo clippy --workspace --all-targets -- -D warnings",
                "cargo test --workspace",
            ]
        );
        // Without the tools, only what can run.
        assert_eq!(detect_gates(&f, |_| false), vec!["cargo test --workspace"]);
    }

    #[test]
    fn a_node_project_gets_its_own_lint_and_typecheck() {
        let f = facts(&[("package.json", r#"{"scripts": {"lint": "eslint .", "typecheck": "tsc", "test": "vitest"}}"#)]);
        assert_eq!(detect_gates(&f, |_| true), vec!["npm run lint", "npm run typecheck", "npm test"]);
    }

    #[test]
    fn a_declared_recipe_wins_over_the_language() {
        let f = facts(&[("justfile", "lint:\n  cargo clippy\ntest:\n  cargo test\n"), ("Cargo.toml", "")]);
        assert_eq!(detect_gates(&f, |_| true), vec!["just lint", "just test"]);
    }

    #[test]
    fn the_repository_file_is_read_strictly() {
        let c = RepoChecks::parse("commands = [\"cargo test\"]\nafter_stage = [\"cargo check\"]\n").unwrap();
        assert_eq!(c.after_stage, vec!["cargo check"]);
        assert!(RepoChecks::parse("comands = []").is_err(), "une faute de frappe ne passe pas en silence");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_split_without_a_shell() {
        let check = Check::parse("cargo clippy --workspace -- -D warnings").unwrap();
        assert_eq!(check.program, "cargo");
        assert_eq!(
            check.args,
            vec!["clippy", "--workspace", "--", "-D", "warnings"]
        );
        assert_eq!(check.label(), "cargo clippy --workspace -- -D warnings");
    }

    #[test]
    fn quotes_hold_an_argument_together() {
        let check = Check::parse(r#"pytest -k "not slow" -q"#).unwrap();
        assert_eq!(check.args, vec!["-k", "not slow", "-q"]);
    }

    #[test]
    fn nothing_to_run_is_not_a_command() {
        assert_eq!(Check::parse("   "), None);
        assert_eq!(Check::parse(""), None);
    }

    #[test]
    fn a_shell_metacharacter_stays_an_argument() {
        // Nothing here interprets it, which is the whole point of not going
        // through a shell: it would be passed to the program, not run.
        let check = Check::parse("cargo test; rm -rf /").unwrap();
        assert_eq!(check.program, "cargo");
        assert_eq!(check.args, vec!["test;", "rm", "-rf", "/"]);
    }

    #[test]
    fn an_empty_gate_passes_and_a_failed_one_names_its_command() {
        let green = ChecksOutcome {
            round: 1,
            runs: vec![],
        };
        assert!(green.passed());
        assert!(green.failed().is_none());

        let red = ChecksOutcome {
            round: 1,
            runs: vec![
                CheckRun {
                    command: "cargo build".into(),
                    ok: true,
                    code: Some(0),
                    duration_ms: 1000,
                    tail: String::new(),
                },
                CheckRun {
                    command: "cargo test".into(),
                    ok: false,
                    code: Some(101),
                    duration_ms: 2000,
                    tail: "error[E0308]: mismatched types".into(),
                },
            ],
        };
        assert!(!red.passed());
        assert_eq!(red.failed().unwrap().command, "cargo test");
        let lines = red.blocking_lines();
        assert_eq!(lines[0], "`cargo test` échoue.");
        assert!(lines[1].contains("E0308"));
    }

    #[test]
    fn a_run_reads_in_words() {
        let run = CheckRun {
            command: "cargo test".into(),
            ok: false,
            code: None,
            duration_ms: 900_000,
            tail: String::new(),
        };
        assert!(run.label_fr().contains("ne rend pas la main"));
    }

    #[test]
    fn only_the_end_of_an_output_is_kept() {
        let output = (0..200)
            .map(|i| format!("ligne {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let kept = tail(&output, TAIL_LINES);
        assert_eq!(kept.lines().count(), TAIL_LINES);
        assert!(kept.starts_with("ligne 160"));
        assert!(kept.ends_with("ligne 199"));

        // A single enormous line cannot blow up an event either.
        let wide = "x".repeat(10_000);
        assert!(tail(&wide, TAIL_LINES).chars().count() <= TAIL_COLUMNS + 1);
    }

    fn facts(files: &[(&str, &str)]) -> RepoFacts {
        RepoFacts {
            files: files
                .iter()
                .map(|(n, c)| (n.to_string(), c.to_string()))
                .collect(),
        }
    }

    #[test]
    fn a_repository_is_asked_how_it_checks_itself() {
        assert_eq!(
            detect(&facts(&[("Cargo.toml", "[workspace]\nmembers = []")])),
            Some("cargo test --workspace".into())
        );
        assert_eq!(
            detect(&facts(&[("Cargo.toml", "[package]\nname = \"x\"")])),
            Some("cargo test".into())
        );
        assert_eq!(
            detect(&facts(&[(
                "package.json",
                r#"{"scripts": {"test": "vitest run"}}"#
            )])),
            Some("npm test".into())
        );
        assert_eq!(detect(&facts(&[("README.md", "")])), None);
    }

    #[test]
    fn a_recipe_the_repository_wrote_wins_over_the_language_default() {
        let both = facts(&[
            ("Cargo.toml", "[workspace]"),
            (
                "justfile",
                "build:\n    cargo build\n\ntest:\n    cargo test",
            ),
        ]);
        assert_eq!(detect(&both), Some("just test".into()));
    }

    #[test]
    fn a_placeholder_test_script_is_not_a_check() {
        // `npm init` writes this one; running it fails every ticket of a
        // project that simply has no test yet.
        let empty = facts(&[(
            "package.json",
            r#"{"scripts": {"test": "echo \"Error: no test specified\" && exit 1"}}"#,
        )]);
        assert_eq!(detect(&empty), None);
    }

    #[test]
    fn a_commented_target_is_not_a_recipe() {
        let makefile = facts(&[("Makefile", "# test: ce qu'on ferait\nbuild:\n\tcc x.c")]);
        assert_eq!(detect(&makefile), None);
    }
}
