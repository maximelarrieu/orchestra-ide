//! Application state, Elm style: `App` is pure data, `update` folds messages
//! into it, and rendering only reads it. No ratatui type appears here, so the
//! whole state machine is testable without a terminal.
//!
//! One `App`, its methods spread by concern: `input` (the key cascade and the
//! text fields), `nav` (screens, lanes, rows, the « à toi » queue), one file
//! per screen for its own keys (`board`, `ticket`, `agent`, `proposal`,
//! `todo_rules`, `cost`), `palette` (the `:` line), `replies` (what the
//! daemon sends back, and the reads that ask for it) and `describe` (events
//! in words). This file keeps the state, the selectors and `update`.

use orchestra_core::conventions::{Rule, RuleKind, RuleStatus};
use orchestra_core::events::{Event, EventKind};
use orchestra_core::guard::GitPolicy;
use orchestra_core::model::{
    ProjectId, RoleDefinition, TicketId, Todo, TodoDigest, TodoId, TodoStatus,
};
use orchestra_core::protocol::{
    Command, GroupBy, Reply, TicketDetail, TicketSummary, TimeRange, UsageQuery, UsageRow,
    UsageTotals,
};

use crate::forms::{NewTicketForm, TeamEditor, TicketField};
use crate::keymap::Action;
use crate::widgets::LiveLog;

mod agent;
mod board;
mod cost;
mod describe;
mod input;
mod nav;
mod palette;
mod proposal;
mod replies;
mod ticket;
mod todo_rules;
#[cfg(test)]
mod tests;

pub use describe::{agent_label, describe};
use describe::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Board,
    Ticket,
    Agent,
    Cost,
    NewTicket,
    Proposal,
    Todo,
    Rules,
}

impl Screen {
    pub const ALL: [Screen; 8] = [
        Screen::Board,
        Screen::Ticket,
        Screen::Agent,
        Screen::Cost,
        Screen::NewTicket,
        Screen::Proposal,
        Screen::Todo,
        Screen::Rules,
    ];

    pub fn from_number(n: u8) -> Option<Self> {
        Self::ALL.get(n.checked_sub(1)? as usize).copied()
    }

    pub fn title_fr(self) -> &'static str {
        match self {
            Screen::Board => "Tableau",
            Screen::Ticket => "Ticket",
            Screen::Agent => "Agent",
            Screen::Cost => "Coût",
            Screen::NewTicket => "Nouveau ticket",
            Screen::Proposal => "Équipe",
            Screen::Todo => "TODO",
            Screen::Rules => "Rôles & règles",
        }
    }

    /// Every screen exists now; the method stays for the next one that does not.
    pub fn is_implemented(self) -> bool {
        true
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// A destructive action waiting for a yes.
///
/// Cancelling is one keystroke away from ordinary navigation, and `x` happens
/// to sit inside the word someone types out of reflex to leave a program.
/// Nothing irreversible goes through without an answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Confirm {
    pub question: String,
    pub command: Command,
}

/// Which pane of the board has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardPane {
    Projects,
    Tickets,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    Key(Action),
    Event(Box<Event>),
    Reply(Box<Reply>),
    /// The socket went away.
    Disconnected,
    Reconnected(String),
    Tick,
}

/// A column of the board.
///
/// Statuses are the daemon's vocabulary; these are the user's. « À relire » and
/// « PR à valider » are the same status seen from two places: what waits for
/// your eyes here, and what waits for your click on GitHub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Todo,
    Running,
    Review,
    PullRequest,
    Merged,
    Stopped,
}

impl Lane {
    /// Left to right, the way work moves.
    pub const ALL: [Lane; 6] = [
        Lane::Todo,
        Lane::Running,
        Lane::Review,
        Lane::PullRequest,
        Lane::Merged,
        Lane::Stopped,
    ];

    pub fn title_fr(self) -> &'static str {
        match self {
            Lane::Todo => "À faire",
            Lane::Running => "En cours",
            Lane::Review => "À relire",
            Lane::PullRequest => "PR à valider",
            Lane::Merged => "Fusionné",
            Lane::Stopped => "Arrêtés",
        }
    }

    /// Columns that are always there, even empty: they are the shape of the
    /// workflow. The other two only show up when they hold something, so a
    /// project that never opens a request does not carry an empty column.
    pub fn always_shown(self) -> bool {
        !matches!(self, Lane::PullRequest | Lane::Stopped)
    }

    pub fn of(t: &TicketSummary) -> Lane {
        use orchestra_core::model::TicketStatus::*;
        match t.ticket.status {
            Draft | Planned => Lane::Todo,
            Running => Lane::Running,
            Review if t.pull_request.is_some() => Lane::PullRequest,
            Review => Lane::Review,
            Done => Lane::Merged,
            Failed | Cancelled => Lane::Stopped,
        }
    }
}

/// A project as the board shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectRow {
    pub id: ProjectId,
    pub name: String,
    pub path: String,
    pub discovered: bool,
}

/// Time window of the cost screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Today,
    Week,
    Month,
    All,
}

impl Period {
    pub const ALL: [Period; 4] = [Period::Today, Period::Week, Period::Month, Period::All];

    pub fn label_fr(self) -> &'static str {
        match self {
            Period::Today => "aujourd'hui",
            Period::Week => "7 jours",
            Period::Month => "30 jours",
            Period::All => "tout",
        }
    }

    pub fn range(self) -> TimeRange {
        match self {
            // Since midnight, not "24 hours ago": that is what a person means
            // by today when they glance at a dashboard.
            Period::Today => TimeRange {
                since: Some(start_of_today()),
                until: None,
            },
            Period::Week => TimeRange::last_days(7),
            Period::Month => TimeRange::last_days(30),
            Period::All => TimeRange::all(),
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// Midnight UTC of the current day.
fn start_of_today() -> time::OffsetDateTime {
    let now = orchestra_core::now();
    now.replace_time(time::Time::MIDNIGHT)
}

/// `AAAA-MM-JJ`, what `:todo due` takes.
fn parse_due_date(s: &str) -> Option<time::OffsetDateTime> {
    let mut parts = s.trim().splitn(3, '-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    let month = time::Month::try_from(month).ok()?;
    let date = time::Date::from_calendar_date(year, month, day).ok()?;
    Some(date.midnight().assume_utc())
}

#[derive(Debug, Clone)]
pub struct CostView {
    pub rows: Vec<UsageRow>,
    pub totals: UsageTotals,
    pub selected: usize,
    pub group: GroupBy,
    pub period: Period,
    pub include_unmanaged: bool,
    /// Daily cost, oldest first, for the trend line.
    pub daily: Vec<(String, f64)>,
}

impl Default for CostView {
    fn default() -> Self {
        CostView {
            rows: Vec::new(),
            totals: UsageTotals::default(),
            selected: 0,
            group: GroupBy::Project,
            period: Period::Week,
            include_unmanaged: true,
            daily: Vec::new(),
        }
    }
}

impl CostView {
    /// The two queries the screen needs: the grouped table and the trend.
    pub fn queries(&self) -> [UsageQuery; 2] {
        let base = UsageQuery {
            range: self.period.range(),
            include_unmanaged: self.include_unmanaged,
            ..Default::default()
        };
        [
            UsageQuery {
                group_by: vec![self.group],
                ..base.clone()
            },
            UsageQuery {
                group_by: vec![GroupBy::Day],
                limit: 60,
                ..base
            },
        ]
    }

    /// Grouping choices offered by the `m` key.
    pub const GROUPS: [GroupBy; 4] = [
        GroupBy::Project,
        GroupBy::Ticket,
        GroupBy::Role,
        GroupBy::Model,
    ];

    pub fn next_group(&self) -> GroupBy {
        let i = Self::GROUPS
            .iter()
            .position(|g| *g == self.group)
            .unwrap_or(0);
        Self::GROUPS[(i + 1) % Self::GROUPS.len()]
    }
}

pub struct App {
    /// Rows the agent log had at the last frame, kept by the renderer so a
    /// page is a page and not a guess.
    pub log_height: std::cell::Cell<usize>,
    pub screen: Screen,
    /// The ticket currently open, when one is.
    pub ticket: Option<Box<TicketDetail>>,
    /// True while the orchestrator is composing a team.
    pub planning: bool,
    /// When the planning was asked for, so the screen can say how long it has
    /// been going.
    pub planning_since: Option<time::OffsetDateTime>,
    /// What the orchestrator has been doing, newest last. Thinking is long and
    /// silent; without this the screen looks frozen and the user presses keys.
    pub planning_trace: Vec<String>,
    pub form: NewTicketForm,
    pub form_field: TicketField,
    pub editor: TeamEditor,
    /// Index of the agent being watched, within the open ticket.
    pub agent_selected: usize,
    /// True once the user has picked a row himself: the screen then stops
    /// choosing for him.
    pub agent_pinned: bool,
    /// Live log of the watched agent.
    pub log: LiveLog,
    /// Agent to open as soon as its ticket arrives.
    pub pending_agent: Option<orchestra_core::model::AgentId>,
    /// A destructive action waiting for a yes.
    pub confirm: Option<Confirm>,
    /// Text being typed for an agent, if the input is open.
    pub steer: Option<String>,
    /// True when that text will interrupt rather than queue.
    pub steer_hard: bool,
    pub roles: Vec<RoleDefinition>,
    /// Model aliases offered when cycling a member's model.
    pub model_aliases: Vec<String>,
    pub board_pane: BoardPane,
    pub projects: Vec<ProjectRow>,
    pub project_selected: usize,
    /// Whether a project list has ever arrived. The very first one lands the
    /// selection on the first real project rather than on "Tous les
    /// projets", so a single-project setup opens the way it always did;
    /// every later refresh instead keeps wherever the cursor already was,
    /// "Tous les projets" included.
    projects_loaded: bool,
    pub tickets: Vec<TicketSummary>,
    pub ticket_selected: usize,
    pub todos: Vec<Todo>,
    pub todo_selected: usize,
    /// Set while `NewTicket` is filled from a todo rather than from scratch:
    /// submitting promotes it instead of creating an unrelated ticket.
    pub promoting_todo: Option<TodoId>,
    /// Conventions and decisions of the selected project (the global
    /// conventions alone under « Tous les projets »).
    pub rules: Vec<Rule>,
    /// Rule files the daemon could not read, shown on the Rules screen.
    pub rule_errors: Vec<String>,
    /// Role files the daemon could not read, shown on the same screen.
    pub role_errors: Vec<String>,
    /// The selection on the Rules screen, across its roles first and then its
    /// rules: one list, see [`App::selected_role`] and [`App::selected_rule`].
    pub rule_selected: usize,
    /// A file the run loop should open in `$EDITOR`, suspending the screen.
    edit_request: Option<std::path::PathBuf>,
    /// Set while a `CreateRule` is in flight: its file opens once written.
    awaiting_rule_file: bool,
    pub cost: CostView,
    /// Last few events, newest last: the "what is happening" strip.
    pub activity: Vec<String>,
    pub status: String,
    pub connected: bool,
    pub daemon_version: Option<String>,
    pub show_help: bool,
    /// The activity strip folded away (`A`): on a short terminal it takes
    /// rows the screen itself needs.
    pub activity_hidden: bool,
    /// Text being typed in the command palette, if open.
    pub palette: Option<String>,
    pub should_quit: bool,
    /// Commands the update loop wants the caller to send to the daemon.
    pub outbox: Vec<Command>,
    /// Counts the one-second ticks, to pace polling and turn the spinner.
    pub ticks: u64,
    /// When the watched agent last said or did something.
    pub last_activity: Option<time::OffsetDateTime>,
}

const ACTIVITY_MAX: usize = 200;

impl Default for App {
    fn default() -> Self {
        App {
            screen: Screen::Board,
            ticket: None,
            planning: false,
            planning_since: None,
            planning_trace: Vec::new(),
            form: NewTicketForm::default(),
            form_field: TicketField::Title,
            editor: TeamEditor::default(),
            agent_selected: 0,
            agent_pinned: false,
            log: LiveLog::default(),
            pending_agent: None,
            confirm: None,
            steer: None,
            steer_hard: false,
            roles: Vec::new(),
            model_aliases: ["fable", "opus", "sonnet", "haiku"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            board_pane: BoardPane::Projects,
            projects: Vec::new(),
            project_selected: 0,
            projects_loaded: false,
            tickets: Vec::new(),
            ticket_selected: 0,
            todos: Vec::new(),
            todo_selected: 0,
            promoting_todo: None,
            rules: Vec::new(),
            rule_errors: Vec::new(),
            role_errors: Vec::new(),
            rule_selected: 0,
            edit_request: None,
            awaiting_rule_file: false,
            cost: CostView::default(),
            activity: Vec::new(),
            status: "connexion au daemon…".into(),
            connected: false,
            daemon_version: None,
            show_help: false,
            activity_hidden: false,
            palette: None,
            should_quit: false,
            outbox: Vec::new(),
            ticks: 0,
            last_activity: None,
            log_height: std::cell::Cell::new(20),
        }
    }
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    /// The highlighted project, or `None` on "Tous les projets" — index 0,
    /// ahead of every real project — which is also what an empty list gives,
    /// so every caller that already treats "no project" as "nothing to
    /// scope to" keeps working unchanged.
    pub fn selected_project(&self) -> Option<&ProjectRow> {
        self.project_selected
            .checked_sub(1)
            .and_then(|i| self.projects.get(i))
    }

    /// A project's name from its id, for a ticket shown outside its own
    /// project's filter — the board pools every project's tickets under
    /// "Tous les projets", and a bare number does not say which one.
    pub fn project_name(&self, id: ProjectId) -> Option<&str> {
        self.projects
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.as_str())
    }

    /// The board's columns and what they hold, as indices into `tickets`.
    pub fn lanes(&self) -> Vec<(Lane, Vec<usize>)> {
        Lane::ALL
            .iter()
            .filter_map(|lane| {
                let held: Vec<usize> = self
                    .tickets
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| Lane::of(t) == *lane)
                    .map(|(i, _)| i)
                    .collect();
                if held.is_empty() && !lane.always_shown() {
                    return None;
                }
                Some((*lane, held))
            })
            .collect()
    }

    /// Which column holds the selection, and where in it.
    pub fn selected_lane(&self) -> Option<(usize, usize)> {
        let lanes = self.lanes();
        lanes.iter().enumerate().find_map(|(col, (_, held))| {
            held.iter()
                .position(|i| *i == self.ticket_selected)
                .map(|row| (col, row))
        })
    }

    pub fn selected_ticket(&self) -> Option<&TicketSummary> {
        self.tickets.get(self.ticket_selected)
    }

    pub fn selected_ticket_id(&self) -> Option<TicketId> {
        self.selected_ticket().map(|t| t.ticket.id)
    }

    /// The rule under the cursor of the Rules screen, when it is on one.
    pub fn selected_rule(&self) -> Option<&Rule> {
        self.rule_selected
            .checked_sub(self.roles.len())
            .and_then(|i| self.rules.get(i))
    }

    /// The role under the cursor of the Rules screen, when it is on one: the
    /// roles come first on that screen.
    pub fn selected_role(&self) -> Option<&RoleDefinition> {
        self.roles.get(self.rule_selected)
    }

    /// Everything the Rules screen lists: roles, then conventions and ADRs.
    pub fn book_len(&self) -> usize {
        self.roles.len() + self.rules.len()
    }

    /// Rules an agent proposed, or the user drafted, that wait for a decision.
    pub fn pending_rules(&self) -> usize {
        self.rules
            .iter()
            .filter(|r| r.status == RuleStatus::Proposed)
            .count()
    }

    /// The file to open in an editor, if a key or a reply asked for one.
    pub fn take_edit_request(&mut self) -> Option<std::path::PathBuf> {
        self.edit_request.take()
    }

    /// Back from the editor: read the rules again, the file has changed.
    pub fn edited(&mut self, outcome: Result<(), String>) -> Vec<Command> {
        match outcome {
            Ok(()) => self.status = "fichier enregistré".into(),
            Err(e) => self.status = format!("éditeur : {e}"),
        }
        self.request_rules();
        std::mem::take(&mut self.outbox)
    }

    /// The Rules screen's two lists, read together: they share its cursor.
    fn request_rules(&mut self) {
        let project_id = self.selected_project().map(|p| p.id);
        self.outbox.push(Command::ListRoles { project_id });
        self.outbox.push(Command::ListRules { project_id });
    }

    pub fn selected_todo(&self) -> Option<&Todo> {
        self.todos.get(self.todo_selected)
    }

    /// Overdue, due-today and urgent open todos — the morning digest, read
    /// straight from what the board already keeps. The rule itself lives in
    /// `orchestra-core` so the CLI's own notification shares it.
    pub fn todo_digest(&self) -> TodoDigest {
        orchestra_core::model::todo_digest(&self.todos, orchestra_core::now())
    }

    /// The agent the Agent screen is showing.
    pub fn watched_agent(&self) -> Option<&orchestra_core::protocol::AgentSummary> {
        self.ticket.as_ref()?.agents.get(self.agent_selected)
    }

    pub fn watched_agent_id(&self) -> Option<orchestra_core::model::AgentId> {
        self.watched_agent().map(|a| a.agent.id)
    }

    /// Fold one message into the state. Returns commands to send, if any.
    pub fn update(&mut self, msg: Msg) -> Vec<Command> {
        match msg {
            Msg::Key(a) => self.on_key(a),
            Msg::Event(e) => self.on_event(*e),
            Msg::Reply(r) => self.on_reply(*r),
            Msg::Disconnected => {
                self.connected = false;
                self.status = "daemon déconnecté — relance `orchestra daemon`".into();
            }
            Msg::Reconnected(version) => {
                self.connected = true;
                self.daemon_version = Some(version);
                self.status = "connecté".into();
            }
            Msg::Tick => self.on_tick(),
        }
        std::mem::take(&mut self.outbox)
    }

    /// Unmanaged sessions produce no per-response event, on purpose: the
    /// watcher would flood the log. The cost figures are therefore polled,
    /// which is a read of the store like any other.
    fn on_tick(&mut self) {
        self.ticks += 1;
        // What the screen shows is polled every second; the aggregate behind
        // the cost figures is heavier, so it keeps the slower beat. It is
        // polled on every screen, not only on its own: the running total sits
        // in the bar at the bottom, which is drawn everywhere.
        let slow = self.ticks.is_multiple_of(2);
        if slow {
            self.request_usage();
        }
        match self.screen {
            Screen::Board => self.request_tickets(),
            // The log streams, but a status, a cost and an elapsed time come
            // from the ticket: without this the header stays on "démarrage"
            // while the agent is plainly working, and a ticket that changed
            // state is only discovered by leaving the screen and coming back.
            Screen::Ticket | Screen::Agent => self.refresh_open_ticket(None),
            // Rule files are edited by hand too, outside of Orchestra.
            Screen::Rules => self.request_rules(),
            _ => {}
        }
    }

    /// True when a text field has focus, so the caller uses the input keymap.
    pub fn is_typing(&self) -> bool {
        // A confirmation is answered with single keys, not typed text.
        self.palette.is_some()
            || self.screen == Screen::NewTicket
            || self.steer.is_some()
            || (self.screen == Screen::Proposal && self.editor.is_editing())
    }
}
