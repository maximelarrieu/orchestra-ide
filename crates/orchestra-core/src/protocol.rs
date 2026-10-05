//! Wire protocol between the daemon and its clients (TUI, CLI, `orchestra tail`).
//!
//! One JSON object per line over a Unix socket. A client sends [`Request`]s and
//! reads [`Frame`]s: responses to its own requests, plus events once it has
//! subscribed.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::conventions::{Rule, RuleKind, RuleStatus};
use crate::events::{Event, EventFilter};
use crate::model::{
    Agent, AgentId, Project, ProjectId, RoleDefinition, Team, Ticket, TicketId, TicketStatus,
    Todo, TodoId, TodoStatus, Tokens,
};

/// Bumped when a change would confuse an older client. The daemon refuses
/// clients announcing a different version.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub cmd: Command,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    Ping,
    ListProjects,
    AddProject {
        path: PathBuf,
        #[serde(default)]
        name: Option<String>,
    },
    /// Forget a project Orchestra discovered. Its tokens are kept, attached to
    /// no project.
    ForgetProject {
        project_id: ProjectId,
    },
    /// Point a project at the place its repository was moved to.
    MoveProject {
        project_id: ProjectId,
        path: PathBuf,
    },
    ListTickets {
        #[serde(default)]
        project_id: Option<ProjectId>,
        #[serde(default)]
        status: Option<Vec<TicketStatus>>,
    },
    GetTicket {
        ticket_id: TicketId,
    },
    /// What the ticket's branch changes against the default branch.
    GetDiff {
        ticket_id: TicketId,
    },
    /// Write an epic down. Planning its split is a separate command.
    CreateEpic {
        project_id: ProjectId,
        title: String,
        brief: String,
    },
    /// Ask the orchestrator for a split. Replies at once; the split arrives
    /// as `epic_split_ready` (or `epic_split_failed`).
    PlanEpic {
        epic_id: crate::epic::EpicId,
    },
    ListEpics {
        #[serde(default)]
        project_id: Option<ProjectId>,
    },
    GetEpic {
        epic_id: crate::epic::EpicId,
    },
    /// Create the tickets of the split, as the user edited it.
    AcceptEpic {
        epic_id: crate::epic::EpicId,
        proposal: crate::epic::EpicProposal,
    },
    CreateTicket {
        project_id: ProjectId,
        title: String,
        brief: String,
    },
    /// Runs the orchestrator. Replies immediately; the proposal arrives as a
    /// `ProposalReady` event.
    PlanTicket {
        ticket_id: TicketId,
    },
    AcceptProposal {
        ticket_id: TicketId,
        /// The team as edited by the user, not necessarily the proposal.
        team: Team,
    },
    LaunchTicket {
        ticket_id: TicketId,
        #[serde(default)]
        open_panes: bool,
    },
    CancelTicket {
        ticket_id: TicketId,
    },
    /// Runs the integrator on a ticket the relecture cleared, then fuses its
    /// branch into the project's default branch. Replies immediately; the
    /// outcome arrives as events.
    IntegrateTicket {
        ticket_id: TicketId,
    },
    /// Close a ticket by hand, for a branch the user merged himself.
    FinishTicket {
        ticket_id: TicketId,
    },
    /// Send a closed ticket back to « à relire », worktree recreated if it was
    /// cleaned up: its branch still holds the work.
    ReopenTicket {
        ticket_id: TicketId,
    },
    SteerAgent {
        agent_id: AgentId,
        text: String,
        /// Interrupt and resume instead of queueing a message.
        #[serde(default)]
        hard: bool,
    },
    CancelAgent {
        agent_id: AgentId,
    },
    /// Agents across every ticket, so a screen can find the one that is
    /// working without knowing which ticket it belongs to.
    ListAgents {
        #[serde(default)]
        only_active: bool,
    },
    /// Turns this connection into an event stream. `backlog` past events are
    /// replayed first, then live ones.
    Subscribe {
        #[serde(default)]
        filter: EventFilter,
        #[serde(default)]
        since_seq: Option<i64>,
        #[serde(default)]
        backlog: u32,
    },
    Unsubscribe,
    GetUsage {
        #[serde(default)]
        query: UsageQuery,
    },
    /// Show the pane that follows this agent, opening one if there is none.
    OpenPane {
        agent_id: AgentId,
    },
    /// Hand an agent back to the user: a pane with `claude --resume` on its
    /// session, in its worktree. The agent stops being ours and becomes
    /// [`crate::model::AgentStatus::Manual`].
    TakeOver {
        agent_id: AgentId,
    },
    ListRoles {
        #[serde(default)]
        project_id: Option<ProjectId>,
    },
    /// Sent by `orchestra-hook` on every hook event of a managed agent.
    Hook {
        #[serde(default)]
        agent_id: Option<AgentId>,
        payload: serde_json::Value,
    },
    /// Daemon version, uptime, running agents. Used by `orchestra doctor`.
    Status,
    ListTodos,
    CreateTodo {
        title: String,
        #[serde(default)]
        notes: String,
        #[serde(default)]
        urgent: bool,
        #[serde(default, with = "time::serde::rfc3339::option")]
        due_at: Option<OffsetDateTime>,
    },
    UpdateTodo {
        todo_id: TodoId,
        title: String,
        notes: String,
        urgent: bool,
        #[serde(default, with = "time::serde::rfc3339::option")]
        due_at: Option<OffsetDateTime>,
    },
    SetTodoStatus {
        todo_id: TodoId,
        status: TodoStatus,
    },
    DeleteTodo {
        todo_id: TodoId,
    },
    /// Creates a Draft ticket on `project_id` from this todo, then marks the
    /// todo as promoted. Replies immediately; `TodoPromoted` and
    /// `TicketCreated` follow as events.
    PromoteTodo {
        todo_id: TodoId,
        project_id: ProjectId,
        title: String,
        brief: String,
    },
    /// Conventions and decisions a project sees: the global conventions, its
    /// own overriding them by name, and its ADRs. Without a project, the
    /// global conventions alone.
    ListRules {
        #[serde(default)]
        project_id: Option<ProjectId>,
    },
    /// Write a new rule file from a skeleton and say where, so a client can
    /// open it in an editor. An ADR needs a project.
    CreateRule {
        #[serde(default)]
        project_id: Option<ProjectId>,
        rule_kind: RuleKind,
        title: String,
    },
    /// Accept, reject or supersede a rule, by rewriting its `status:` line.
    SetRuleStatus {
        #[serde(default)]
        project_id: Option<ProjectId>,
        rule_kind: RuleKind,
        name: String,
        status: RuleStatus,
    },
    DeleteRule {
        #[serde(default)]
        project_id: Option<ProjectId>,
        rule_kind: RuleKind,
        name: String,
    },
    /// Move a project's convention to the global ones, for every project.
    PromoteRule {
        project_id: ProjectId,
        name: String,
    },
    /// Write a new role file from a skeleton — in the project's catalog when
    /// there is a project, the global one otherwise — and say where, so a
    /// client can open it in an editor.
    CreateRole {
        #[serde(default)]
        project_id: Option<ProjectId>,
        name: String,
    },
    /// Open or close git to a role, by rewriting its `git:` line.
    SetRoleGit {
        #[serde(default)]
        project_id: Option<ProjectId>,
        name: String,
        git: crate::guard::GitPolicy,
    },
    DeleteRole {
        #[serde(default)]
        project_id: Option<ProjectId>,
        name: String,
    },
    /// Move a project's role to the global catalog, for every project.
    PromoteRole {
        project_id: ProjectId,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Reply {
    Pong,
    /// The command was accepted; its outcome will arrive as events.
    Ack,
    Projects {
        projects: Vec<Project>,
    },
    Project {
        project: Box<Project>,
    },
    Tickets {
        tickets: Vec<TicketSummary>,
    },
    Todos {
        todos: Vec<Todo>,
    },
    Ticket {
        detail: Box<TicketDetail>,
    },
    Diff {
        diff: Box<TicketDiff>,
    },
    Epic {
        detail: Box<EpicDetail>,
    },
    Epics {
        epics: Vec<crate::epic::Epic>,
    },
    Roles {
        /// `git` is always resolved in this reply: what an agent of the role
        /// will get, declared or not.
        roles: Vec<RoleDefinition>,
        /// Role files that could not be read, and why: a role the user edited
        /// into something broken must not simply vanish from the list.
        #[serde(default)]
        errors: Vec<String>,
    },
    /// A role file just written, for a client to open.
    RoleFile {
        path: PathBuf,
    },
    Rules {
        rules: Vec<Rule>,
        /// Files that could not be read, and why — a broken rule the user
        /// believes is enforced must show somewhere.
        #[serde(default)]
        errors: Vec<String>,
    },
    RuleFile {
        path: PathBuf,
    },
    Agents {
        agents: Vec<AgentSummary>,
    },
    Usage {
        rows: Vec<UsageRow>,
        totals: UsageTotals,
    },
    Pane {
        pane_id: String,
    },
    Status {
        status: Box<DaemonStatus>,
    },
    /// Subscription accepted; events follow on this connection.
    Subscribed {
        /// Last sequence in the store when the subscription started.
        current_seq: i64,
    },
    /// Verdict returned to `orchestra-hook`.
    Hook {
        allow: bool,
        #[serde(default)]
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        ApiError {
            code,
            message: message.into(),
        }
    }

    pub fn not_found(what: impl std::fmt::Display) -> Self {
        ApiError::new(ErrorCode::NotFound, format!("{what} introuvable"))
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        ApiError::new(ErrorCode::Invalid, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        ApiError::new(ErrorCode::Conflict, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        ApiError::new(ErrorCode::Internal, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        ApiError::new(ErrorCode::Unsupported, message)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotFound,
    /// Bad arguments, or an operation the current state forbids.
    Invalid,
    /// Something the daemon cannot do right now (agent already running, ...).
    Conflict,
    Internal,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(flatten)]
    pub result: ResponseResult,
}

/// `Result` needs a tagged representation to survive JSON round-trips.
///
/// The tag is `outcome`, not `status`: `Reply` is flattened into the same
/// object and already owns a `status` field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ResponseResult {
    Ok {
        #[serde(flatten)]
        reply: Reply,
    },
    Err {
        error: ApiError,
    },
}

impl ResponseResult {
    pub fn into_result(self) -> Result<Reply, ApiError> {
        match self {
            ResponseResult::Ok { reply } => Ok(reply),
            ResponseResult::Err { error } => Err(error),
        }
    }
}

impl From<Result<Reply, ApiError>> for ResponseResult {
    fn from(r: Result<Reply, ApiError>) -> Self {
        match r {
            Ok(reply) => ResponseResult::Ok { reply },
            Err(error) => ResponseResult::Err { error },
        }
    }
}

/// What the daemon writes on the socket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Frame {
    /// First line of every connection.
    Hello {
        version: String,
        protocol: u32,
        #[serde(with = "time::serde::rfc3339")]
        daemon_started: OffsetDateTime,
    },
    Response(Response),
    Event(Box<Event>),
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

/// One row of the board: a ticket plus what the board needs to draw it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TicketSummary {
    pub ticket: Ticket,
    pub agents_total: usize,
    pub agents_active: usize,
    pub agents_done: usize,
    pub tokens: Tokens,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    /// The pull request waiting for its human, when there is one: « à relire »
    /// and « PR à valider » are two different places to look.
    #[serde(default)]
    pub pull_request: Option<String>,
    /// Why a ready branch has not been merged yet, when it is waiting on the
    /// user: the card must say so, not only the activity strip.
    #[serde(default)]
    pub merge_blocked: Option<String>,
    /// What the ticket waits on the user for, if anything (`attention::of`).
    #[serde(default)]
    pub attention: Option<crate::attention::Attention>,
    /// The epic it belongs to, and what it still waits for there.
    #[serde(default)]
    pub epic: Option<crate::epic::EpicLink>,
}

/// An epic and the tickets it became, in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpicDetail {
    pub epic: crate::epic::Epic,
    pub tickets: Vec<EpicTicketRow>,
}

/// One ticket of an accepted epic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpicTicketRow {
    pub ticket_id: TicketId,
    pub number: i64,
    pub title: String,
    pub status: TicketStatus,
    /// Numbers of the tickets it depends on.
    #[serde(default)]
    pub depends_on: Vec<i64>,
}

/// One file a branch touches. `None` counts for a binary file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffFile {
    pub path: String,
    #[serde(default)]
    pub added: Option<u32>,
    #[serde(default)]
    pub removed: Option<u32>,
}

/// What a ticket's branch changes, from where it left the default branch
/// (`base...branch`): what the merge would bring in, nothing the default
/// branch did since.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketDiff {
    pub branch: String,
    pub base: String,
    pub files: Vec<DiffFile>,
    /// The unified patch, cut at `DIFF_PATCH_MAX` bytes on a line boundary.
    pub patch: String,
    #[serde(default)]
    pub truncated: bool,
}

/// The most of a patch sent over the socket: a frame is one line of at most
/// 4 MiB, and nobody reads more than this in a terminal.
pub const DIFF_PATCH_MAX: usize = 200 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TicketDetail {
    pub ticket: Ticket,
    pub project: Project,
    pub agents: Vec<AgentSummary>,
    pub tokens: Tokens,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    /// Most recent events of the ticket, oldest first.
    pub recent_events: Vec<Event>,
    /// What the last relecture concluded, when one has run. This is what makes
    /// the integration offer itself, so it is read from the ticket's whole
    /// history rather than from the events that happen to be recent.
    #[serde(default)]
    pub review: Option<ReviewOutcome>,
    /// What the repository's own checks said on their last pass, when there
    /// are any. `None` means this project declares none — not that they
    /// failed — so a project without a gate is never held back by one.
    #[serde(default)]
    pub checks: Option<crate::checks::ChecksOutcome>,
    /// The pull request opened for this ticket, while it is open.
    #[serde(default)]
    pub pull_request: Option<String>,
    /// Why the ready branch is still waiting to be merged, when it is.
    #[serde(default)]
    pub merge_blocked: Option<String>,
    /// How this project integrates, so the screen names the key for what it
    /// really does.
    #[serde(default)]
    pub integration_mode: crate::config::IntegrationMode,
}

/// The last relecture verdict of a ticket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewOutcome {
    pub round: u32,
    pub verdict: crate::review::Verdict,
    #[serde(default)]
    pub blocking: Vec<String>,
}

impl ReviewOutcome {
    pub fn is_ready(&self) -> bool {
        self.verdict.is_ready()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSummary {
    pub agent: Agent,
    pub tokens: Tokens,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    /// Number of assistant turns seen so far.
    pub turns: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub version: String,
    pub protocol: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    pub projects: usize,
    pub tickets_running: usize,
    pub agents_running: usize,
    pub last_seq: i64,
    /// Transcript files the watcher is tracking.
    pub watched_files: usize,
    #[serde(default)]
    pub claude_version: Option<String>,
}

// ---------------------------------------------------------------------------
// Usage queries
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    Project,
    Ticket,
    Agent,
    Role,
    Model,
    Day,
}

impl GroupBy {
    pub const ALL: [GroupBy; 6] = [
        GroupBy::Project,
        GroupBy::Ticket,
        GroupBy::Agent,
        GroupBy::Role,
        GroupBy::Model,
        GroupBy::Day,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            GroupBy::Project => "project",
            GroupBy::Ticket => "ticket",
            GroupBy::Agent => "agent",
            GroupBy::Role => "role",
            GroupBy::Model => "model",
            GroupBy::Day => "day",
        }
    }

    pub fn label_fr(self) -> &'static str {
        match self {
            GroupBy::Project => "projet",
            GroupBy::Ticket => "ticket",
            GroupBy::Agent => "agent",
            GroupBy::Role => "rôle",
            GroupBy::Model => "modèle",
            GroupBy::Day => "jour",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.as_str() == s)
    }
}

/// Half-open time range `[since, until)`; `None` means unbounded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct TimeRange {
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub since: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub until: Option<OffsetDateTime>,
}

impl TimeRange {
    pub fn all() -> Self {
        Self::default()
    }

    pub fn last_days(n: i64) -> Self {
        TimeRange {
            since: Some(crate::now() - time::Duration::days(n)),
            until: None,
        }
    }

    pub fn contains(&self, ts: OffsetDateTime) -> bool {
        self.since.is_none_or(|s| ts >= s) && self.until.is_none_or(|u| ts < u)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageQuery {
    pub group_by: Vec<GroupBy>,
    pub range: TimeRange,
    pub project_id: Option<ProjectId>,
    pub ticket_id: Option<TicketId>,
    pub agent_id: Option<AgentId>,
    /// Include sessions Orchestra did not start (the user's own `claude` runs).
    pub include_unmanaged: bool,
    pub limit: u32,
}

impl Default for UsageQuery {
    fn default() -> Self {
        UsageQuery {
            group_by: vec![GroupBy::Project],
            range: TimeRange::all(),
            project_id: None,
            ticket_id: None,
            agent_id: None,
            include_unmanaged: true,
            limit: 200,
        }
    }
}

/// One aggregated line. `keys` holds the group values in the order requested,
/// e.g. `{"project": "orchestra-ide", "model": "claude-opus-5"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageRow {
    pub keys: BTreeMap<String, String>,
    pub tokens: Tokens,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    pub messages: u64,
    /// True when at least one model in the row used the fallback price.
    #[serde(default)]
    pub cost_estimated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageTotals {
    pub tokens: Tokens,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    pub messages: u64,
    /// Sum of `result.total_cost_usd` reported by Claude Code itself, when
    /// available. Lets the UI show the drift against our price table.
    #[serde(default)]
    pub reported_cost_usd: Option<f64>,
}

/// Helper for clients: a request id generator.
#[derive(Debug, Default)]
pub struct RequestIds(u64);

impl RequestIds {
    pub fn next_id(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }
}

/// Identifies a Claude Code session, managed or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRef {
    pub session_id: Uuid,
    #[serde(default)]
    pub agent_id: Option<AgentId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventKind, NewEvent};

    fn line<T: Serialize>(v: &T) -> String {
        let s = serde_json::to_string(v).unwrap();
        assert!(!s.contains('\n'), "les trames doivent tenir sur une ligne");
        s
    }

    #[test]
    fn requests_round_trip_on_one_line() {
        let cmds = vec![
            Command::Ping,
            Command::ListProjects,
            Command::AddProject {
                path: PathBuf::from("/tmp/p"),
                name: Some("p".into()),
            },
            Command::CreateTicket {
                project_id: Uuid::new_v4(),
                title: "t".into(),
                brief: "brief\navec\nsauts".into(),
            },
            Command::GetDiff {
                ticket_id: Uuid::new_v4(),
            },
            Command::SteerAgent {
                agent_id: Uuid::new_v4(),
                text: "vas-y".into(),
                hard: true,
            },
            Command::Subscribe {
                filter: EventFilter::board(),
                since_seq: Some(42),
                backlog: 100,
            },
            Command::GetUsage {
                query: UsageQuery::default(),
            },
            Command::Status,
            Command::ListAgents { only_active: true },
            Command::ListTodos,
            Command::CreateTodo {
                title: "acheter du café".into(),
                notes: "pour le bureau".into(),
                urgent: true,
                due_at: Some(crate::now()),
            },
            Command::UpdateTodo {
                todo_id: Uuid::new_v4(),
                title: "t".into(),
                notes: "n".into(),
                urgent: false,
                due_at: None,
            },
            Command::SetTodoStatus {
                todo_id: Uuid::new_v4(),
                status: TodoStatus::Done,
            },
            Command::DeleteTodo {
                todo_id: Uuid::new_v4(),
            },
            Command::PromoteTodo {
                todo_id: Uuid::new_v4(),
                project_id: Uuid::new_v4(),
                title: "t".into(),
                brief: "brief assez long pour passer la validation".into(),
            },
            Command::ListRules { project_id: None },
            Command::CreateRule {
                project_id: Some(Uuid::new_v4()),
                rule_kind: RuleKind::Adr,
                title: "t".into(),
            },
            Command::SetRuleStatus {
                project_id: None,
                rule_kind: RuleKind::Convention,
                name: "commits".into(),
                status: RuleStatus::Rejected,
            },
            Command::DeleteRule {
                project_id: None,
                rule_kind: RuleKind::Convention,
                name: "commits".into(),
            },
            Command::CreateRole {
                project_id: None,
                name: "data".into(),
            },
            Command::SetRoleGit {
                project_id: Some(Uuid::new_v4()),
                name: "backend".into(),
                git: crate::guard::GitPolicy::Full,
            },
            Command::DeleteRole {
                project_id: None,
                name: "data".into(),
            },
            Command::PromoteRole {
                project_id: Uuid::new_v4(),
                name: "data".into(),
            },
            Command::PromoteRule {
                project_id: Uuid::new_v4(),
                name: "commits".into(),
            },
        ];
        for cmd in cmds {
            let req = Request { id: 7, cmd };
            let s = line(&req);
            assert_eq!(serde_json::from_str::<Request>(&s).unwrap(), req);
        }
    }

    #[test]
    fn command_tag_is_flattened_into_the_request() {
        let req = Request {
            id: 1,
            cmd: Command::Ping,
        };
        let v: serde_json::Value = serde_json::from_str(&line(&req)).unwrap();
        assert_eq!(v["id"], 1);
        assert_eq!(v["cmd"], "ping");
    }

    #[test]
    fn responses_carry_ok_or_err() {
        let ok = Response {
            id: 1,
            result: Ok(Reply::Pong).into(),
        };
        let v: serde_json::Value = serde_json::from_str(&line(&ok)).unwrap();
        assert_eq!(v["outcome"], "ok");
        assert_eq!(v["reply"], "pong");
        assert_eq!(serde_json::from_str::<Response>(&line(&ok)).unwrap(), ok);

        let err = Response {
            id: 2,
            result: Err(ApiError::not_found("ticket")).into(),
        };
        let back: Response = serde_json::from_str(&line(&err)).unwrap();
        let e = back.result.into_result().unwrap_err();
        assert_eq!(e.code, ErrorCode::NotFound);
        assert!(e.message.contains("introuvable"));
    }

    #[test]
    fn every_reply_survives_a_response_round_trip() {
        // `Reply` is flattened into `Response`, so a field colliding with the
        // envelope's own tag silently breaks that variant. This caught
        // `Reply::Status { status }` against a `status` tag.
        let now = crate::now();
        let replies = vec![
            Reply::Pong,
            Reply::Ack,
            Reply::Projects { projects: vec![] },
            Reply::Tickets { tickets: vec![] },
            Reply::Diff {
                diff: Box::new(TicketDiff {
                    branch: "orch/1-x".into(),
                    base: "main".into(),
                    files: vec![DiffFile { path: "a.rs".into(), added: Some(3), removed: None }],
                    patch: "+ligne\n".into(),
                    truncated: false,
                }),
            },
            Reply::Todos { todos: vec![] },
            Reply::Roles {
                roles: vec![],
                errors: vec!["casse.md : pas d'entête".into()],
            },
            Reply::RoleFile {
                path: PathBuf::from("/roles/data.md"),
            },
            Reply::Rules {
                rules: vec![],
                errors: vec!["x".into()],
            },
            Reply::RuleFile {
                path: PathBuf::from("/x.md"),
            },
            Reply::Agents { agents: vec![] },
            Reply::Usage {
                rows: vec![],
                totals: UsageTotals::default(),
            },
            Reply::Pane {
                pane_id: "terminal_3".into(),
            },
            Reply::Subscribed { current_seq: 12 },
            Reply::Hook {
                allow: false,
                reason: Some("hors worktree".into()),
            },
            Reply::Status {
                status: Box::new(DaemonStatus {
                    version: "0.1.0".into(),
                    protocol: PROTOCOL_VERSION,
                    started_at: now,
                    projects: 1,
                    tickets_running: 2,
                    agents_running: 3,
                    last_seq: 4,
                    watched_files: 5,
                    claude_version: None,
                }),
            },
        ];
        for reply in replies {
            let resp = Response {
                id: 1,
                result: Ok(reply.clone()).into(),
            };
            let s = line(&resp);
            let back: Response = serde_json::from_str(&s)
                .unwrap_or_else(|e| panic!("réponse illisible pour {reply:?} : {e}\n{s}"));
            assert_eq!(back.result.into_result().unwrap(), reply);
        }
    }

    #[test]
    fn frames_distinguish_hello_response_and_event() {
        let hello = Frame::Hello {
            version: "0.1.0".into(),
            protocol: PROTOCOL_VERSION,
            daemon_started: crate::now(),
        };
        assert_eq!(serde_json::from_str::<Frame>(&line(&hello)).unwrap(), hello);

        let event = Frame::Event(Box::new(Event::from_new(
            9,
            NewEvent::new(EventKind::Warning {
                message: "attention".into(),
            }),
        )));
        let v: serde_json::Value = serde_json::from_str(&line(&event)).unwrap();
        assert_eq!(v["type"], "event");
        assert_eq!(serde_json::from_str::<Frame>(&line(&event)).unwrap(), event);
    }

    #[test]
    fn usage_query_defaults_are_sane() {
        let q = UsageQuery::default();
        assert_eq!(q.group_by, vec![GroupBy::Project]);
        assert!(q.include_unmanaged);
        assert_eq!(serde_json::from_str::<UsageQuery>("{}").unwrap(), q);
    }

    #[test]
    fn group_by_round_trips() {
        for g in GroupBy::ALL {
            assert_eq!(GroupBy::parse(g.as_str()), Some(g));
        }
        assert_eq!(GroupBy::parse("nope"), None);
    }

    #[test]
    fn time_range_is_half_open() {
        let now = crate::now();
        let r = TimeRange {
            since: Some(now),
            until: Some(now + time::Duration::hours(1)),
        };
        assert!(r.contains(now));
        assert!(!r.contains(now - time::Duration::seconds(1)));
        assert!(!r.contains(now + time::Duration::hours(1)));
        assert!(TimeRange::all().contains(now));
        assert!(TimeRange::last_days(7).contains(now));
    }

    #[test]
    fn request_ids_increase() {
        let mut ids = RequestIds::default();
        assert_eq!(ids.next_id(), 1);
        assert_eq!(ids.next_id(), 2);
    }
}
