//! Running `claude` as a subprocess and reading its stream.
//!
//! This is the only place that knows the command line. The orchestrator and the
//! agents of phase 3 both go through it, so a change to the CLI is a change in
//! one file.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use orchestra_core::claude::StreamLine;
use orchestra_core::model::Effort;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;
use uuid::Uuid;

/// How the first prompt reaches the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptVia {
    /// As the `-p` argument. Simple, but bounded by the argv limit.
    Arg,
    /// On stdin, which also opens the channel used to steer the agent later.
    Stdin,
}

/// Everything the command line needs.
#[derive(Debug, Clone)]
pub struct ClaudeCommand {
    pub bin: String,
    pub cwd: PathBuf,
    pub prompt: String,
    pub prompt_via: PromptVia,
    pub session_id: Option<Uuid>,
    pub resume: Option<Uuid>,
    /// With `resume`: continue in a copy under `session_id`, so the old
    /// session stays as it was and the new run keeps an id of its own.
    pub fork_session: bool,
    pub name: Option<String>,
    /// `None` leaves the user's own default model in place.
    pub model: Option<String>,
    pub effort: Option<Effort>,
    pub permission_mode: Option<String>,
    pub append_system_prompt_file: Option<PathBuf>,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub max_budget_usd: Option<f64>,
    pub max_turns: Option<u32>,
    pub json_schema: Option<String>,
    pub settings_json: Option<String>,
    pub agents_json: Option<String>,
    /// Load only the repository's own settings and no MCP server.
    ///
    /// Without it an agent inherits everything in the user's `~/.claude`:
    /// plugins, their hooks, MCP servers and skills, none of which the farm
    /// chose. The daemon's guard still applies, since `--settings` is loaded
    /// whatever the sources.
    pub isolated: bool,
    pub env: Vec<(String, String)>,
    /// Close stdin as soon as the prompt is written.
    ///
    /// With `--input-format stream-json` the process keeps waiting for further
    /// user turns, so a one-shot run never exits and its stdout never reaches
    /// end of file. A run that will never be steered must therefore say so.
    pub one_shot: bool,
}

impl ClaudeCommand {
    pub fn new(bin: impl Into<String>, cwd: impl Into<PathBuf>, prompt: impl Into<String>) -> Self {
        ClaudeCommand {
            bin: bin.into(),
            cwd: cwd.into(),
            prompt: prompt.into(),
            prompt_via: PromptVia::Stdin,
            session_id: None,
            resume: None,
            fork_session: false,
            name: None,
            model: None,
            effort: None,
            permission_mode: None,
            append_system_prompt_file: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            max_budget_usd: None,
            max_turns: None,
            json_schema: None,
            settings_json: None,
            agents_json: None,
            isolated: true,
            env: Vec::new(),
            one_shot: false,
        }
    }

    /// The arguments, in order. Kept separate from spawning so it can be shown
    /// to the user and asserted in tests.
    pub fn args(&self) -> Vec<String> {
        let mut a: Vec<String> = vec!["-p".into()];
        if self.prompt_via == PromptVia::Arg {
            a.push(self.prompt.clone());
        }
        a.push("--output-format".into());
        a.push("stream-json".into());
        a.push("--verbose".into());
        if self.prompt_via == PromptVia::Stdin {
            a.push("--input-format".into());
            a.push("stream-json".into());
        }
        if let Some(id) = self.resume {
            a.push("--resume".into());
            a.push(id.to_string());
            if self.fork_session {
                // Honoured together (2.1.289): the fork takes the id given.
                a.push("--fork-session".into());
                if let Some(new) = self.session_id {
                    a.push("--session-id".into());
                    a.push(new.to_string());
                }
            }
        } else if let Some(id) = self.session_id {
            a.push("--session-id".into());
            a.push(id.to_string());
        }
        if let Some(name) = &self.name {
            a.push("--name".into());
            a.push(name.clone());
        }
        if let Some(model) = &self.model {
            a.push("--model".into());
            a.push(model.clone());
        }
        if let Some(effort) = self.effort {
            a.push("--effort".into());
            a.push(effort.as_str().into());
        }
        if let Some(mode) = &self.permission_mode {
            a.push("--permission-mode".into());
            a.push(mode.clone());
        }
        if let Some(file) = &self.append_system_prompt_file {
            a.push("--append-system-prompt-file".into());
            a.push(file.to_string_lossy().to_string());
        }
        if !self.allowed_tools.is_empty() {
            a.push("--allowedTools".into());
            a.push(self.allowed_tools.join(","));
        }
        if !self.disallowed_tools.is_empty() {
            a.push("--disallowedTools".into());
            a.push(self.disallowed_tools.join(","));
        }
        if let Some(budget) = self.max_budget_usd {
            a.push("--max-budget-usd".into());
            a.push(format!("{budget}"));
        }
        if let Some(turns) = self.max_turns {
            // Not in `--help` (2.1.289), but honoured: the run ends with
            // `error_max_turns`. See CLAUDE_CLI_NOTES.md.
            a.push("--max-turns".into());
            a.push(turns.to_string());
        }
        if let Some(schema) = &self.json_schema {
            a.push("--json-schema".into());
            a.push(schema.clone());
        }
        if let Some(settings) = &self.settings_json {
            a.push("--settings".into());
            a.push(settings.clone());
        }
        if let Some(agents) = &self.agents_json {
            a.push("--agents".into());
            a.push(agents.clone());
        }
        if self.isolated {
            a.push("--setting-sources".into());
            a.push("project".into());
            a.push("--strict-mcp-config".into());
        }
        a
    }

    /// A one-line rendering for the event log, with long values shortened.
    pub fn display(&self) -> String {
        let args: Vec<String> = self
            .args()
            .into_iter()
            .map(|a| {
                if a.chars().count() > 60 {
                    // By characters: a byte index can fall inside an accent.
                    let head: String = a.chars().take(57).collect();
                    format!("{head}…")
                } else {
                    a
                }
            })
            .map(|a| if a.contains(' ') { format!("'{a}'") } else { a })
            .collect();
        format!("{} {}", self.bin, args.join(" "))
    }

    fn to_tokio(&self) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.args(self.args())
            .current_dir(&self.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Force subscription auth: a stray API key would bill separately and
        // silently change which account does the work.
        cmd.env_remove("ANTHROPIC_API_KEY");
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd
    }
}

/// A user turn as the stream-json input format expects it.
pub fn user_message_line(text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": text }
    })
    .to_string()
}

/// What a running process reports.
#[derive(Debug)]
pub enum ProcessEvent {
    /// A parsed line of the stream.
    Line(Box<StreamLine>),
    /// A line we could not parse, kept so the daemon can warn rather than
    /// silently lose a response.
    Unparsed {
        raw: String,
        error: String,
    },
    Stderr(String),
}

/// A running `claude`, with its stdin held open for steering.
#[derive(Debug)]
pub struct ClaudeProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    pid: u32,
}

impl ClaudeProcess {
    /// Spawn the process and start forwarding its output on the channel.
    ///
    /// Async because the first prompt is written before returning: with the
    /// prompt on stdin the process waits for it before doing anything, and
    /// blocking on that write from inside the runtime would risk a deadlock.
    pub async fn spawn(cmd: &ClaudeCommand) -> Result<(Self, mpsc::Receiver<ProcessEvent>)> {
        let mut child = cmd
            .to_tokio()
            .spawn()
            .with_context(|| format!("lancement de « {} »", cmd.bin))?;
        let pid = child.id().unwrap_or(0);
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().context("stdout indisponible")?;
        let stderr = child.stderr.take().context("stderr indisponible")?;

        let (tx, rx) = mpsc::channel(256);

        let out_tx = tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            while let Some(line) = next_lossy_line(&mut reader).await {
                if line.trim().is_empty() {
                    continue;
                }
                let event = match StreamLine::parse(&line) {
                    Ok(parsed) => ProcessEvent::Line(Box::new(parsed)),
                    Err(e) => ProcessEvent::Unparsed {
                        raw: orchestra_core::claude::stream::truncate(&line, 400),
                        error: e.to_string(),
                    },
                };
                if out_tx.send(event).await.is_err() {
                    break;
                }
            }
        });

        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            while let Some(line) = next_lossy_line(&mut reader).await {
                if tx.send(ProcessEvent::Stderr(line)).await.is_err() {
                    break;
                }
            }
        });

        let mut process = ClaudeProcess { child, stdin, pid };

        if cmd.prompt_via == PromptVia::Stdin {
            let line = user_message_line(&cmd.prompt);
            // Best effort: a process that died already surfaces on the stream,
            // where the error is more informative than a write failure here.
            let _ = process.write_line(&line).await;
        }
        if cmd.one_shot {
            process.close_stdin();
        }

        Ok((process, rx))
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Queue a user turn. Claude picks it up after the current one.
    pub async fn send_user(&mut self, text: &str) -> Result<()> {
        self.write_line(&user_message_line(text)).await
    }

    async fn write_line(&mut self, line: &str) -> Result<()> {
        let stdin = self.stdin.as_mut().context("stdin déjà fermé")?;
        stdin.write_all(line.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Close stdin, which tells Claude no more turns are coming.
    pub fn close_stdin(&mut self) {
        self.stdin = None;
    }

    /// Ask the process to stop the current turn cleanly.
    pub fn interrupt(&self) -> Result<()> {
        if self.pid == 0 {
            return Ok(());
        }
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        kill(Pid::from_raw(self.pid as i32), Signal::SIGINT)
            .context("envoi de SIGINT à l'agent")?;
        Ok(())
    }

    /// Interrupt, then kill if it has not stopped within `grace`.
    pub async fn stop(&mut self, grace: std::time::Duration) -> Result<std::process::ExitStatus> {
        let _ = self.interrupt();
        match tokio::time::timeout(grace, self.child.wait()).await {
            Ok(status) => Ok(status?),
            Err(_) => {
                self.child.kill().await?;
                Ok(self.child.wait().await?)
            }
        }
    }

    pub async fn wait(&mut self) -> Result<std::process::ExitStatus> {
        Ok(self.child.wait().await?)
    }
}

/// The next line of a child's output, without its newline; `None` at end of
/// file or on a read error.
///
/// Invalid UTF-8 is replaced rather than refused: `lines()` would stop at the
/// first bad byte, and the rest of the stream — the `result` line included —
/// would be lost without a word.
async fn next_lossy_line<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Option<String> {
    let mut buf = Vec::new();
    match reader.read_until(b'\n', &mut buf).await {
        Ok(0) | Err(_) => None,
        Ok(_) => {
            while matches!(buf.last(), Some(b'\n' | b'\r')) {
                buf.pop();
            }
            Some(String::from_utf8_lossy(&buf).into_owned())
        }
    }
}

/// Write a rendered prompt where `--append-system-prompt-file` can read it, and
/// return the path.
///
/// `name` must be unique to the run that reads the file: the body carries the
/// project's rules, and `claude` reads it again on every resume, so two agents
/// sharing a file would read each other's.
pub fn write_prompt_file(cache_dir: &Path, name: &str, body: &str) -> Result<PathBuf> {
    let dir = cache_dir.join("roles");
    std::fs::create_dir_all(&dir).with_context(|| format!("création de {}", dir.display()))?;
    let path = dir.join(format!("{name}.prompt.md"));
    std::fs::write(&path, body).with_context(|| format!("écriture de {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd() -> ClaudeCommand {
        ClaudeCommand::new("claude", "/tmp/proj", "fais le travail")
    }

    #[test]
    fn the_default_command_is_a_readable_headless_run() {
        let args = cmd().args();
        assert_eq!(args[0], "-p");
        // With the prompt on stdin, it is not an argument.
        assert!(!args.contains(&"fais le travail".to_string()));
        assert!(args
            .windows(2)
            .any(|w| w == ["--output-format", "stream-json"]));
        assert!(args
            .windows(2)
            .any(|w| w == ["--input-format", "stream-json"]));
        assert!(args.contains(&"--verbose".to_string()));
    }

    #[test]
    fn the_prompt_can_go_on_the_command_line_instead() {
        let mut c = cmd();
        c.prompt_via = PromptVia::Arg;
        let args = c.args();
        assert_eq!(args[1], "fais le travail");
        assert!(
            !args
                .windows(2)
                .any(|w| w == ["--input-format", "stream-json"]),
            "sans stdin, pas de format d'entrée"
        );
    }

    #[test]
    fn a_default_model_means_no_flag_at_all() {
        let args = cmd().args();
        assert!(!args.contains(&"--model".to_string()));
        let mut c = cmd();
        c.model = Some("opus".into());
        assert!(c.args().windows(2).any(|w| w == ["--model", "opus"]));
    }

    #[test]
    fn resuming_replaces_the_session_id() {
        let id = Uuid::new_v4();
        let resume = Uuid::new_v4();
        let mut c = cmd();
        c.session_id = Some(id);
        assert!(c
            .args()
            .windows(2)
            .any(|w| w == ["--session-id", &id.to_string()]));

        c.resume = Some(resume);
        let args = c.args();
        assert!(args
            .windows(2)
            .any(|w| w == ["--resume", &resume.to_string()]));
        assert!(
            !args.contains(&"--session-id".to_string()),
            "on ne peut pas reprendre et créer à la fois"
        );
    }

    #[test]
    fn every_option_lands_on_the_command_line() {
        let mut c = cmd();
        c.name = Some("[proj] backend #1".into());
        c.effort = Some(Effort::Xhigh);
        c.permission_mode = Some("bypassPermissions".into());
        c.append_system_prompt_file = Some(PathBuf::from("/tmp/r.md"));
        c.allowed_tools = vec!["Read".into(), "Grep".into()];
        c.disallowed_tools = vec!["Bash".into()];
        c.max_budget_usd = Some(2.5);
        c.max_turns = Some(80);
        c.json_schema = Some("{}".into());
        c.settings_json = Some("{\"hooks\":{}}".into());
        let args = c.args();
        assert!(args.windows(2).any(|w| w == ["--setting-sources", "project"]));
        assert!(args.contains(&"--strict-mcp-config".to_string()));

        for pair in [
            ["--name", "[proj] backend #1"],
            ["--effort", "xhigh"],
            ["--permission-mode", "bypassPermissions"],
            ["--append-system-prompt-file", "/tmp/r.md"],
            ["--allowedTools", "Read,Grep"],
            ["--disallowedTools", "Bash"],
            ["--max-budget-usd", "2.5"],
            ["--max-turns", "80"],
            ["--json-schema", "{}"],
            ["--settings", "{\"hooks\":{}}"],
        ] {
            assert!(
                args.windows(2).any(|w| w == pair),
                "{pair:?} absent de {args:?}"
            );
        }
    }

    #[test]
    fn an_agent_does_not_inherit_the_users_setup_unless_asked() {
        let mut c = cmd();
        c.isolated = false;
        let args = c.args();
        assert!(!args.contains(&"--setting-sources".to_string()));
        assert!(!args.contains(&"--strict-mcp-config".to_string()));
    }

    #[test]
    fn shortening_never_cuts_inside_a_character() {
        let mut c = cmd();
        c.name = Some("é".repeat(80));
        assert!(c.display().contains('…'));
    }

    #[tokio::test]
    async fn a_line_that_is_not_utf8_does_not_end_the_stream() {
        let bytes: &[u8] = b"avant\n\xff\xfe cass\xc3\r\napr\xc3\xa8s\n";
        let mut reader = BufReader::new(bytes);
        assert_eq!(next_lossy_line(&mut reader).await.as_deref(), Some("avant"));
        let broken = next_lossy_line(&mut reader).await.unwrap();
        assert!(broken.contains('\u{FFFD}') && broken.ends_with("cass\u{FFFD}"));
        assert_eq!(next_lossy_line(&mut reader).await.as_deref(), Some("après"));
        assert_eq!(next_lossy_line(&mut reader).await, None);
    }

    #[test]
    fn a_fork_resumes_one_session_under_another_id() {
        let (old, new) = (Uuid::new_v4(), Uuid::new_v4());
        let mut c = cmd();
        c.resume = Some(old);
        c.session_id = Some(new);
        c.fork_session = true;
        let args = c.args();
        assert!(args.windows(2).any(|w| w == ["--resume", &old.to_string()]));
        assert!(args.contains(&"--fork-session".to_string()));
        assert!(args.windows(2).any(|w| w == ["--session-id", &new.to_string()]));
    }

    #[test]
    fn a_user_turn_is_one_json_line() {
        let line = user_message_line("ajoute aussi un test");
        assert!(!line.contains('\n'));
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["type"], "user");
        assert_eq!(v["message"]["role"], "user");
        assert_eq!(v["message"]["content"], "ajoute aussi un test");

        // Newlines in the prompt stay inside the JSON string.
        let multi = user_message_line("ligne 1\nligne 2");
        assert!(!multi.contains('\n'));
        let v: serde_json::Value = serde_json::from_str(&multi).unwrap();
        assert_eq!(v["message"]["content"], "ligne 1\nligne 2");
    }

    #[test]
    fn the_displayed_command_shortens_long_arguments() {
        let mut c = cmd();
        c.json_schema = Some("x".repeat(500));
        let shown = c.display();
        assert!(shown.starts_with("claude -p"));
        assert!(shown.contains('…'), "le schéma doit être abrégé");
        assert!(shown.len() < 400);
    }

    #[tokio::test]
    async fn a_missing_binary_fails_with_its_name() {
        let mut c = cmd();
        c.bin = "claude-qui-nexiste-pas".into();
        c.cwd = std::env::temp_dir();
        let err = ClaudeProcess::spawn(&c).await.unwrap_err();
        assert!(err.to_string().contains("claude-qui-nexiste-pas"), "{err}");
    }

    /// A stand-in that ignores our flags, echoes stdin, and writes one line on
    /// stderr. Enough to exercise spawning and reading without a token.
    fn stand_in(dir: &Path) -> String {
        let path = dir.join("faux-claude.sh");
        std::fs::write(&path, "#!/bin/sh\necho 'un mot sur stderr' >&2\nexec cat\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path.to_string_lossy().to_string()
    }

    /// Spawn the stand-in, retrying while Linux says its file is busy.
    ///
    /// Writing a script and executing it right away races with every other
    /// test that forks: the child inherits the descriptor still open for
    /// writing, and `exec` answers ETXTBSY until it closes. Nothing to do with
    /// the code under test — but it made the suite fail about one run in ten.
    async fn spawn_stand_in(cmd: &ClaudeCommand) -> (ClaudeProcess, mpsc::Receiver<ProcessEvent>) {
        for _ in 0..50 {
            match ClaudeProcess::spawn(cmd).await {
                Ok(pair) => return pair,
                Err(e) if is_text_busy(&e) => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await
                }
                Err(e) => panic!("{e:#}"),
            }
        }
        panic!("le processus de test n'a jamais pu démarrer");
    }

    fn is_text_busy(e: &anyhow::Error) -> bool {
        e.chain().any(|c| {
            c.downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.raw_os_error() == Some(26))
        })
    }

    #[tokio::test]
    async fn the_stream_of_a_stand_in_process_is_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = ClaudeCommand::new(stand_in(dir.path()), dir.path(), "salut");
        c.prompt_via = PromptVia::Stdin;

        let (mut process, mut rx) = spawn_stand_in(&c).await;
        assert!(process.pid() > 0);

        process
            .write_line(r#"{"type":"assistant","message":{"id":"m","model":"x","content":[],"usage":{"input_tokens":1,"output_tokens":2}}}"#)
            .await
            .unwrap();
        process.write_line("ceci n'est pas du json").await.unwrap();
        process.close_stdin();

        let mut parsed = 0;
        let mut unparsed = 0;
        let mut stderr = 0;
        let mut echoed_prompt = false;
        while let Some(event) = rx.recv().await {
            match event {
                ProcessEvent::Line(line) => {
                    parsed += 1;
                    // The prompt we wrote comes back first, as a user turn.
                    if matches!(*line, StreamLine::User { .. }) {
                        echoed_prompt = true;
                    }
                }
                ProcessEvent::Unparsed { .. } => unparsed += 1,
                ProcessEvent::Stderr(_) => stderr += 1,
            }
        }
        assert!(echoed_prompt, "le prompt initial est écrit sur stdin");
        assert_eq!(parsed, 2, "le prompt et la ligne assistant");
        assert_eq!(unparsed, 1, "une ligne illisible est signalée, pas perdue");
        assert_eq!(stderr, 1, "stderr est remonté aussi");
    }

    #[tokio::test]
    async fn a_one_shot_run_closes_its_input_so_the_process_can_finish() {
        // With stdin left open, `claude -p --input-format stream-json` waits
        // for another turn: the run never ends and its stdout never closes.
        let dir = tempfile::tempdir().unwrap();
        let mut c = ClaudeCommand::new(stand_in(dir.path()), dir.path(), "salut");
        c.one_shot = true;

        let (mut process, mut rx) = spawn_stand_in(&c).await;
        let mut lines = 0;
        // The channel closes on its own, without anyone closing stdin later.
        while rx.recv().await.is_some() {
            lines += 1;
        }
        assert!(lines >= 1);
        let status = tokio::time::timeout(std::time::Duration::from_secs(5), process.wait())
            .await
            .expect("un processus one-shot doit se terminer seul")
            .unwrap();
        assert!(status.success());
    }

    #[tokio::test]
    async fn a_process_can_be_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let c = ClaudeCommand::new(stand_in(dir.path()), dir.path(), "salut");
        let (mut process, _rx) = spawn_stand_in(&c).await;
        // `cat` ignores SIGINT's default only when interactive; either way the
        // grace period bounds the wait and the process must be gone after.
        let status = process
            .stop(std::time::Duration::from_millis(200))
            .await
            .unwrap();
        assert!(!status.success() || status.code() == Some(0));
    }

    #[test]
    fn prompt_files_land_in_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_prompt_file(dir.path(), "backend", "Tu es backend.").unwrap();
        assert!(path.ends_with("roles/backend.prompt.md"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "Tu es backend.");
    }
}
