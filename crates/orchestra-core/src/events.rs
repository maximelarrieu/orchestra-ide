//! The single event type every component writes and every UI reads.
//!
//! Events are persisted to SQLite first (which assigns `seq`), then broadcast.
//! A client can therefore replay a backlog and resume a live stream with no gap
//! by remembering the last `seq` it saw.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::model::{
    AgentId, AgentStatus, ExitReason, ProjectId, TeamProposal, TicketId, TicketStatus, TodoId,
    TodoStatus, UsageSample,
};
use crate::review::Verdict;

/// An event on its way to the store: same as [`Event`] without `seq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewEvent {
    #[serde(with = "time::serde::rfc3339")]
    pub ts: OffsetDateTime,
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    #[serde(default)]
    pub ticket_id: Option<TicketId>,
    #[serde(default)]
    pub agent_id: Option<AgentId>,
    #[serde(default)]
    pub todo_id: Option<TodoId>,
    pub kind: EventKind,
}

impl NewEvent {
    pub fn new(kind: EventKind) -> Self {
        NewEvent {
            ts: crate::now(),
            project_id: None,
            ticket_id: None,
            agent_id: None,
            todo_id: None,
            kind,
        }
    }

    pub fn project(mut self, id: ProjectId) -> Self {
        self.project_id = Some(id);
        self
    }

    pub fn ticket(mut self, id: TicketId) -> Self {
        self.ticket_id = Some(id);
        self
    }

    pub fn agent(mut self, id: AgentId) -> Self {
        self.agent_id = Some(id);
        self
    }

    pub fn todo(mut self, id: TodoId) -> Self {
        self.todo_id = Some(id);
        self
    }

    /// Attach the full scope of an agent in one call.
    pub fn for_agent(
        kind: EventKind,
        project: ProjectId,
        ticket: TicketId,
        agent: AgentId,
    ) -> Self {
        NewEvent::new(kind)
            .project(project)
            .ticket(ticket)
            .agent(agent)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Monotonic sequence number, assigned by the store.
    pub seq: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub ts: OffsetDateTime,
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    #[serde(default)]
    pub ticket_id: Option<TicketId>,
    #[serde(default)]
    pub agent_id: Option<AgentId>,
    #[serde(default)]
    pub todo_id: Option<TodoId>,
    pub kind: EventKind,
}

impl Event {
    pub fn from_new(seq: i64, e: NewEvent) -> Self {
        Event {
            seq,
            ts: e.ts,
            project_id: e.project_id,
            ticket_id: e.ticket_id,
            agent_id: e.agent_id,
            todo_id: e.todo_id,
            kind: e.kind,
        }
    }
}

/// Discriminant stored in the `events.kind` column, so the store can index and
/// filter without deserialising the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventTag {
    ProjectAdded,
    ProjectForgotten,
    TicketCreated,
    TicketStatusChanged,
    ProposalReady,
    ProposalFailed,
    WorktreeCreated,
    AgentSpawned,
    AgentStatusChanged,
    AgentText,
    AgentThinking,
    ToolStarted,
    ToolFinished,
    SubagentStarted,
    SubagentFinished,
    Usage,
    AgentSteered,
    AgentResult,
    TicketResumed,
    ReviewVerdict,
    CheckStarted,
    CheckFinished,
    TicketMerged,
    PullRequestOpened,
    PullRequestClosed,
    HookBlocked,
    PaneOpened,
    PaneClosed,
    UnmanagedSessionSeen,
    DaemonStarted,
    Warning,
    TodoAdded,
    TodoUpdated,
    TodoStatusChanged,
    TodoDeleted,
    TodoPromoted,
}

impl EventTag {
    pub const ALL: [EventTag; 36] = [
        EventTag::ProjectAdded,
        EventTag::ProjectForgotten,
        EventTag::TicketCreated,
        EventTag::TicketStatusChanged,
        EventTag::ProposalReady,
        EventTag::ProposalFailed,
        EventTag::WorktreeCreated,
        EventTag::AgentSpawned,
        EventTag::AgentStatusChanged,
        EventTag::AgentText,
        EventTag::AgentThinking,
        EventTag::ToolStarted,
        EventTag::ToolFinished,
        EventTag::SubagentStarted,
        EventTag::SubagentFinished,
        EventTag::Usage,
        EventTag::AgentSteered,
        EventTag::AgentResult,
        EventTag::TicketResumed,
        EventTag::ReviewVerdict,
        EventTag::CheckStarted,
        EventTag::CheckFinished,
        EventTag::TicketMerged,
        EventTag::PullRequestOpened,
        EventTag::PullRequestClosed,
        EventTag::HookBlocked,
        EventTag::PaneOpened,
        EventTag::PaneClosed,
        EventTag::UnmanagedSessionSeen,
        EventTag::DaemonStarted,
        EventTag::Warning,
        EventTag::TodoAdded,
        EventTag::TodoUpdated,
        EventTag::TodoStatusChanged,
        EventTag::TodoDeleted,
        EventTag::TodoPromoted,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            EventTag::ProjectAdded => "project_added",
            EventTag::ProjectForgotten => "project_forgotten",
            EventTag::TicketCreated => "ticket_created",
            EventTag::TicketStatusChanged => "ticket_status_changed",
            EventTag::ProposalReady => "proposal_ready",
            EventTag::ProposalFailed => "proposal_failed",
            EventTag::WorktreeCreated => "worktree_created",
            EventTag::AgentSpawned => "agent_spawned",
            EventTag::AgentStatusChanged => "agent_status_changed",
            EventTag::AgentText => "agent_text",
            EventTag::AgentThinking => "agent_thinking",
            EventTag::ToolStarted => "tool_started",
            EventTag::ToolFinished => "tool_finished",
            EventTag::SubagentStarted => "subagent_started",
            EventTag::SubagentFinished => "subagent_finished",
            EventTag::Usage => "usage",
            EventTag::AgentSteered => "agent_steered",
            EventTag::AgentResult => "agent_result",
            EventTag::TicketResumed => "ticket_resumed",
            EventTag::ReviewVerdict => "review_verdict",
            EventTag::CheckStarted => "check_started",
            EventTag::CheckFinished => "check_finished",
            EventTag::TicketMerged => "ticket_merged",
            EventTag::PullRequestOpened => "pull_request_opened",
            EventTag::PullRequestClosed => "pull_request_closed",
            EventTag::HookBlocked => "hook_blocked",
            EventTag::PaneOpened => "pane_opened",
            EventTag::PaneClosed => "pane_closed",
            EventTag::UnmanagedSessionSeen => "unmanaged_session_seen",
            EventTag::DaemonStarted => "daemon_started",
            EventTag::Warning => "warning",
            EventTag::TodoAdded => "todo_added",
            EventTag::TodoUpdated => "todo_updated",
            EventTag::TodoStatusChanged => "todo_status_changed",
            EventTag::TodoDeleted => "todo_deleted",
            EventTag::TodoPromoted => "todo_promoted",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }

    /// Chatty per-turn events. The board and cost views skip these.
    pub fn is_verbose(self) -> bool {
        matches!(
            self,
            EventTag::AgentText
                | EventTag::AgentThinking
                | EventTag::ToolStarted
                | EventTag::ToolFinished
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    ProjectAdded {
        name: String,
        path: PathBuf,
    },
    /// A project was removed from Orchestra's list, managed or discovered.
    /// Refused while it still has tickets; tokens already logged stay, now
    /// attached to no project.
    ProjectForgotten {
        name: String,
    },
    TicketCreated {
        number: i64,
        title: String,
    },
    TicketStatusChanged {
        from: TicketStatus,
        to: TicketStatus,
    },
    ProposalReady {
        proposal: Box<TeamProposal>,
    },
    ProposalFailed {
        error: String,
    },
    WorktreeCreated {
        path: PathBuf,
        branch: String,
    },
    AgentSpawned {
        role: String,
        session_id: Uuid,
        pid: u32,
        /// Full command line, so the user can see what runs under the hood.
        cmdline: String,
    },
    AgentStatusChanged {
        status: AgentStatus,
        #[serde(default)]
        reason: Option<ExitReason>,
    },
    /// One assistant text block, already redacted.
    AgentText {
        text: String,
    },
    /// Size of a thinking block. The content is never stored.
    AgentThinking {
        chars: usize,
    },
    ToolStarted {
        tool_use_id: String,
        tool: String,
        /// One-line summary, e.g. `cargo test --workspace`.
        summary: String,
    },
    ToolFinished {
        tool_use_id: String,
        ok: bool,
        summary: String,
    },
    SubagentStarted {
        subagent_id: String,
        name: String,
    },
    SubagentFinished {
        subagent_id: String,
    },
    Usage {
        sample: Box<UsageSample>,
    },
    AgentSteered {
        text: String,
        /// `user` today; room for automated steering later.
        by: String,
        /// Hard redirect (interrupt + resume) rather than a queued message.
        #[serde(default)]
        hard: bool,
    },
    AgentResult {
        subtype: String,
        num_turns: u32,
        duration_ms: u64,
        #[serde(default)]
        total_cost_usd: Option<f64>,
        text: String,
    },
    /// A ticket picked up where it stopped, rather than run again from the
    /// start. `skipped` names the roles whose work was taken as it stands.
    TicketResumed {
        #[serde(default)]
        skipped: Vec<String>,
    },
    /// What the relecture concluded, and who goes back to work.
    ReviewVerdict {
        /// 1 for the first relecture, 2 after one correction round, …
        round: u32,
        verdict: Verdict,
        /// What blocks, one line each. Empty when nothing does.
        #[serde(default)]
        blocking: Vec<String>,
        /// Roles sent back to work by this verdict.
        #[serde(default)]
        roles: Vec<String>,
    },
    /// A verification of the repository is running in the ticket's worktree.
    ///
    /// Published before the command starts: a suite takes minutes, and a
    /// screen that says nothing during them looks like a screen that froze.
    CheckStarted {
        /// 1 for the first pass of the gate, 2 after one repair round, …
        round: u32,
        command: String,
    },
    /// What that verification came back with. This is a measurement, not an
    /// opinion: it is what opens or closes the integration.
    CheckFinished {
        round: u32,
        run: Box<crate::checks::CheckRun>,
    },
    /// The ticket's branch went into the default branch, in fast-forward.
    TicketMerged {
        branch: String,
        into: String,
        /// Commits the branch brought in.
        commits: usize,
        /// The remote the default branch was pushed to, when it was.
        #[serde(default)]
        pushed_to: Option<String>,
    },
    /// A pull request was opened for the ticket's branch.
    PullRequestOpened {
        url: String,
        #[serde(default)]
        number: Option<u64>,
    },
    /// That pull request left the open state, one way or the other.
    PullRequestClosed {
        url: String,
        /// True when it was merged, false when it was closed without merging.
        merged: bool,
    },
    /// The `PreToolUse` guard refused a call.
    HookBlocked {
        tool: String,
        reason: String,
    },
    PaneOpened {
        pane_id: String,
    },
    PaneClosed {
        pane_id: String,
    },
    /// A Claude Code session we did not start showed up in the transcripts.
    UnmanagedSessionSeen {
        session_id: Uuid,
        cwd: PathBuf,
    },
    DaemonStarted {
        version: String,
    },
    Warning {
        message: String,
    },
    TodoAdded {
        title: String,
    },
    TodoUpdated {
        title: String,
    },
    TodoStatusChanged {
        title: String,
        from: TodoStatus,
        to: TodoStatus,
    },
    TodoDeleted {
        title: String,
    },
    /// A todo became a ticket. The ticket's own `TicketCreated` event carries
    /// the rest of the story.
    TodoPromoted {
        title: String,
        ticket_number: i64,
        project_name: String,
    },
}

impl EventKind {
    pub fn tag(&self) -> EventTag {
        match self {
            EventKind::ProjectAdded { .. } => EventTag::ProjectAdded,
            EventKind::ProjectForgotten { .. } => EventTag::ProjectForgotten,
            EventKind::TicketCreated { .. } => EventTag::TicketCreated,
            EventKind::TicketStatusChanged { .. } => EventTag::TicketStatusChanged,
            EventKind::ProposalReady { .. } => EventTag::ProposalReady,
            EventKind::ProposalFailed { .. } => EventTag::ProposalFailed,
            EventKind::WorktreeCreated { .. } => EventTag::WorktreeCreated,
            EventKind::AgentSpawned { .. } => EventTag::AgentSpawned,
            EventKind::AgentStatusChanged { .. } => EventTag::AgentStatusChanged,
            EventKind::AgentText { .. } => EventTag::AgentText,
            EventKind::AgentThinking { .. } => EventTag::AgentThinking,
            EventKind::ToolStarted { .. } => EventTag::ToolStarted,
            EventKind::ToolFinished { .. } => EventTag::ToolFinished,
            EventKind::SubagentStarted { .. } => EventTag::SubagentStarted,
            EventKind::SubagentFinished { .. } => EventTag::SubagentFinished,
            EventKind::Usage { .. } => EventTag::Usage,
            EventKind::AgentSteered { .. } => EventTag::AgentSteered,
            EventKind::AgentResult { .. } => EventTag::AgentResult,
            EventKind::TicketResumed { .. } => EventTag::TicketResumed,
            EventKind::ReviewVerdict { .. } => EventTag::ReviewVerdict,
            EventKind::CheckStarted { .. } => EventTag::CheckStarted,
            EventKind::CheckFinished { .. } => EventTag::CheckFinished,
            EventKind::TicketMerged { .. } => EventTag::TicketMerged,
            EventKind::PullRequestOpened { .. } => EventTag::PullRequestOpened,
            EventKind::PullRequestClosed { .. } => EventTag::PullRequestClosed,
            EventKind::HookBlocked { .. } => EventTag::HookBlocked,
            EventKind::PaneOpened { .. } => EventTag::PaneOpened,
            EventKind::PaneClosed { .. } => EventTag::PaneClosed,
            EventKind::UnmanagedSessionSeen { .. } => EventTag::UnmanagedSessionSeen,
            EventKind::DaemonStarted { .. } => EventTag::DaemonStarted,
            EventKind::Warning { .. } => EventTag::Warning,
            EventKind::TodoAdded { .. } => EventTag::TodoAdded,
            EventKind::TodoUpdated { .. } => EventTag::TodoUpdated,
            EventKind::TodoStatusChanged { .. } => EventTag::TodoStatusChanged,
            EventKind::TodoDeleted { .. } => EventTag::TodoDeleted,
            EventKind::TodoPromoted { .. } => EventTag::TodoPromoted,
        }
    }
}

/// What a subscriber wants to receive. An unset field means "no constraint".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EventFilter {
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    #[serde(default)]
    pub ticket_id: Option<TicketId>,
    #[serde(default)]
    pub agent_id: Option<AgentId>,
    #[serde(default)]
    pub todo_id: Option<TodoId>,
    /// Only these tags. Empty means every tag.
    #[serde(default)]
    pub tags: Vec<EventTag>,
    /// Drop the chatty per-turn events (see [`EventTag::is_verbose`]).
    #[serde(default)]
    pub exclude_verbose: bool,
}

impl EventFilter {
    pub fn all() -> Self {
        Self::default()
    }

    pub fn for_agent(id: AgentId) -> Self {
        EventFilter {
            agent_id: Some(id),
            ..Default::default()
        }
    }

    pub fn for_ticket(id: TicketId) -> Self {
        EventFilter {
            ticket_id: Some(id),
            ..Default::default()
        }
    }

    /// What the board subscribes to: status changes and costs, no chatter.
    pub fn board() -> Self {
        EventFilter {
            exclude_verbose: true,
            ..Default::default()
        }
    }

    pub fn matches(&self, e: &Event) -> bool {
        if let Some(p) = self.project_id {
            if e.project_id != Some(p) {
                return false;
            }
        }
        if let Some(t) = self.ticket_id {
            if e.ticket_id != Some(t) {
                return false;
            }
        }
        if let Some(a) = self.agent_id {
            if e.agent_id != Some(a) {
                return false;
            }
        }
        if let Some(t) = self.todo_id {
            if e.todo_id != Some(t) {
                return false;
            }
        }
        let tag = e.kind.tag();
        if self.exclude_verbose && tag.is_verbose() {
            return false;
        }
        if !self.tags.is_empty() && !self.tags.contains(&tag) {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Size, TeamProposal};

    fn ev(kind: EventKind) -> Event {
        Event::from_new(1, NewEvent::new(kind))
    }

    #[test]
    fn every_tag_round_trips_and_matches_its_kind() {
        for tag in EventTag::ALL {
            assert_eq!(EventTag::parse(tag.as_str()), Some(tag));
        }
        assert_eq!(
            EventKind::Warning {
                message: "x".into()
            }
            .tag(),
            EventTag::Warning
        );
        assert_eq!(
            EventKind::AgentThinking { chars: 3 }.tag(),
            EventTag::AgentThinking
        );
    }

    #[test]
    fn tag_strings_match_the_kind_discriminants() {
        // The store writes `tag()` into a column and the payload separately;
        // both must agree or a filter on the column would miss events.
        let kinds = vec![
            EventKind::AgentText { text: "t".into() },
            EventKind::AgentThinking { chars: 1 },
            EventKind::Warning {
                message: "w".into(),
            },
            EventKind::DaemonStarted {
                version: "v".into(),
            },
            EventKind::TodoStatusChanged {
                title: "t".into(),
                from: crate::model::TodoStatus::Open,
                to: crate::model::TodoStatus::Done,
            },
        ];
        for kind in kinds {
            let v = serde_json::to_value(&kind).unwrap();
            assert_eq!(v["kind"].as_str().unwrap(), kind.tag().as_str());
        }
    }

    #[test]
    fn kind_serialises_with_a_kind_field() {
        let json = serde_json::to_value(EventKind::AgentText { text: "hi".into() }).unwrap();
        assert_eq!(json["kind"], "agent_text");
        assert_eq!(json["text"], "hi");
        let back: EventKind = serde_json::from_value(json).unwrap();
        assert_eq!(back, EventKind::AgentText { text: "hi".into() });
    }

    #[test]
    fn proposal_event_round_trips() {
        let kind = EventKind::ProposalReady {
            proposal: Box::new(TeamProposal {
                summary: "s".into(),
                members: vec![],
                risks: vec!["r".into()],
                estimated_size: Size::L,
            }),
        };
        let s = serde_json::to_string(&kind).unwrap();
        assert_eq!(serde_json::from_str::<EventKind>(&s).unwrap(), kind);
    }

    #[test]
    fn filter_narrows_by_scope_and_tag() {
        let agent = Uuid::new_v4();
        let other = Uuid::new_v4();
        let mut e = ev(EventKind::AgentText { text: "hi".into() });
        e.agent_id = Some(agent);

        assert!(EventFilter::all().matches(&e));
        assert!(EventFilter::for_agent(agent).matches(&e));
        assert!(!EventFilter::for_agent(other).matches(&e));
        // AgentText is verbose, so the board filter drops it.
        assert!(!EventFilter::board().matches(&e));

        let mut only_usage = EventFilter::all();
        only_usage.tags = vec![EventTag::Usage];
        assert!(!only_usage.matches(&e));

        let status = ev(EventKind::AgentStatusChanged {
            status: AgentStatus::Running,
            reason: None,
        });
        assert!(EventFilter::board().matches(&status));
    }

    #[test]
    fn ticket_filter_ignores_other_tickets() {
        let t = Uuid::new_v4();
        let mut e = ev(EventKind::TicketStatusChanged {
            from: TicketStatus::Draft,
            to: TicketStatus::Planned,
        });
        e.ticket_id = Some(t);
        assert!(EventFilter::for_ticket(t).matches(&e));
        assert!(!EventFilter::for_ticket(Uuid::new_v4()).matches(&e));
    }

    #[test]
    fn todo_filter_ignores_other_todos() {
        let t = Uuid::new_v4();
        let mut e = ev(EventKind::TodoStatusChanged {
            title: "x".into(),
            from: crate::model::TodoStatus::Open,
            to: crate::model::TodoStatus::Done,
        });
        e.todo_id = Some(t);
        let filter = EventFilter {
            todo_id: Some(t),
            ..Default::default()
        };
        assert!(filter.matches(&e));
        let other = EventFilter {
            todo_id: Some(Uuid::new_v4()),
            ..Default::default()
        };
        assert!(!other.matches(&e));
    }

    #[test]
    fn builders_attach_scope() {
        let p = Uuid::new_v4();
        let t = Uuid::new_v4();
        let a = Uuid::new_v4();
        let e = NewEvent::for_agent(EventKind::AgentThinking { chars: 1 }, p, t, a);
        assert_eq!(
            (e.project_id, e.ticket_id, e.agent_id),
            (Some(p), Some(t), Some(a))
        );
    }
}
