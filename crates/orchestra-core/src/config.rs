//! `~/.config/orchestra/config.toml` plus the XDG paths everything derives.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::model::Effort;
use crate::pricing::PriceTable;

/// `defaults.model = "default"` means: do not pass `--model`, let Claude Code
/// use whatever the user configured.
pub const MODEL_DEFAULT: &str = "default";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub daemon: DaemonConfig,
    pub defaults: AgentDefaults,
    pub orchestrator: AgentDefaults,
    pub review: ReviewConfig,
    pub checks: ChecksConfig,
    pub integration: IntegrationConfig,
    pub zellij: ZellijConfig,
    pub models: ModelsConfig,
    pub pricing: PriceTable,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            daemon: DaemonConfig::default(),
            defaults: AgentDefaults::default(),
            orchestrator: AgentDefaults {
                model: MODEL_DEFAULT.into(),
                effort: Effort::Medium,
                max_budget_usd: Some(2.0),
            },
            review: ReviewConfig::default(),
            checks: ChecksConfig::default(),
            integration: IntegrationConfig::default(),
            zellij: ZellijConfig::default(),
            models: ModelsConfig::default(),
            pricing: PriceTable::defaults(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    /// Overrides the XDG data dir when set (the SQLite file lives here).
    pub data_dir: Option<PathBuf>,
    /// Where `git worktree add` puts ticket worktrees.
    pub worktrees_dir: Option<PathBuf>,
    pub max_concurrent_agents: usize,
    /// How many times a crashed agent is resumed before giving up.
    pub max_attempts: u32,
    pub claude_bin: String,
    /// Root of the Claude Code transcripts the watcher tails.
    pub transcripts_dir: Option<PathBuf>,
    pub branch_prefix: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        DaemonConfig {
            data_dir: None,
            worktrees_dir: None,
            max_concurrent_agents: 3,
            max_attempts: 2,
            claude_bin: "claude".into(),
            transcripts_dir: None,
            branch_prefix: "orch/".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentDefaults {
    pub model: String,
    pub effort: Effort,
    pub max_budget_usd: Option<f64>,
}

impl Default for AgentDefaults {
    fn default() -> Self {
        AgentDefaults {
            model: MODEL_DEFAULT.into(),
            effort: Effort::Medium,
            max_budget_usd: Some(5.0),
        }
    }
}

impl AgentDefaults {
    /// `None` when the model is `"default"`, i.e. do not pass `--model` at all.
    pub fn model_flag(&self) -> Option<&str> {
        let m = self.model.trim();
        if m.is_empty() || m == MODEL_DEFAULT {
            None
        } else {
            Some(m)
        }
    }
}

/// The relecture that closes every ticket.
///
/// It is not left to the orchestrator's judgement: a ticket nobody read is a
/// ticket whose author is the only one who has seen the code. The reviewer is
/// appended to every proposal, where the user can still take it out before
/// accepting — the orchestrator proposes, the user decides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReviewConfig {
    /// Append the reviewer to every team.
    pub enabled: bool,
    /// The role it uses, which must exist in the catalog.
    pub role: String,
    /// How many correction rounds a blocking verdict may trigger. Zero means
    /// the relecture reports and stops.
    pub max_rounds: u32,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        ReviewConfig {
            enabled: true,
            role: "reviewer".into(),
            // Two rounds catch what one round misses; past that the reviewer
            // and the team are usually disagreeing rather than converging, and
            // that is a call for the user.
            max_rounds: 2,
        }
    }
}

/// The repository's own verification, run by the daemon rather than reported
/// by an agent.
///
/// A relecture says whether the tests pass; this says so with an exit code.
/// The two are not redundant: one judges, the other measures, and only the
/// measurement can be trusted about itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChecksConfig {
    pub enabled: bool,
    /// The commands to run, in order, in the ticket's worktree. Empty means
    /// the daemon guesses one from the main repository — see
    /// [`crate::checks::detect`] — and a project it cannot guess simply has no
    /// gate rather than a made-up one.
    pub commands: Vec<String>,
    /// How long a single command is given before it is killed. A suite that
    /// hangs must not hold a ticket for the afternoon.
    pub timeout_secs: u64,
    /// How many repair rounds a red check may trigger, on the same budget
    /// logic as [`ReviewConfig::max_rounds`]. Past it the ticket fails, which
    /// is what it is: the team did not deliver something that builds.
    pub max_rounds: u32,
}

impl Default for ChecksConfig {
    fn default() -> Self {
        ChecksConfig {
            enabled: true,
            commands: Vec::new(),
            // Fifteen minutes is a long suite and a short afternoon.
            timeout_secs: 900,
            // The same two as the relecture, so there is one number in the
            // user's head rather than two.
            max_rounds: 2,
        }
    }
}

/// The one role allowed to run git for real.
///
/// It is not part of the team: it is launched from the ticket screen, only
/// once the relecture said nothing blocks. Even then it stays in the worktree
/// — it brings the default branch into the ticket's branch and settles the
/// conflicts there. The fusion itself is a `--ff-only` run by the daemon, so
/// no agent ever holds a shell in the main repository.
/// How a cleared branch reaches the default branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationMode {
    /// The daemon fast-forwards the default branch on this machine.
    #[default]
    Merge,
    /// The branch is pushed and a pull request is opened for it; the merge is
    /// the user's click, not ours.
    Pr,
}

impl IntegrationMode {
    pub fn is_pr(self) -> bool {
        matches!(self, IntegrationMode::Pr)
    }

    pub fn label_fr(self) -> &'static str {
        match self {
            IntegrationMode::Merge => "fusion locale",
            IntegrationMode::Pr => "pull request",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IntegrationConfig {
    pub enabled: bool,
    /// The role it uses, which must exist in the catalog.
    pub role: String,
    /// `merge` fuses here; `pr` pushes the branch and opens a pull request.
    /// A project with no remote falls back to `merge`, with a warning: there
    /// is nowhere to open a request.
    pub mode: IntegrationMode,
    /// How often an open pull request is checked, in seconds. Merged, the
    /// ticket closes on its own; closed without merging, it is cancelled.
    pub pr_poll_secs: u64,
    /// Let Orchestra push: the integrator sends its branch up, and the daemon
    /// sends the default branch up once the fusion is done. Off by default —
    /// pushing is visible outside this machine, and that is the user's call.
    pub push: bool,
    /// Delete the worktree once the branch is merged. The branch is kept.
    pub remove_worktree: bool,
}

impl Default for IntegrationConfig {
    fn default() -> Self {
        IntegrationConfig {
            enabled: true,
            role: "integrator".into(),
            mode: IntegrationMode::Merge,
            // A pull request is not a race: a minute is invisible to the user
            // and costs one `gh` call per open request.
            pr_poll_secs: 60,
            push: false,
            remove_worktree: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZellijConfig {
    /// Open an `orchestra tail` pane for every agent that starts.
    pub auto_pane: bool,
    pub layout: String,
}

impl Default for ZellijConfig {
    fn default() -> Self {
        ZellijConfig {
            auto_pane: true,
            layout: "orchestra".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelsConfig {
    /// Offered when cycling the model field in the proposal editor.
    pub aliases: Vec<String>,
}

impl Default for ModelsConfig {
    fn default() -> Self {
        ModelsConfig {
            // No `default` entry: leaving the model unset already means that,
            // and offering both would put the same choice in the list twice.
            aliases: ["fable", "opus", "sonnet", "haiku"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

impl Config {
    pub fn from_toml(s: &str) -> Result<Self> {
        toml::from_str(s).map_err(CoreError::from)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("la config est sérialisable")
    }

    /// Read the file if it exists, else return defaults.
    pub fn load_from(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => Config::from_toml(&s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(CoreError::Config(format!(
                "lecture de {}: {e}",
                path.display()
            ))),
        }
    }

    pub fn load() -> Result<Self> {
        Config::load_from(&Paths::config_file())
    }

    pub fn validate(&self) -> Result<()> {
        if self.daemon.max_concurrent_agents == 0 {
            return Err(CoreError::Config(
                "daemon.max_concurrent_agents doit être au moins 1".into(),
            ));
        }
        if self.daemon.claude_bin.trim().is_empty() {
            return Err(CoreError::Config("daemon.claude_bin est vide".into()));
        }
        if self.daemon.max_attempts == 0 {
            return Err(CoreError::Config(
                "daemon.max_attempts doit être au moins 1".into(),
            ));
        }
        for (label, d) in [
            ("defaults", &self.defaults),
            ("orchestrator", &self.orchestrator),
        ] {
            if let Some(b) = d.max_budget_usd {
                if b <= 0.0 {
                    return Err(CoreError::Config(format!(
                        "{label}.max_budget_usd doit être positif"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Every filesystem location the project uses. Env overrides come first so a
/// test or a second instance can be fully relocated.
pub struct Paths;

impl Paths {
    fn env_path(key: &str) -> Option<PathBuf> {
        std::env::var_os(key)
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
    }

    fn home() -> PathBuf {
        Self::env_path("HOME").unwrap_or_else(|| PathBuf::from("/"))
    }

    /// `$ORCHESTRA_CONFIG_DIR`, `$XDG_CONFIG_HOME/orchestra`, `~/.config/orchestra`.
    pub fn config_dir() -> PathBuf {
        Self::env_path("ORCHESTRA_CONFIG_DIR")
            .or_else(|| Self::env_path("XDG_CONFIG_HOME").map(|p| p.join("orchestra")))
            .unwrap_or_else(|| Self::home().join(".config/orchestra"))
    }

    pub fn config_file() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    /// Global role catalog.
    pub fn roles_dir() -> PathBuf {
        Self::config_dir().join("roles")
    }

    pub fn data_dir() -> PathBuf {
        Self::env_path("ORCHESTRA_DATA_DIR")
            .or_else(|| Self::env_path("XDG_DATA_HOME").map(|p| p.join("orchestra")))
            .unwrap_or_else(|| Self::home().join(".local/share/orchestra"))
    }

    pub fn db_file() -> PathBuf {
        Self::data_dir().join("orchestra.db")
    }

    pub fn worktrees_dir() -> PathBuf {
        Self::data_dir().join("worktrees")
    }

    /// Logs and per-agent stderr.
    pub fn state_dir() -> PathBuf {
        Self::env_path("ORCHESTRA_STATE_DIR")
            .or_else(|| Self::env_path("XDG_STATE_HOME").map(|p| p.join("orchestra")))
            .unwrap_or_else(|| Self::home().join(".local/state/orchestra"))
    }

    /// Rendered role prompts handed to `claude --append-system-prompt-file`.
    pub fn cache_dir() -> PathBuf {
        Self::env_path("ORCHESTRA_CACHE_DIR")
            .or_else(|| Self::env_path("XDG_CACHE_HOME").map(|p| p.join("orchestra")))
            .unwrap_or_else(|| Self::home().join(".cache/orchestra"))
    }

    /// `$ORCHESTRA_SOCK`, else `$XDG_RUNTIME_DIR/orchestra.sock`, else
    /// `/tmp/orchestra-<uid>.sock`.
    pub fn socket() -> PathBuf {
        if let Some(p) = Self::env_path("ORCHESTRA_SOCK") {
            return p;
        }
        if let Some(rt) = Self::env_path("XDG_RUNTIME_DIR") {
            return rt.join("orchestra.sock");
        }
        let uid = Self::env_path("UID")
            .and_then(|v| v.to_string_lossy().parse::<u32>().ok())
            .unwrap_or(1000);
        PathBuf::from(format!("/tmp/orchestra-{uid}.sock"))
    }

    /// Guards against a second daemon; lives next to the socket.
    pub fn lock_file() -> PathBuf {
        Self::socket().with_extension("lock")
    }

    /// Claude Code's own transcript root.
    pub fn transcripts_dir() -> PathBuf {
        Self::env_path("ORCHESTRA_TRANSCRIPTS_DIR")
            .unwrap_or_else(|| Self::home().join(".claude/projects"))
    }

    /// Per-project overrides: `<project>/.orchestra`.
    pub fn project_dir(project: &Path) -> PathBuf {
        project.join(".orchestra")
    }

    pub fn project_roles_dir(project: &Path) -> PathBuf {
        Self::project_dir(project).join("roles")
    }
}

/// Resolved paths, with `config.toml` overrides already applied.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPaths {
    pub data_dir: PathBuf,
    pub db_file: PathBuf,
    pub worktrees_dir: PathBuf,
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub socket: PathBuf,
    pub lock_file: PathBuf,
    pub transcripts_dir: PathBuf,
    pub roles_dir: PathBuf,
}

impl ResolvedPaths {
    pub fn from_config(cfg: &Config) -> Self {
        let data_dir = cfg.daemon.data_dir.clone().unwrap_or_else(Paths::data_dir);
        ResolvedPaths {
            db_file: data_dir.join("orchestra.db"),
            worktrees_dir: cfg
                .daemon
                .worktrees_dir
                .clone()
                .unwrap_or_else(|| data_dir.join("worktrees")),
            data_dir,
            state_dir: Paths::state_dir(),
            cache_dir: Paths::cache_dir(),
            socket: Paths::socket(),
            lock_file: Paths::lock_file(),
            transcripts_dir: cfg
                .daemon
                .transcripts_dir
                .clone()
                .unwrap_or_else(Paths::transcripts_dir),
            roles_dir: Paths::roles_dir(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_matches_defaults() {
        // assets/config.example.toml is generated from Config::default(); if this
        // fails, regenerate it rather than editing it by hand.
        let generated = Config::default().to_toml();
        let shipped = include_str!("../../../assets/config.example.toml");
        let strip = |s: &str| {
            s.lines()
                .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(strip(&generated), strip(shipped));
    }

    #[test]
    fn defaults_are_valid_and_round_trip() {
        let c = Config::default();
        c.validate().unwrap();
        let back = Config::from_toml(&c.to_toml()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn partial_file_keeps_defaults() {
        let c = Config::from_toml(
            r#"
            [daemon]
            max_concurrent_agents = 8

            [defaults]
            model = "sonnet"
            effort = "max"
        "#,
        )
        .unwrap();
        assert_eq!(c.daemon.max_concurrent_agents, 8);
        assert_eq!(c.daemon.claude_bin, "claude");
        assert_eq!(c.defaults.model, "sonnet");
        assert_eq!(c.defaults.effort, Effort::Max);
        // Untouched sections keep their defaults, pricing included.
        assert_eq!(c.orchestrator.effort, Effort::Medium);
        assert!(c.pricing.lookup("claude-opus-5").is_some());
    }

    #[test]
    fn model_default_means_no_flag() {
        let mut d = AgentDefaults::default();
        assert_eq!(d.model_flag(), None);
        d.model = "  ".into();
        assert_eq!(d.model_flag(), None);
        d.model = "opus".into();
        assert_eq!(d.model_flag(), Some("opus"));
    }

    #[test]
    fn validation_catches_nonsense() {
        let mut c = Config::default();
        c.daemon.max_concurrent_agents = 0;
        assert!(c.validate().is_err());

        let mut c = Config::default();
        c.daemon.claude_bin = "".into();
        assert!(c.validate().is_err());

        let mut c = Config::default();
        c.daemon.max_attempts = 0;
        assert!(c.validate().is_err());

        let mut c = Config::default();
        c.defaults.max_budget_usd = Some(0.0);
        assert!(c.validate().is_err());
    }

    #[test]
    fn missing_file_yields_defaults() {
        let c = Config::load_from(Path::new("/nonexistent/orchestra/config.toml")).unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn config_overrides_resolved_paths() {
        let mut c = Config::default();
        c.daemon.data_dir = Some(PathBuf::from("/tmp/orch-data"));
        let p = ResolvedPaths::from_config(&c);
        assert_eq!(p.db_file, PathBuf::from("/tmp/orch-data/orchestra.db"));
        assert_eq!(p.worktrees_dir, PathBuf::from("/tmp/orch-data/worktrees"));

        c.daemon.worktrees_dir = Some(PathBuf::from("/tmp/elsewhere"));
        let p = ResolvedPaths::from_config(&c);
        assert_eq!(p.worktrees_dir, PathBuf::from("/tmp/elsewhere"));
    }

    #[test]
    fn lock_sits_next_to_the_socket() {
        let sock = Paths::socket();
        let lock = Paths::lock_file();
        assert_eq!(sock.parent(), lock.parent());
        assert_eq!(lock.extension().unwrap(), "lock");
    }

    #[test]
    fn project_paths_are_under_dot_orchestra() {
        let p = Path::new("/home/x/proj");
        assert_eq!(Paths::project_dir(p), Path::new("/home/x/proj/.orchestra"));
        assert_eq!(
            Paths::project_roles_dir(p),
            Path::new("/home/x/proj/.orchestra/roles")
        );
    }
}
