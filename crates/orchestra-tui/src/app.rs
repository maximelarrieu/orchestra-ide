//! Application state, Elm style: `App` is pure data, `update` folds messages
//! into it, and rendering only reads it. No ratatui type appears here, so the
//! whole state machine is testable without a terminal.

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
            palette: None,
            should_quit: false,
            outbox: Vec::new(),
            ticks: 0,
            last_activity: None,
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

    /// Move one column over, keeping roughly the same height. Past the first
    /// column, the focus goes back to the projects.
    fn move_lane(&mut self, delta: isize) {
        if self.screen != Screen::Board {
            return;
        }
        if self.board_pane == BoardPane::Projects {
            if delta > 0 {
                self.board_pane = BoardPane::Tickets;
            }
            return;
        }
        let lanes = self.lanes();
        let (col, row) = match self.selected_lane() {
            Some(pos) => pos,
            None => {
                // Nothing selected in a column: take the first ticket there is.
                if let Some(first) = lanes.iter().find_map(|(_, held)| held.first()) {
                    self.ticket_selected = *first;
                }
                return;
            }
        };
        // Empty columns are stepped over: there is nothing to land on.
        let mut next = col as isize + delta;
        while next >= 0 && (next as usize) < lanes.len() {
            let held = &lanes[next as usize].1;
            if let Some(i) = held.get(row.min(held.len().saturating_sub(1))) {
                self.ticket_selected = *i;
                return;
            }
            next += delta;
        }
        if next < 0 {
            self.board_pane = BoardPane::Projects;
        }
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

    fn on_key(&mut self, action: Action) {
        // A pending question takes every key until it is answered.
        if self.confirm.is_some() {
            self.on_confirm_key(action);
            return;
        }
        // The palette swallows keys while it is open.
        if let Some(buf) = self.palette.as_mut() {
            match action {
                Action::Char(c) => buf.push(c),
                Action::Backspace => {
                    buf.pop();
                }
                Action::Cancel => self.palette = None,
                Action::Submit => {
                    let line = self.palette.take().unwrap_or_default();
                    self.run_palette(&line);
                }
                Action::Quit => self.should_quit = true,
                _ => {}
            }
            return;
        }
        if self.show_help {
            // Any key closes the help overlay, except quitting outright.
            match action {
                Action::Quit => self.should_quit = true,
                _ => self.show_help = false,
            }
            return;
        }

        if self.screen == Screen::NewTicket {
            self.on_form_key(action);
            return;
        }
        if self.screen == Screen::Proposal && self.editor.is_editing() {
            self.on_objective_key(action);
            return;
        }
        if self.steer.is_some() {
            self.on_steer_key(action);
            return;
        }

        match action {
            Action::Quit => self.should_quit = true,
            Action::Back => self.on_back(),
            Action::Help => self.show_help = true,
            Action::CommandPalette => self.palette = Some(String::new()),
            Action::Refresh => self.refresh(),
            Action::Screen(n) => {
                if let Some(s) = Screen::from_number(n) {
                    self.go(s);
                }
            }
            Action::NextScreen => self.go(self.screen.next()),
            Action::PrevScreen => self.go(self.screen.prev()),
            Action::Left => self.move_lane(-1),
            Action::Right => self.move_lane(1),
            Action::Up => self.move_selection(-1),
            Action::Down => self.move_selection(1),
            Action::Top => self.set_selection(0),
            Action::Bottom => {
                if self.screen == Screen::Agent {
                    self.log.follow();
                } else {
                    self.set_selection(usize::MAX)
                }
            }
            Action::Select => self.open_selection(),
            Action::Char(c) => self.on_char(c),
            _ => {}
        }
    }

    /// Escape and `q` step back one screen rather than quitting outright, so a
    /// half-written ticket is not lost to a reflex.
    fn on_back(&mut self) {
        match self.screen {
            Screen::Board => self.should_quit = true,
            Screen::Ticket => {
                self.ticket = None;
                self.agent_selected = 0;
                self.agent_pinned = false;
                self.screen = Screen::Board;
            }
            Screen::Proposal | Screen::Agent => {
                self.screen = Screen::Ticket;
                // Back to the whole ticket's events.
                self.outbox.push(Command::Subscribe {
                    filter: orchestra_core::events::EventFilter::board(),
                    since_seq: None,
                    backlog: 50,
                });
            }
            _ => self.screen = Screen::Board,
        }
    }

    fn on_form_key(&mut self, action: Action) {
        match action {
            Action::Char(c) => self.form.type_char(self.form_field, c),
            Action::Backspace => self.form.backspace(self.form_field),
            Action::NextField => self.form_field = self.form_field.next(),
            Action::Submit => self.form.newline(self.form_field),
            Action::Accept => self.submit_form(),
            Action::Cancel => {
                self.form.clear();
                self.promoting_todo = None;
                self.screen = Screen::Board;
            }
            Action::Quit => self.should_quit = true,
            _ => {}
        }
    }

    fn submit_form(&mut self) {
        let Some(project) = self.selected_project().map(|p| p.id) else {
            self.form.error = Some("choisis d'abord un projet sur le tableau".into());
            return;
        };
        match self.form.validated() {
            Ok((title, brief)) => {
                match self.promoting_todo.take() {
                    Some(todo_id) => {
                        self.outbox.push(Command::PromoteTodo {
                            todo_id,
                            project_id: project,
                            title,
                            brief,
                        });
                        self.status = "promotion en ticket…".into();
                    }
                    None => {
                        self.outbox.push(Command::CreateTicket {
                            project_id: project,
                            title,
                            brief,
                        });
                        self.status = "ticket créé".into();
                    }
                }
                self.form.clear();
                self.screen = Screen::Board;
            }
            Err(e) => self.form.error = Some(e),
        }
    }

    /// `o`, `y` or Enter confirms; anything else declines.
    fn on_confirm_key(&mut self, action: Action) {
        let accepted = matches!(
            action,
            Action::Char('o') | Action::Char('y') | Action::Select
        );
        let confirm = self.confirm.take();
        match (accepted, confirm) {
            (true, Some(c)) => {
                self.outbox.push(c.command);
                self.status = "c'est parti".into();
            }
            _ => self.status = "annulation abandonnée".into(),
        }
    }

    /// Queue a destructive action behind a question.
    fn ask(&mut self, question: impl Into<String>, command: Command) {
        self.confirm = Some(Confirm {
            question: question.into(),
            command,
        });
    }

    fn on_steer_key(&mut self, action: Action) {
        match action {
            Action::Char(c) => {
                if let Some(buf) = self.steer.as_mut() {
                    buf.push(c);
                }
            }
            Action::Backspace => {
                if let Some(buf) = self.steer.as_mut() {
                    buf.pop();
                }
            }
            Action::Submit | Action::Accept => self.send_steer(),
            Action::Cancel => self.steer = None,
            Action::Quit => self.should_quit = true,
            _ => {}
        }
    }

    fn send_steer(&mut self) {
        let text = self.steer.take().unwrap_or_default().trim().to_string();
        let Some(agent_id) = self.watched_agent_id() else {
            return;
        };
        if text.is_empty() {
            return;
        }
        self.outbox.push(Command::SteerAgent {
            agent_id,
            text,
            hard: self.steer_hard,
        });
        self.status = if self.steer_hard {
            "redirection envoyée".into()
        } else {
            "consigne envoyée".into()
        };
    }

    fn on_objective_key(&mut self, action: Action) {
        match action {
            Action::Char(c) => self.editor.type_char(c),
            Action::Backspace => self.editor.backspace(),
            Action::Submit | Action::Accept => self.editor.finish_editing(true),
            Action::Cancel => self.editor.finish_editing(false),
            Action::Quit => self.should_quit = true,
            _ => {}
        }
    }

    fn on_char(&mut self, c: char) {
        match self.screen {
            Screen::Cost => self.on_cost_char(c),
            Screen::Board => self.on_board_char(c),
            Screen::Ticket => self.on_ticket_char(c),
            Screen::Agent => self.on_agent_char(c),
            Screen::Proposal => self.on_proposal_char(c),
            Screen::Todo => self.on_todo_char(c),
            Screen::Rules => self.on_rules_char(c),
            _ => {}
        }
    }

    fn on_todo_char(&mut self, c: char) {
        match c {
            'd' => self.delete_selected_todo(),
            'p' => self.promote_selected_todo(),
            _ => {}
        }
    }

    fn on_rules_char(&mut self, c: char) {
        if self.selected_role().is_some() {
            self.on_role_char(c);
            return;
        }
        let Some(r) = self.selected_rule() else {
            self.status = "aucune règle sélectionnée".into();
            return;
        };
        let project_id = self.selected_project().map(|p| p.id);
        let (kind, name, title) = (r.kind, r.name.clone(), r.title.clone());
        match c {
            'a' => self.outbox.push(Command::SetRuleStatus {
                project_id,
                rule_kind: kind,
                name,
                status: RuleStatus::Accepted,
            }),
            'r' => self.ask(
                format!("Rejeter la {} « {title} » ?", kind.label_fr()),
                Command::SetRuleStatus {
                    project_id,
                    rule_kind: kind,
                    name,
                    status: RuleStatus::Rejected,
                },
            ),
            // Only a decision is superseded; a convention is simply rejected.
            's' if kind == RuleKind::Adr => self.outbox.push(Command::SetRuleStatus {
                project_id,
                rule_kind: kind,
                name,
                status: RuleStatus::Superseded,
            }),
            'e' => {
                if r.source.exists() {
                    self.edit_request = Some(r.source.clone());
                } else {
                    self.status =
                        "convention livrée, pas encore installée : lance « orchestra init »".into();
                }
            }
            'g' => match (project_id, r.scope) {
                (Some(project_id), orchestra_core::model::RoleScope::Project)
                    if kind == RuleKind::Convention =>
                {
                    self.ask(
                        format!("Rendre « {title} » globale, pour tous les projets ?"),
                        Command::PromoteRule { project_id, name },
                    )
                }
                _ => self.status = "seule une convention de projet peut devenir globale".into(),
            },
            'd' => self.ask(
                format!("Supprimer la {} « {title} » ?", kind.label_fr()),
                Command::DeleteRule {
                    project_id,
                    rule_kind: kind,
                    name,
                },
            ),
            _ => {}
        }
    }

    /// A role is a file: edited by hand, its git opened or closed, moved to
    /// the global catalog, deleted. Created from the palette (`:role add`).
    fn on_role_char(&mut self, c: char) {
        let Some(r) = self.selected_role() else {
            return;
        };
        let project_id = self.selected_project().map(|p| p.id);
        let (name, scope, source) = (r.name.clone(), r.scope, r.source.clone());
        let git = r.git.unwrap_or_default();
        match c {
            'e' => self.edit_request = Some(source),
            // Opening git is outward-facing — a push leaves the machine — so it
            // is asked; closing it takes nothing away that cannot be given back.
            'p' => match git {
                GitPolicy::Confined => self.ask(
                    format!(
                        "Donner git complet (push, merge, rebase) au rôle « {name} » ? \
                         Il restera dans son worktree."
                    ),
                    Command::SetRoleGit {
                        project_id,
                        name,
                        git: GitPolicy::Full,
                    },
                ),
                GitPolicy::Full => self.outbox.push(Command::SetRoleGit {
                    project_id,
                    name,
                    git: GitPolicy::Confined,
                }),
            },
            'g' => match (project_id, scope) {
                (Some(project_id), orchestra_core::model::RoleScope::Project) => self.ask(
                    format!("Rendre le rôle « {name} » global, pour tous les projets ?"),
                    Command::PromoteRole { project_id, name },
                ),
                _ => self.status = "seul un rôle de projet peut devenir global".into(),
            },
            'd' => self.ask(
                match scope {
                    orchestra_core::model::RoleScope::Project => {
                        format!("Supprimer la version du projet du rôle « {name} » ?")
                    }
                    orchestra_core::model::RoleScope::Global => {
                        format!("Supprimer le rôle « {name} », pour tous les projets ?")
                    }
                },
                Command::DeleteRole { project_id, name },
            ),
            _ => {}
        }
    }

    fn delete_selected_todo(&mut self) {
        let Some(t) = self.selected_todo() else {
            self.status = "aucun todo à supprimer".into();
            return;
        };
        self.ask(
            format!("Supprimer le todo « {} » ?", t.title),
            Command::DeleteTodo { todo_id: t.id },
        );
    }

    /// Seed the new-ticket form from the selected todo and remember it, so
    /// submitting promotes it instead of creating an unrelated ticket.
    fn promote_selected_todo(&mut self) {
        if self.selected_project().is_none() {
            self.status = "choisis d'abord un projet sur le tableau".into();
            return;
        }
        let Some(t) = self.selected_todo() else {
            self.status = "aucun todo à promouvoir".into();
            return;
        };
        let (id, title, notes) = (t.id, t.title.clone(), t.notes.clone());
        self.form.seed(&title, &notes);
        self.form_field = TicketField::Title;
        self.promoting_todo = Some(id);
        self.screen = Screen::NewTicket;
    }

    fn on_board_char(&mut self, c: char) {
        match c {
            'n' => self.open_new_ticket(),
            'd' if self.board_pane == BoardPane::Projects => self.forget_selected_project(),
            _ => {}
        }
    }

    /// Ask to remove the highlighted project. The daemon refuses while it
    /// still holds tickets, and that refusal comes back as an ordinary
    /// status line rather than something checked here.
    fn forget_selected_project(&mut self) {
        let Some(p) = self.selected_project() else {
            self.status = "aucun projet à oublier".into();
            return;
        };
        self.ask(
            format!("Oublier le projet « {} » ?", p.name),
            Command::ForgetProject { project_id: p.id },
        );
    }

    /// A project by id, or by a fragment of its name or path: what
    /// `:project forget` takes, since a name is what someone types.
    fn find_project(&self, spec: &str) -> Option<&ProjectRow> {
        let needle = spec.to_lowercase();
        self.projects.iter().find(|p| {
            p.id.to_string() == spec
                || p.name.to_lowercase().contains(&needle)
                || p.path.to_lowercase().contains(&needle)
        })
    }

    fn open_new_ticket(&mut self) {
        if self.selected_project().is_none() {
            self.status = if self.projects.is_empty() {
                "ajoute d'abord un projet : `:project add <chemin>`".into()
            } else {
                "choisis un projet plutôt que « Tous les projets »".into()
            };
            return;
        }
        self.form.clear();
        self.form_field = TicketField::Title;
        self.promoting_todo = None;
        self.screen = Screen::NewTicket;
    }

    fn on_ticket_char(&mut self, c: char) {
        let Some(detail) = self.ticket.as_ref() else {
            return;
        };
        let ticket_id = detail.ticket.id;
        match c {
            'p' => self.start_planning(ticket_id),
            'a' => self.open_editor(),
            'n' => self.open_new_ticket(),
            'L' => {
                self.outbox.push(Command::LaunchTicket {
                    ticket_id,
                    open_panes: false,
                });
                self.status = "lancement de l'équipe…".into();
            }
            'x' => {
                let number = detail.ticket.number;
                self.ask(
                    format!("Arrêter le ticket #{number} et ses agents ?"),
                    Command::CancelTicket { ticket_id },
                );
            }
            // Only offered on a branch the relecture cleared; the daemon
            // refuses it in every other case anyway.
            'f' if self.can_integrate() => {
                let branch = detail.ticket.branch.clone().unwrap_or_default();
                let into = detail.project.default_branch.clone();
                let question = match detail.integration_mode {
                    _ if detail.merge_blocked.is_some() => format!(
                        "Réessayer la fusion de « {branch} » dans « {into} » ? Sans agent si \
                         la branche n'a pas bougé depuis."
                    ),
                    orchestra_core::config::IntegrationMode::Pr => {
                        format!("Pousser « {branch} » et ouvrir une pull request vers « {into} » ?")
                    }
                    _ => format!("Intégrer « {branch} » dans « {into} » et terminer le ticket ?"),
                };
                self.ask(question, Command::IntegrateTicket { ticket_id });
            }
            // A ticket closed one way, to be closed another: the branch and the
            // verdict are still there, only the decision is taken back.
            'o' if detail.ticket.status.is_terminal() => {
                let number = detail.ticket.number;
                self.ask(
                    format!("Rouvrir le ticket #{number} ? Il repasse « à relire »."),
                    Command::ReopenTicket { ticket_id },
                );
            }
            't' if detail.ticket.status == orchestra_core::model::TicketStatus::Review => {
                let number = detail.ticket.number;
                self.ask(
                    format!("Marquer le ticket #{number} terminé, sans rien fusionner ?"),
                    Command::FinishTicket { ticket_id },
                );
            }
            _ => {}
        }
    }

    /// True when the open ticket can be handed to the integrator: relu, rien
    /// ne bloque, une branche à livrer, et rien déjà en vol.
    pub fn can_integrate(&self) -> bool {
        let Some(detail) = self.ticket.as_ref() else {
            return false;
        };
        detail.ticket.status == orchestra_core::model::TicketStatus::Review
            && detail.ticket.branch.is_some()
            && detail.review.as_ref().is_some_and(|r| r.is_ready())
            // A green relecture on a red build integrates nothing: the daemon
            // refuses it too, and offering the key would only produce an
            // error the user did not ask for.
            && detail.checks.as_ref().is_none_or(|c| c.passed())
            // A request is already waiting for its human: pressing the key
            // again would only reopen the same one.
            && detail.pull_request.is_none()
    }

    /// What the integration key does on this project, in the user's words.
    pub fn integration_label(&self) -> &'static str {
        if self.ticket.as_ref().is_some_and(|d| d.merge_blocked.is_some()) {
            return "réessayer la fusion";
        }
        match self.ticket.as_ref().map(|d| d.integration_mode) {
            Some(orchestra_core::config::IntegrationMode::Pr) => "ouvrir la pull request",
            _ => "intégrer et terminer",
        }
    }

    /// Ask for a team, and start following what the orchestrator does.
    ///
    /// Planning is one long silence: the run reads files for half a minute
    /// before answering. So the screen subscribes to the ticket's own events —
    /// verbose ones included — and shows them while it waits.
    fn start_planning(&mut self, ticket_id: TicketId) {
        self.outbox.push(Command::PlanTicket { ticket_id });
        self.planning = true;
        self.planning_since = Some(orchestra_core::now());
        self.planning_trace.clear();
        self.outbox.push(Command::Subscribe {
            filter: orchestra_core::events::EventFilter::for_ticket(ticket_id),
            since_seq: None,
            backlog: 0,
        });
        self.status = "l'orchestrateur compose l'équipe…".into();
    }

    /// Back from a single agent's stream to the wider one: the planned
    /// ticket's while the orchestrator works, the board's otherwise.
    fn subscribe_wide(&mut self) {
        let filter = match (self.planning, self.ticket.as_ref()) {
            (true, Some(detail)) => orchestra_core::events::EventFilter::for_ticket(detail.ticket.id),
            _ => orchestra_core::events::EventFilter::board(),
        };
        self.outbox.push(Command::Subscribe {
            filter,
            since_seq: None,
            backlog: 0,
        });
    }

    /// Planning is over, whatever its outcome: back to the board's stream.
    fn stop_planning(&mut self) {
        if !self.planning {
            return;
        }
        self.planning = false;
        self.planning_since = None;
        // An agent being watched has its own subscription; leave it alone.
        if self.screen != Screen::Agent {
            self.outbox.push(Command::Subscribe {
                filter: orchestra_core::events::EventFilter::board(),
                since_seq: None,
                backlog: 0,
            });
        }
    }

    /// Fold one event into the planning trace.
    fn trace_planning(&mut self, e: &Event) {
        if !self.planning || e.ticket_id.is_none() {
            return;
        }
        if e.ticket_id != self.ticket.as_ref().map(|d| d.ticket.id) {
            return;
        }
        if let Some(line) = plan_line(e) {
            if self.planning_trace.len() == PLAN_TRACE_MAX {
                self.planning_trace.remove(0);
            }
            self.planning_trace.push(line);
        }
    }

    /// The orchestrator run of the open ticket, most recent first.
    pub fn orchestrator_agent(&self) -> Option<&orchestra_core::protocol::AgentSummary> {
        self.ticket
            .as_ref()?
            .agents
            .iter()
            .filter(|a| a.agent.role == orchestra_core::model::ORCHESTRATOR_ROLE)
            .max_by_key(|a| a.agent.started_at)
    }

    /// Watch an agent of the open ticket.
    ///
    /// Opening a ticket lands on the agent that is working, not on the first
    /// row: the orchestrator sorts before the team and watching it while the
    /// real agent runs looks exactly like a frozen screen.
    fn open_agent(&mut self) {
        self.select_liveliest_agent();
        let Some(agent_id) = self.watched_agent_id() else {
            self.status = "aucun agent sur ce ticket".into();
            return;
        };
        self.log.clear();
        self.screen = Screen::Agent;
        // The backlog of this agent arrives through the event stream.
        self.outbox.push(Command::Subscribe {
            filter: orchestra_core::events::EventFilter::for_agent(agent_id),
            since_seq: None,
            backlog: 500,
        });
    }

    /// Prefer a running agent, then the most recently started one.
    fn select_liveliest_agent(&mut self) {
        let Some(detail) = self.ticket.as_ref() else {
            return;
        };
        if self
            .watched_agent()
            .is_some_and(|a| a.agent.status.is_active())
        {
            return;
        }
        let running = detail
            .agents
            .iter()
            .position(|a| a.agent.status.is_active());
        // Nothing is working: there is no team to follow, so the row the user
        // chose himself wins. Without this, every row of a finished ticket
        // opened the last agent that ran.
        if self.agent_pinned && running.is_none() {
            return;
        }
        let fallback = || {
            detail
                .agents
                .iter()
                .enumerate()
                .filter(|(_, a)| a.agent.role != orchestra_core::model::ORCHESTRATOR_ROLE)
                .max_by_key(|(_, a)| a.agent.started_at)
                .map(|(i, _)| i)
                .or(detail.agents.len().checked_sub(1))
        };
        if let Some(index) = running.or_else(fallback) {
            self.agent_selected = index;
        }
    }

    fn on_agent_char(&mut self, c: char) {
        let Some(agent) = self.watched_agent() else {
            return;
        };
        let active = agent.agent.status.is_active();
        let agent_id = agent.agent.id;
        match c {
            'o' => {
                self.outbox.push(Command::OpenPane { agent_id });
                self.status = "ouverture du pane…".into();
            }
            // Only once it has stopped: two hands on one Claude session would
            // each undo the other's turn, and the daemon refuses it anyway.
            'T' if !active => {
                self.outbox.push(Command::TakeOver { agent_id });
                self.status = "reprise en main…".into();
            }
            's' if active => {
                self.steer = Some(String::new());
                self.steer_hard = false;
            }
            'S' if active => {
                self.steer = Some(String::new());
                self.steer_hard = true;
            }
            'x' if active => {
                let role = agent.agent.role.clone();
                self.ask(
                    format!("Arrêter l'agent « {role} » ? Son travail en cours sera perdu."),
                    Command::CancelAgent { agent_id },
                );
            }
            _ => {}
        }
    }

    /// Open the team editor on the ticket's proposal, or its accepted team.
    fn open_editor(&mut self) {
        let Some(detail) = self.ticket.as_ref() else {
            return;
        };
        let proposal = detail.ticket.proposal.clone().or_else(|| {
            detail
                .ticket
                .team
                .as_ref()
                .map(|t| orchestra_core::model::TeamProposal {
                    summary: String::new(),
                    members: t.members.clone(),
                    risks: Vec::new(),
                    estimated_size: orchestra_core::model::Size::M,
                })
        });
        match proposal {
            Some(p) => {
                self.editor.load(&p, self.roles.clone());
                self.screen = Screen::Proposal;
            }
            None => {
                self.status = "pas encore de proposition — « p » pour planifier".into();
            }
        }
    }

    fn on_proposal_char(&mut self, c: char) {
        match c {
            'm' => {
                let aliases = self.model_aliases.clone();
                self.editor.cycle_model(&aliases);
            }
            'e' => self.editor.cycle_effort(),
            'd' => self.editor.remove_selected(),
            'a' => self.editor.add_next_role(),
            'J' => self.editor.reorder(1),
            'K' => self.editor.reorder(-1),
            'o' => self.editor.start_editing_objective(),
            'y' => self.accept_team(),
            'r' => {
                if let Some(ticket_id) = self.ticket.as_ref().map(|d| d.ticket.id) {
                    self.start_planning(ticket_id);
                    self.screen = Screen::Ticket;
                    self.status = "nouvelle proposition demandée…".into();
                }
            }
            _ => {}
        }
    }

    fn accept_team(&mut self) {
        let Some(detail) = self.ticket.as_ref() else {
            return;
        };
        match self.editor.stages() {
            Ok(stages) => {
                self.outbox.push(Command::AcceptProposal {
                    ticket_id: detail.ticket.id,
                    team: orchestra_core::model::Team {
                        members: self.editor.members.clone(),
                        stages,
                    },
                });
                self.screen = Screen::Ticket;
                self.status = "équipe acceptée".into();
            }
            Err(e) => self.editor.error = Some(e),
        }
    }

    fn on_cost_char(&mut self, c: char) {
        match c {
            'm' => {
                self.cost.group = self.cost.next_group();
                self.cost.selected = 0;
                self.request_usage();
            }
            'p' => {
                self.cost.period = self.cost.period.next();
                self.cost.selected = 0;
                self.request_usage();
            }
            'u' => {
                self.cost.include_unmanaged = !self.cost.include_unmanaged;
                self.request_usage();
            }
            _ => {}
        }
    }

    fn go(&mut self, screen: Screen) {
        let entering_agent = screen == Screen::Agent && self.screen != Screen::Agent;
        let leaving_agent = screen != Screen::Agent && self.screen == Screen::Agent;
        if screen == Screen::Rules && self.screen != Screen::Rules {
            // The project may have changed on the board since the last read.
            self.request_rules();
        }
        self.screen = screen;
        // The daemon keeps one subscription per connection: whatever the
        // screen streams must be asked for again on the way in and out, or the
        // board goes on showing one agent and the agent screen shows nothing.
        if leaving_agent {
            self.subscribe_wide();
        }
        if entering_agent {
            match self.watched_agent_id() {
                Some(agent_id) => {
                    self.log.clear();
                    self.outbox.push(Command::Subscribe {
                        filter: orchestra_core::events::EventFilter::for_agent(agent_id),
                        since_seq: None,
                        backlog: 500,
                    });
                }
                None => {
                    // Reached from the tab strip rather than from a ticket:
                    // find the agent that is working, wherever it is.
                    self.outbox.push(Command::ListAgents { only_active: true });
                    self.status = "recherche d'un agent en cours…".into();
                }
            }
        }
        if !screen.is_implemented() {
            self.status = format!("« {} » arrive dans une phase suivante", screen.title_fr());
        } else {
            self.status = "connecté".into();
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let (len, sel) = match (self.screen, self.board_pane) {
            (Screen::Board, BoardPane::Projects) => {
                // +1 for "Tous les projets", index 0, ahead of the real list.
                (self.projects.len() + 1, &mut self.project_selected)
            }
            (Screen::Board, BoardPane::Tickets) => {
                // Up and down stay in the column; left and right change it.
                let lanes = self.lanes();
                match self.selected_lane() {
                    Some((col, row)) => {
                        let held = &lanes[col].1;
                        let next = (row as isize + delta).clamp(0, held.len() as isize - 1);
                        self.ticket_selected = held[next as usize];
                    }
                    None => {
                        if let Some(first) = lanes.iter().find_map(|(_, held)| held.first()) {
                            self.ticket_selected = *first;
                        }
                    }
                }
                return;
            }
            (Screen::Cost, _) => (self.cost.rows.len(), &mut self.cost.selected),
            (Screen::Todo, _) => (self.todos.len(), &mut self.todo_selected),
            (Screen::Rules, _) => (self.book_len(), &mut self.rule_selected),
            (Screen::Proposal, _) => {
                self.editor.move_selection(delta);
                return;
            }
            (Screen::Ticket, _) => {
                let count = self.ticket.as_ref().map(|d| d.agents.len()).unwrap_or(0);
                if count > 0 {
                    let next = (self.agent_selected as isize + delta).clamp(0, count as isize - 1);
                    self.agent_selected = next as usize;
                    self.agent_pinned = true;
                }
                return;
            }
            (Screen::Agent, _) => {
                // On a live log, down means towards the newest.
                if delta < 0 {
                    self.log.scroll_up(delta.unsigned_abs(), 20);
                } else {
                    self.log.scroll_down(delta as usize);
                }
                return;
            }
            _ => return,
        };
        if len == 0 {
            *sel = 0;
            return;
        }
        let next = (*sel as isize + delta).clamp(0, len as isize - 1);
        *sel = next as usize;
        if self.screen == Screen::Board && self.board_pane == BoardPane::Projects {
            self.request_tickets();
        }
    }

    fn set_selection(&mut self, index: usize) {
        let (len, sel) = match (self.screen, self.board_pane) {
            (Screen::Board, BoardPane::Projects) => {
                (self.projects.len() + 1, &mut self.project_selected)
            }
            // The ends of a column, not of the whole board.
            (Screen::Board, BoardPane::Tickets) => {
                let lanes = self.lanes();
                if let Some((col, _)) = self.selected_lane() {
                    let held = &lanes[col].1;
                    let row = index.min(held.len().saturating_sub(1));
                    self.ticket_selected = held[row];
                }
                return;
            }
            (Screen::Cost, _) => (self.cost.rows.len(), &mut self.cost.selected),
            (Screen::Todo, _) => (self.todos.len(), &mut self.todo_selected),
            (Screen::Rules, _) => (self.book_len(), &mut self.rule_selected),
            _ => return,
        };
        *sel = index.min(len.saturating_sub(1));
        if self.screen == Screen::Board && self.board_pane == BoardPane::Projects {
            self.request_tickets();
        }
    }

    fn open_selection(&mut self) {
        match (self.screen, self.board_pane) {
            (Screen::Board, BoardPane::Projects) => {
                self.board_pane = BoardPane::Tickets;
                self.request_tickets();
            }
            (Screen::Board, BoardPane::Tickets) => {
                if let Some(ticket_id) = self.selected_ticket_id() {
                    self.agent_selected = 0;
                    self.agent_pinned = false;
                    self.outbox.push(Command::GetTicket { ticket_id });
                    self.go(Screen::Ticket);
                }
            }
            (Screen::Proposal, _) => self.editor.start_editing_objective(),
            (Screen::Ticket, _) => self.open_agent(),
            _ => {}
        }
    }

    fn run_palette(&mut self, line: &str) {
        let line = line.trim();
        let (verb, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        match verb {
            "project" | "projet" => {
                let (sub, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                match sub {
                    "add" if !arg.is_empty() => {
                        self.outbox.push(Command::AddProject {
                            path: arg.into(),
                            name: None,
                        });
                        self.status = format!("ajout du projet {arg}…");
                    }
                    "forget" | "oublie" | "oublier" if !arg.is_empty() => {
                        match self.find_project(arg) {
                            Some(p) => {
                                let (id, name) = (p.id, p.name.clone());
                                self.ask(
                                    format!("Oublier le projet « {name} » ?"),
                                    Command::ForgetProject { project_id: id },
                                );
                            }
                            None => {
                                self.status = format!("aucun projet ne correspond à « {arg} »")
                            }
                        }
                    }
                    _ => {
                        self.status =
                            "usage : :project add <chemin> | :project forget <nom>".into()
                    }
                }
            }
            "todo" => {
                let (sub, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                match sub {
                    "add" if !arg.is_empty() => {
                        self.outbox.push(Command::CreateTodo {
                            title: arg.into(),
                            notes: String::new(),
                            urgent: false,
                            due_at: None,
                        });
                        self.status = "ajout du todo…".into();
                    }
                    "urgent" if !arg.is_empty() => match self.todo_at(arg) {
                        Some(t) => self.outbox.push(Command::UpdateTodo {
                            todo_id: t.id,
                            title: t.title.clone(),
                            notes: t.notes.clone(),
                            urgent: !t.urgent,
                            due_at: t.due_at,
                        }),
                        None => self.status = format!("aucun todo « {arg} »"),
                    },
                    "due" if !arg.is_empty() => {
                        let (n, date) = arg.split_once(char::is_whitespace).unwrap_or((arg, ""));
                        match (self.todo_at(n), parse_due_date(date)) {
                            (Some(t), Some(due)) => self.outbox.push(Command::UpdateTodo {
                                todo_id: t.id,
                                title: t.title.clone(),
                                notes: t.notes.clone(),
                                urgent: t.urgent,
                                due_at: Some(due),
                            }),
                            (None, _) => self.status = format!("aucun todo « {n} »"),
                            (_, None) => self.status = "date invalide — AAAA-MM-JJ".into(),
                        }
                    }
                    "open" | "doing" | "done" | "drop" if !arg.is_empty() => {
                        let status = match sub {
                            "open" => TodoStatus::Open,
                            "doing" => TodoStatus::InProgress,
                            "done" => TodoStatus::Done,
                            _ => TodoStatus::Dropped,
                        };
                        match self.todo_at(arg) {
                            Some(t) => self.outbox.push(Command::SetTodoStatus {
                                todo_id: t.id,
                                status,
                            }),
                            None => self.status = format!("aucun todo « {arg} »"),
                        }
                    }
                    _ => {
                        self.status = "usage : :todo add <titre> | urgent|due|open|doing|done|drop <n>".into()
                    }
                }
            }
            "convention" | "adr" => {
                let kind = if verb == "adr" {
                    RuleKind::Adr
                } else {
                    RuleKind::Convention
                };
                let (sub, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                let arg = arg.trim();
                match sub {
                    "add" if !arg.is_empty() => {
                        let project_id = self.selected_project().map(|p| p.id);
                        if kind == RuleKind::Adr && project_id.is_none() {
                            self.status = "un ADR appartient à un projet : choisis-en un sur le tableau".into();
                            return;
                        }
                        self.outbox.push(Command::CreateRule {
                            project_id,
                            rule_kind: kind,
                            title: arg.into(),
                        });
                        self.awaiting_rule_file = true;
                        self.status = format!("création de la {}…", kind.label_fr());
                    }
                    _ => self.status = format!("usage : :{verb} add <titre>"),
                }
            }
            "role" | "rôle" => {
                let (sub, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                let arg = arg.trim();
                match sub {
                    "add" if !arg.is_empty() => {
                        self.outbox.push(Command::CreateRole {
                            project_id: self.selected_project().map(|p| p.id),
                            name: arg.into(),
                        });
                        self.awaiting_rule_file = true;
                        self.status = format!("création du rôle « {arg} »…");
                    }
                    _ => self.status = "usage : :role add <nom>".into(),
                }
            }
            "regles" | "règles" | "rules" | "roles" | "rôles" => self.go(Screen::Rules),
            "usage" | "cout" | "coût" => self.go(Screen::Cost),
            "refresh" => self.refresh(),
            "q" | "quit" => self.should_quit = true,
            "" => {}
            other => self.status = format!("commande inconnue : {other}"),
        }
    }

    /// The nth todo as shown on screen (1-based), what `:todo` commands take.
    fn todo_at(&self, spec: &str) -> Option<&Todo> {
        let n: usize = spec.trim().parse().ok()?;
        n.checked_sub(1).and_then(|i| self.todos.get(i))
    }

    /// Take the commands queued by the last update.
    pub fn take_outbox(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.outbox)
    }

    /// Ask the daemon for everything the current screen shows.
    pub fn refresh(&mut self) {
        self.outbox.push(Command::ListProjects);
        self.request_tickets();
        self.request_usage();
        self.outbox.push(Command::ListTodos);
        self.request_rules();
    }

    fn request_tickets(&mut self) {
        self.outbox.push(Command::ListTickets {
            project_id: self.selected_project().map(|p| p.id),
            status: None,
        });
    }

    /// Open the ticket of whichever agent is working.
    fn adopt_live_agent(&mut self, agents: Vec<orchestra_core::protocol::AgentSummary>) {
        let Some(live) = agents
            .iter()
            .find(|a| a.agent.status.is_active())
            .or_else(|| agents.first())
        else {
            self.status = "aucun agent ne tourne en ce moment".into();
            return;
        };
        let already_open = self
            .ticket
            .as_ref()
            .is_some_and(|d| d.ticket.id == live.agent.ticket_id);
        if !already_open {
            self.outbox.push(Command::GetTicket {
                ticket_id: live.agent.ticket_id,
            });
        }
        self.pending_agent = Some(live.agent.id);
        self.status = format!("agent « {} »", live.agent.role);
    }

    /// Reload the open ticket when an event concerns it.
    fn refresh_open_ticket(&mut self, ticket_id: Option<TicketId>) {
        let Some(open) = self.ticket.as_ref().map(|d| d.ticket.id) else {
            return;
        };
        if ticket_id.is_none_or(|id| id == open) {
            self.outbox.push(Command::GetTicket { ticket_id: open });
        }
    }

    fn request_usage(&mut self) {
        for query in self.cost.queries() {
            self.outbox.push(Command::GetUsage { query });
        }
    }

    fn on_reply(&mut self, reply: Reply) {
        match reply {
            Reply::Projects { projects } => {
                let previous = self.selected_project().map(|p| p.id);
                let first_load = !self.projects_loaded;
                self.projects_loaded = true;
                self.projects = projects
                    .into_iter()
                    .map(|p| ProjectRow {
                        id: p.id,
                        name: p.name,
                        path: p.path.display().to_string(),
                        discovered: p.kind == orchestra_core::model::ProjectKind::Discovered,
                    })
                    .collect();
                // Keep the highlight on the same project across refreshes,
                // or on "Tous les projets" if that is where it was, or if
                // the project it was on is simply gone. Only the very first
                // list, with nothing chosen yet, lands on the first project
                // instead of the aggregate — a single-project setup still
                // opens straight onto its own board.
                let fallback = if first_load && !self.projects.is_empty() {
                    1
                } else {
                    0
                };
                self.project_selected = previous
                    .and_then(|id| self.projects.iter().position(|p| p.id == id))
                    .map(|i| i + 1)
                    .unwrap_or(fallback)
                    .min(self.projects.len());
                if self.projects.is_empty() {
                    self.status =
                        "aucun projet — `:project add <chemin>` ou `orchestra project add`".into();
                }
            }
            Reply::Project { project } => {
                self.status = format!("projet « {} » ajouté", project.name);
                self.outbox.push(Command::ListProjects);
            }
            Reply::Tickets { tickets } => {
                let previous = self.selected_ticket_id();
                self.tickets = tickets;
                self.ticket_selected = previous
                    .and_then(|id| self.tickets.iter().position(|t| t.ticket.id == id))
                    .unwrap_or(0)
                    .min(self.tickets.len().saturating_sub(1));
            }
            Reply::Usage { rows, totals } => {
                // Two queries feed this screen and the reply does not name
                // which one it answers, so the trend is recognised by its key.
                let is_daily = rows.first().is_some_and(|r| r.keys.contains_key("day"));
                if is_daily {
                    let mut daily: Vec<(String, f64)> = rows
                        .into_iter()
                        .map(|r| {
                            (
                                r.keys.get("day").cloned().unwrap_or_default(),
                                r.cost_usd.unwrap_or(0.0),
                            )
                        })
                        .collect();
                    daily.sort_by(|a, b| a.0.cmp(&b.0));
                    self.cost.daily = daily;
                } else {
                    self.cost.rows = rows;
                    self.cost.totals = totals;
                    self.cost.selected = self
                        .cost
                        .selected
                        .min(self.cost.rows.len().saturating_sub(1));
                }
            }
            Reply::Ticket { detail } => {
                let previous = self.watched_agent_id();
                self.ticket = Some(detail);
                // A refresh used to clear this, so the indicator vanished a
                // second after the key was pressed while the orchestrator ran
                // for another minute. Only its own agent says when it is over.
                match self.orchestrator_agent().map(|a| a.agent.clone()) {
                    Some(a) if a.status.is_active() => {
                        self.planning = true;
                        self.planning_since.get_or_insert_with(|| {
                            a.started_at.unwrap_or_else(orchestra_core::now)
                        });
                    }
                    Some(a) if a.ended_at >= self.planning_since => self.stop_planning(),
                    _ => {}
                }
                // A ticket fetched to reach one particular agent.
                if let Some(wanted) = self.pending_agent.take() {
                    if let Some(index) = self
                        .ticket
                        .as_ref()
                        .and_then(|d| d.agents.iter().position(|a| a.agent.id == wanted))
                    {
                        self.agent_selected = index;
                        self.log.clear();
                        self.screen = Screen::Agent;
                        self.outbox.push(Command::Subscribe {
                            filter: orchestra_core::events::EventFilter::for_agent(wanted),
                            since_seq: None,
                            backlog: 500,
                        });
                        return;
                    }
                }
                // Keep watching the same agent across refreshes.
                if let Some(id) = previous {
                    if let Some(index) = self
                        .ticket
                        .as_ref()
                        .and_then(|d| d.agents.iter().position(|a| a.agent.id == id))
                    {
                        self.agent_selected = index;
                    }
                }
                let count = self.ticket.as_ref().map(|d| d.agents.len()).unwrap_or(0);
                self.agent_selected = self.agent_selected.min(count.saturating_sub(1));
                // The cursor follows the team: while watching, the screen moves
                // to whoever takes over; on the ticket, it is what scrolls the
                // list to the agent that works — one of a dozen rows, in a pane
                // that shows six. A row the user picked himself is never moved
                // out from under him there.
                let may_follow = match self.screen {
                    Screen::Ticket => !self.agent_pinned,
                    _ => true,
                };
                if may_follow
                    && !self
                        .watched_agent()
                        .is_some_and(|a| a.agent.status.is_active())
                {
                    let previous = self.watched_agent_id();
                    self.select_liveliest_agent();
                    if self.screen == Screen::Agent && self.watched_agent_id() != previous {
                        self.log.clear();
                        if let Some(id) = self.watched_agent_id() {
                            self.outbox.push(Command::Subscribe {
                                filter: orchestra_core::events::EventFilter::for_agent(id),
                                since_seq: None,
                                backlog: 500,
                            });
                        }
                    }
                }
            }
            Reply::Roles { roles, errors } => {
                // The cursor stays on what it was on, role or rule, even when
                // a role appears or goes above it.
                let previous_role = self.selected_role().map(|r| r.name.clone());
                let previous_rule = self.selected_rule().map(|r| (r.kind, r.name.clone()));
                self.roles = roles;
                self.role_errors = errors;
                self.rule_selected = match (previous_role, previous_rule) {
                    (Some(name), _) => self.roles.iter().position(|r| r.name == name),
                    (None, Some((k, n))) => self
                        .rules
                        .iter()
                        .position(|r| r.kind == k && r.name == n)
                        .map(|i| i + self.roles.len()),
                    (None, None) => None,
                }
                .unwrap_or(self.rule_selected)
                .min(self.book_len().saturating_sub(1));
            }
            Reply::Rules { rules, errors } => {
                let previous = self
                    .selected_rule()
                    .map(|r| (r.kind, r.name.clone()));
                self.rules = rules;
                self.rule_errors = errors;
                self.rule_selected = previous
                    .and_then(|(k, n)| self.rules.iter().position(|r| r.kind == k && r.name == n))
                    .map(|i| i + self.roles.len())
                    .unwrap_or(self.rule_selected)
                    .min(self.book_len().saturating_sub(1));
            }
            Reply::RuleFile { path } | Reply::RoleFile { path } => {
                if std::mem::take(&mut self.awaiting_rule_file) {
                    self.edit_request = Some(path);
                } else {
                    self.status = format!("écrit : {}", path.display());
                }
                self.request_rules();
            }
            Reply::Todos { todos } => {
                let previous = self.selected_todo().map(|t| t.id);
                self.todos = todos;
                self.todo_selected = previous
                    .and_then(|id| self.todos.iter().position(|t| t.id == id))
                    .unwrap_or(0)
                    .min(self.todos.len().saturating_sub(1));
            }
            Reply::Agents { agents } => self.adopt_live_agent(agents),
            Reply::Status { status } => {
                self.daemon_version = Some(status.version.clone());
            }
            // The pane is on screen already; what the line adds is which one,
            // for a session where a dozen of them are open.
            Reply::Pane { pane_id } => {
                self.status = format!("pane {pane_id}");
                self.refresh_open_ticket(None);
            }
            Reply::Pong | Reply::Ack | Reply::Subscribed { .. } => {}
            _ => {}
        }
    }

    fn on_event(&mut self, e: Event) {
        // What the orchestrator does while it composes the team.
        self.trace_planning(&e);
        // While watching one agent, its events feed the live log.
        if self.screen == Screen::Agent && e.agent_id == self.watched_agent_id() {
            if self.log.push_event(&e) {
                self.last_activity = Some(e.ts);
            }
            if matches!(
                e.kind,
                EventKind::AgentStatusChanged { .. } | EventKind::Usage { .. }
            ) {
                self.refresh_open_ticket(e.ticket_id);
            }
            return;
        }
        if let Some(line) = describe(&e) {
            self.activity.push(line);
            if self.activity.len() > ACTIVITY_MAX {
                self.activity.drain(..self.activity.len() - ACTIVITY_MAX);
            }
        }
        // Anything that changes a board number triggers a targeted refresh.
        match &e.kind {
            EventKind::ProjectAdded { .. }
            | EventKind::ProjectForgotten { .. }
            | EventKind::UnmanagedSessionSeen { .. } => {
                self.outbox.push(Command::ListProjects);
            }
            EventKind::TodoAdded { .. }
            | EventKind::TodoUpdated { .. }
            | EventKind::TodoStatusChanged { .. }
            | EventKind::TodoDeleted { .. } => {
                self.outbox.push(Command::ListTodos);
            }
            EventKind::RuleProposed { .. }
            | EventKind::RuleCreated { .. }
            | EventKind::RuleStatusChanged { .. }
            | EventKind::RuleDeleted { .. }
            | EventKind::RoleCreated { .. }
            | EventKind::RoleUpdated { .. }
            | EventKind::RoleDeleted { .. } => self.request_rules(),
            EventKind::TodoPromoted { .. } => {
                self.outbox.push(Command::ListTodos);
                self.request_tickets();
            }
            EventKind::TicketCreated { .. }
            | EventKind::TicketStatusChanged { .. }
            | EventKind::AgentStatusChanged { .. } => {
                self.request_tickets();
                self.refresh_open_ticket(e.ticket_id);
            }
            EventKind::ProposalReady { .. } => {
                self.stop_planning();
                self.status = "proposition prête — « a » pour la relire".into();
                self.refresh_open_ticket(e.ticket_id);
            }
            EventKind::ProposalFailed { error } => {
                self.stop_planning();
                self.status = format!("planification échouée : {error}");
            }
            EventKind::PullRequestOpened { url, .. } => {
                self.status = format!("pull request ouverte : {url}");
                self.request_tickets();
                self.refresh_open_ticket(e.ticket_id);
            }
            EventKind::PullRequestClosed { merged, .. } => {
                self.status = if *merged {
                    "pull request fusionnée — ticket terminé".into()
                } else {
                    "pull request fermée sans fusion".into()
                };
                self.request_tickets();
                self.refresh_open_ticket(e.ticket_id);
            }
            // A gate that refuses is the one thing that changes what the
            // ticket can do next, so it is said out loud rather than left in
            // the activity strip.
            EventKind::CheckFinished { run, .. } => {
                if !run.ok {
                    self.status = format!("✗ {}", run.label_fr());
                }
                self.request_tickets();
                self.refresh_open_ticket(e.ticket_id);
            }
            EventKind::CheckStarted { command, .. } => {
                self.status = format!("vérification : {command}…");
            }
            // Two moments the user must not have to go looking for.
            EventKind::ReviewVerdict { round, verdict, .. } => {
                self.status = format!("relecture {round} : {}", verdict.label_fr());
                self.request_tickets();
                self.refresh_open_ticket(e.ticket_id);
            }
            EventKind::MergeBlocked { reason, .. } => {
                self.status = format!(
                    "{} fusion en attente : {reason} — « f » pour réessayer",
                    crate::theme::merge_waiting().symbol
                );
                self.request_tickets();
                self.refresh_open_ticket(e.ticket_id);
            }
            EventKind::TicketMerged {
                branch,
                into,
                pushed_to,
                ..
            } => {
                self.status = match pushed_to {
                    Some(remote) => format!("{branch} fusionnée dans {into}, poussée sur {remote}"),
                    None => format!("{branch} fusionnée dans {into}"),
                };
                self.request_tickets();
                self.refresh_open_ticket(e.ticket_id);
            }
            // A managed agent reports every response. The cost view is polled
            // instead, so a busy agent cannot spin the loop with queries.
            EventKind::Usage { .. } => {}
            _ => {}
        }
    }
}

/// How an agent is named on screen.
///
/// A relecture can send a role back to work, so the same role appears several
/// times on one ticket. Two identical rows would be unreadable, hence the run
/// number on every one but the first.
pub fn agent_label(agents: &[orchestra_core::protocol::AgentSummary], index: usize) -> String {
    let Some(agent) = agents.get(index) else {
        return String::new();
    };
    let earlier = agents[..index]
        .iter()
        .filter(|a| a.agent.role == agent.agent.role)
        .count();
    if earlier == 0 {
        agent.agent.role.clone()
    } else {
        format!("{} · reprise {earlier}", agent.agent.role)
    }
}

/// How many lines of the orchestrator's work the ticket screen keeps.
const PLAN_TRACE_MAX: usize = 8;

/// One line of the planning trace, or `None` for an event that says nothing
/// about what the orchestrator is doing.
///
/// Deliberately not [`describe`]: this reads as a running commentary of one
/// run, in the present tense, not as a log of the whole daemon.
fn plan_line(e: &Event) -> Option<String> {
    match &e.kind {
        EventKind::ToolStarted { tool, summary, .. } => {
            let verb = match tool.as_str() {
                "Read" => "lit",
                "Grep" => "cherche",
                "Glob" => "liste",
                other => other,
            };
            let summary = summary.trim();
            Some(if summary.is_empty() {
                verb.to_string()
            } else {
                format!("{verb} {summary}")
            })
        }
        // The content of a thinking block is never stored; its size still says
        // that something is happening.
        EventKind::AgentThinking { chars } => Some(format!(
            "réfléchit ({} caractères)",
            orchestra_core::pricing::fmt_tokens(*chars as u64)
        )),
        EventKind::AgentText { text } => {
            let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
            Some(orchestra_core::claude::stream::truncate(line, 70))
        }
        _ => None,
    }
}

/// One activity line for an event, or `None` when it is not worth showing.
pub fn describe(e: &Event) -> Option<String> {
    let t = e.ts.time();
    let stamp = format!("{:02}:{:02}:{:02}", t.hour(), t.minute(), t.second());
    let body = match &e.kind {
        EventKind::DaemonStarted { version } => format!("daemon {version} démarré"),
        EventKind::ProjectAdded { name, .. } => format!("projet « {name} » ajouté"),
        EventKind::ProjectForgotten { name } => format!("projet « {name} » oublié"),
        EventKind::TicketCreated { number, title } => format!("ticket #{number} « {title} » créé"),
        EventKind::TicketStatusChanged { from, to } => {
            format!("ticket {} → {}", from.label_fr(), to.label_fr())
        }
        EventKind::ProposalReady { proposal } => {
            format!("équipe proposée : {} rôles", proposal.members.len())
        }
        EventKind::ProposalFailed { error } => format!("planification échouée : {error}"),
        EventKind::WorktreeCreated { branch, .. } => format!("worktree sur {branch}"),
        EventKind::AgentSpawned { role, pid, .. } => format!("agent {role} démarré (pid {pid})"),
        EventKind::AgentStatusChanged { status, reason } => match reason {
            Some(r) => format!("agent {} — {}", status.label_fr(), r.label_fr()),
            None => format!("agent {}", status.label_fr()),
        },
        EventKind::ToolStarted { tool, summary, .. } => format!("{tool} {summary}"),
        EventKind::ToolFinished { ok, summary, .. } => {
            format!("{} {summary}", if *ok { "ok" } else { "échec" })
        }
        EventKind::AgentSteered { text, hard, .. } => {
            format!(
                "{} : {text}",
                if *hard { "redirection" } else { "consigne" }
            )
        }
        EventKind::AgentResult {
            subtype, num_turns, ..
        } => {
            format!("agent terminé ({subtype}, {num_turns} tours)")
        }
        EventKind::PullRequestOpened { url, number } => match number {
            Some(n) => format!("pull request #{n} ouverte — {url}"),
            None => format!("pull request ouverte — {url}"),
        },
        EventKind::PullRequestClosed { merged, .. } => {
            if *merged {
                "pull request fusionnée".to_string()
            } else {
                "pull request fermée sans fusion".to_string()
            }
        }
        EventKind::TicketResumed { skipped } if skipped.is_empty() => "ticket repris".to_string(),
        EventKind::TicketResumed { skipped } => {
            format!("ticket repris — déjà fait : {}", skipped.join(", "))
        }
        EventKind::TicketMerged {
            branch,
            into,
            commits,
            pushed_to,
        } => match pushed_to {
            Some(remote) => {
                format!("{branch} fusionnée dans {into} ({commits} commits), poussée sur {remote}")
            }
            None => format!("{branch} fusionnée dans {into} ({commits} commits)"),
        },
        EventKind::MergeBlocked { branch, reason, .. } => {
            format!("fusion de {branch} en attente : {reason}")
        }
        EventKind::ReviewVerdict {
            round,
            verdict,
            roles,
            ..
        } => {
            let who = if roles.is_empty() {
                String::new()
            } else {
                format!(" — {} repasse(nt)", roles.join(", "))
            };
            format!("relecture {round} : {}{who}", verdict.label_fr())
        }
        EventKind::CheckStarted { command, .. } => format!("vérification : {command}…"),
        EventKind::CheckFinished { run, .. } => {
            let mark = if run.ok { "✓" } else { "✗" };
            format!("{mark} {}", run.label_fr())
        }
        EventKind::HookBlocked { tool, reason } => format!("{tool} bloqué : {reason}"),
        EventKind::UnmanagedSessionSeen { cwd, .. } => {
            format!("session libre détectée dans {}", cwd.display())
        }
        EventKind::Warning { message } => format!("attention : {message}"),
        EventKind::RuleProposed {
            rule_kind,
            title,
            by,
            ..
        } => format!("{} proposée par {by} : « {title} » — à valider (8)", rule_kind.label_fr()),
        EventKind::RuleCreated { rule_kind, title, .. } => {
            format!("{} « {title} » créée", rule_kind.label_fr())
        }
        EventKind::RuleStatusChanged {
            rule_kind,
            title,
            to,
            ..
        } => format!("{} « {title} » : {}", rule_kind.label_fr(), to.label_fr()),
        EventKind::RoleCreated { name, .. } => format!("rôle « {name} » créé"),
        EventKind::RoleUpdated { name, change } => format!("rôle « {name} » : {change}"),
        EventKind::RoleDeleted { name } => format!("rôle « {name} » supprimé"),
        EventKind::RuleDeleted { rule_kind, title, .. } => {
            format!("{} « {title} » supprimée", rule_kind.label_fr())
        }
        EventKind::RulesChecked { round, violations } => {
            if violations.is_empty() {
                format!("conventions respectées (passe {round})")
            } else {
                format!(
                    "conventions : {} écart(s), passe {round} — {}",
                    violations.len(),
                    violations[0].line()
                )
            }
        }
        EventKind::TodoAdded { title } => format!("todo « {title} » ajouté"),
        EventKind::TodoUpdated { title } => format!("todo « {title} » modifié"),
        EventKind::TodoStatusChanged { title, from, to } => {
            format!("todo « {title} » : {} → {}", from.label_fr(), to.label_fr())
        }
        EventKind::TodoDeleted { title } => format!("todo « {title} » supprimé"),
        EventKind::TodoPromoted {
            title,
            ticket_number,
            project_name,
        } => format!("todo « {title} » promu en ticket #{ticket_number} sur {project_name}"),
        // Too chatty or not user-facing.
        EventKind::AgentText { .. }
        | EventKind::AgentThinking { .. }
        | EventKind::Usage { .. }
        | EventKind::SubagentStarted { .. }
        | EventKind::SubagentFinished { .. }
        | EventKind::PaneOpened { .. }
        | EventKind::PaneClosed { .. } => return None,
    };
    Some(format!("{stamp}  {body}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::events::NewEvent;
    use orchestra_core::protocol::Command;
    use uuid::Uuid;

    fn app_with_projects(n: usize) -> App {
        let mut app = App::new();
        app.projects = (0..n)
            .map(|i| ProjectRow {
                id: Uuid::new_v4(),
                name: format!("p{i}"),
                path: format!("/tmp/p{i}"),
                discovered: false,
            })
            .collect();
        app
    }

    #[test]
    fn screens_cycle_both_ways() {
        assert_eq!(Screen::Board.next(), Screen::Ticket);
        assert_eq!(Screen::Board.prev(), Screen::Rules);
        assert_eq!(Screen::from_number(1), Some(Screen::Board));
        assert_eq!(Screen::from_number(6), Some(Screen::Proposal));
        assert_eq!(Screen::from_number(7), Some(Screen::Todo));
        assert_eq!(Screen::from_number(8), Some(Screen::Rules));
        assert_eq!(Screen::from_number(0), None);
        assert_eq!(Screen::from_number(9), None);
    }

    #[test]
    fn selection_stays_in_bounds() {
        let mut app = app_with_projects(3);
        app.update(Msg::Key(Action::Up));
        assert_eq!(app.project_selected, 0);
        for _ in 0..10 {
            app.update(Msg::Key(Action::Down));
        }
        // 0 is "Tous les projets", 1..=3 the three real ones.
        assert_eq!(app.project_selected, 3);
        app.update(Msg::Key(Action::Top));
        assert_eq!(app.project_selected, 0);
        app.update(Msg::Key(Action::Bottom));
        assert_eq!(app.project_selected, 3);
    }

    #[test]
    fn the_first_project_list_lands_on_the_first_project_not_tous() {
        let mut app = App::new();
        assert!(app.selected_project().is_none(), "rien encore reçu");
        let projects = vec![
            orchestra_core::model::Project {
                id: Uuid::new_v4(),
                name: "seul".into(),
                path: "/tmp/seul".into(),
                default_branch: "main".into(),
                zellij_tab: None,
                kind: orchestra_core::model::ProjectKind::Managed,
                created_at: orchestra_core::now(),
            },
            orchestra_core::model::Project {
                id: Uuid::new_v4(),
                name: "autre".into(),
                path: "/tmp/autre".into(),
                default_branch: "main".into(),
                zellij_tab: None,
                kind: orchestra_core::model::ProjectKind::Managed,
                created_at: orchestra_core::now(),
            },
        ];
        app.update(Msg::Reply(Box::new(Reply::Projects {
            projects: projects.clone(),
        })));
        assert_eq!(
            app.selected_project().unwrap().id,
            projects[0].id,
            "un premier chargement ouvre directement sur un projet"
        );

        // A later refresh, with "Tous les projets" now chosen on purpose,
        // does not snap back to a project.
        app.project_selected = 0;
        app.update(Msg::Reply(Box::new(Reply::Projects { projects })));
        assert!(app.selected_project().is_none(), "« Tous » est respecté");
    }

    #[test]
    fn tous_les_projets_asks_for_every_ticket() {
        let mut app = app_with_projects(2);
        assert_eq!(app.project_selected, 0);
        let cmds = app.update(Msg::Key(Action::Select));
        assert!(cmds.iter().any(
            |c| matches!(c, Command::ListTickets { project_id: None, .. })
        ));
    }

    #[test]
    fn empty_lists_do_not_panic() {
        let mut app = App::new();
        for a in [
            Action::Down,
            Action::Up,
            Action::Top,
            Action::Bottom,
            Action::Select,
        ] {
            app.update(Msg::Key(a));
        }
        assert_eq!(app.project_selected, 0);
    }

    #[test]
    fn moving_between_projects_asks_for_their_tickets() {
        let mut app = app_with_projects(2);
        let cmds = app.update(Msg::Key(Action::Down));
        assert!(cmds
            .iter()
            .any(|c| matches!(c, Command::ListTickets { .. })));
    }

    #[test]
    fn palette_captures_keys_then_runs() {
        let mut app = App::new();
        app.update(Msg::Key(Action::CommandPalette));
        assert!(app.palette.is_some());
        // `j` must type, not move.
        for c in "project add /tmp/x".chars() {
            app.update(Msg::Key(Action::Char(c)));
        }
        assert_eq!(app.palette.as_deref(), Some("project add /tmp/x"));
        let cmds = app.update(Msg::Key(Action::Submit));
        assert!(app.palette.is_none());
        assert!(matches!(
            cmds.first(),
            Some(Command::AddProject { path, .. }) if path.as_os_str() == "/tmp/x"
        ));
    }

    #[test]
    fn palette_reports_unknown_commands() {
        let mut app = App::new();
        app.update(Msg::Key(Action::CommandPalette));
        for c in "danse".chars() {
            app.update(Msg::Key(Action::Char(c)));
        }
        app.update(Msg::Key(Action::Submit));
        assert!(app.status.contains("inconnue"), "{}", app.status);
    }

    #[test]
    fn d_on_tous_les_projets_forgets_nothing() {
        let mut app = app_with_projects(2);
        assert_eq!(app.project_selected, 0, "démarre sur « Tous les projets »");
        app.update(Msg::Key(Action::Char('d')));
        assert!(app.confirm.is_none());
        assert!(app.status.contains("aucun projet"), "{}", app.status);
    }

    #[test]
    fn d_asks_before_forgetting_the_selected_project() {
        let mut app = app_with_projects(2);
        app.project_selected = 1;
        let id = app.projects[0].id;
        let cmds = app.update(Msg::Key(Action::Char('d')));
        // Nothing is sent yet — a question is, and it names the project.
        assert!(cmds.is_empty());
        let confirm = app.confirm.as_ref().expect("a question was asked");
        assert!(confirm.question.contains("p0"), "{}", confirm.question);
        assert_eq!(confirm.command, Command::ForgetProject { project_id: id });

        let cmds = app.update(Msg::Key(Action::Char('y')));
        assert!(matches!(
            cmds.as_slice(),
            [Command::ForgetProject { project_id }] if *project_id == id
        ));
        assert!(app.confirm.is_none());
    }

    #[test]
    fn d_on_the_tickets_pane_does_not_touch_projects() {
        let mut app = app_with_projects(1);
        app.board_pane = BoardPane::Tickets;
        app.update(Msg::Key(Action::Char('d')));
        assert!(app.confirm.is_none());
    }

    #[test]
    fn palette_project_forget_resolves_a_project_by_name() {
        let mut app = app_with_projects(2);
        let id = app.projects[1].id;
        app.update(Msg::Key(Action::CommandPalette));
        for c in "project forget p1".chars() {
            app.update(Msg::Key(Action::Char(c)));
        }
        app.update(Msg::Key(Action::Submit));
        let confirm = app.confirm.as_ref().expect("a question was asked");
        assert_eq!(confirm.command, Command::ForgetProject { project_id: id });
    }

    #[test]
    fn palette_project_forget_reports_no_match() {
        let mut app = app_with_projects(1);
        app.update(Msg::Key(Action::CommandPalette));
        for c in "project forget nope".chars() {
            app.update(Msg::Key(Action::Char(c)));
        }
        app.update(Msg::Key(Action::Submit));
        assert!(app.confirm.is_none());
        assert!(app.status.contains("nope"), "{}", app.status);
    }

    #[test]
    fn a_forgotten_project_refreshes_the_list() {
        let mut app = App::new();
        let e = Event::from_new(
            1,
            NewEvent::new(EventKind::ProjectForgotten { name: "p0".into() }),
        );
        let cmds = app.update(Msg::Event(Box::new(e)));
        assert!(cmds.iter().any(|c| matches!(c, Command::ListProjects)));
    }

    fn app_with_todos(n: usize) -> App {
        let mut app = App::new();
        let now = orchestra_core::now();
        app.screen = Screen::Todo;
        app.todos = (0..n)
            .map(|i| Todo {
                id: Uuid::new_v4(),
                title: format!("t{i}"),
                notes: String::new(),
                status: TodoStatus::Open,
                urgent: false,
                due_at: None,
                promoted_ticket_id: None,
                created_at: now,
                updated_at: now,
            })
            .collect();
        app
    }

    #[test]
    fn d_asks_before_deleting_the_selected_todo() {
        let mut app = app_with_todos(2);
        let id = app.todos[0].id;
        let cmds = app.update(Msg::Key(Action::Char('d')));
        assert!(cmds.is_empty());
        let confirm = app.confirm.as_ref().expect("a question was asked");
        assert!(confirm.question.contains("t0"), "{}", confirm.question);
        assert_eq!(confirm.command, Command::DeleteTodo { todo_id: id });

        let cmds = app.update(Msg::Key(Action::Char('y')));
        assert!(matches!(
            cmds.as_slice(),
            [Command::DeleteTodo { todo_id }] if *todo_id == id
        ));
    }

    fn app_with_rules() -> App {
        use orchestra_core::conventions::RuleMode;
        let mut app = App::new();
        app.screen = Screen::Rules;
        app.rules = ["commits", "paginer"]
            .iter()
            .map(|n| Rule {
                name: n.to_string(),
                kind: RuleKind::Convention,
                title: format!("titre {n}"),
                status: if *n == "paginer" {
                    RuleStatus::Proposed
                } else {
                    RuleStatus::Accepted
                },
                applies_to: vec![],
                mode: RuleMode::Any,
                checks: vec![],
                supersedes: None,
                proposed_by: None,
                body: "corps".into(),
                source: format!("/nulle-part/{n}.md").into(),
                scope: orchestra_core::model::RoleScope::Global,
            })
            .collect();
        app
    }

    #[test]
    fn a_accepts_the_selected_rule_and_r_asks_first() {
        let mut app = app_with_rules();
        assert_eq!(app.pending_rules(), 1);
        app.update(Msg::Key(Action::Down));
        let cmds = app.update(Msg::Key(Action::Char('a')));
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetRuleStatus { name, status: RuleStatus::Accepted, .. }] if name == "paginer"
        ));

        let cmds = app.update(Msg::Key(Action::Char('r')));
        assert!(cmds.is_empty(), "rejeter se confirme");
        assert!(app.confirm.as_ref().unwrap().question.contains("titre paginer"));
        let cmds = app.update(Msg::Key(Action::Char('y')));
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetRuleStatus { status: RuleStatus::Rejected, .. }]
        ));
    }

    #[test]
    fn a_rule_that_is_not_on_disk_is_not_sent_to_an_editor() {
        let mut app = app_with_rules();
        app.update(Msg::Key(Action::Char('e')));
        assert!(app.take_edit_request().is_none());
        assert!(app.status.contains("orchestra init"), "{}", app.status);
    }

    #[test]
    fn a_new_rule_opens_in_the_editor_once_written() {
        let mut app = App::new();
        app.update(Msg::Key(Action::CommandPalette));
        for c in "convention add Toujours paginer".chars() {
            app.update(Msg::Key(Action::Char(c)));
        }
        let cmds = app.update(Msg::Key(Action::Submit));
        assert!(matches!(
            cmds.first(),
            Some(Command::CreateRule { rule_kind: RuleKind::Convention, title, project_id: None })
                if title == "Toujours paginer"
        ));
        let cmds = app.update(Msg::Reply(Box::new(Reply::RuleFile {
            path: "/x/toujours-paginer.md".into(),
        })));
        assert!(cmds.iter().any(|c| matches!(c, Command::ListRules { .. })));
        assert_eq!(
            app.take_edit_request().as_deref(),
            Some(std::path::Path::new("/x/toujours-paginer.md"))
        );
        // A file written for another reason does not open anything.
        app.update(Msg::Reply(Box::new(Reply::RuleFile { path: "/x/y.md".into() })));
        assert!(app.take_edit_request().is_none());
    }

    #[test]
    fn an_adr_needs_a_project() {
        let mut app = App::new();
        app.update(Msg::Key(Action::CommandPalette));
        for c in "adr add SQLite".chars() {
            app.update(Msg::Key(Action::Char(c)));
        }
        let cmds = app.update(Msg::Key(Action::Submit));
        assert!(!cmds.iter().any(|c| matches!(c, Command::CreateRule { .. })));
        assert!(app.status.contains("projet"), "{}", app.status);
    }

    #[test]
    fn a_reread_keeps_the_selected_rule() {
        let mut app = app_with_rules();
        app.update(Msg::Key(Action::Down));
        let mut rules = app.rules.clone();
        rules.reverse();
        app.update(Msg::Reply(Box::new(Reply::Rules { rules, errors: vec![] })));
        assert_eq!(app.selected_rule().unwrap().name, "paginer");
    }

    #[test]
    fn palette_todo_add_creates_a_todo() {
        let mut app = App::new();
        app.update(Msg::Key(Action::CommandPalette));
        for c in "todo add acheter du café".chars() {
            app.update(Msg::Key(Action::Char(c)));
        }
        let cmds = app.update(Msg::Key(Action::Submit));
        assert!(matches!(
            cmds.first(),
            Some(Command::CreateTodo { title, .. }) if title == "acheter du café"
        ));
    }

    #[test]
    fn palette_todo_status_resolves_by_position() {
        let mut app = app_with_todos(2);
        let id = app.todos[1].id;
        app.update(Msg::Key(Action::CommandPalette));
        for c in "todo done 2".chars() {
            app.update(Msg::Key(Action::Char(c)));
        }
        let cmds = app.update(Msg::Key(Action::Submit));
        assert!(matches!(
            cmds.first(),
            Some(Command::SetTodoStatus { todo_id, status })
                if *todo_id == id && *status == TodoStatus::Done
        ));
    }

    #[test]
    fn promoting_a_todo_seeds_the_form_and_sends_promote_on_submit() {
        let mut app = app_with_todos(1);
        app.projects = vec![ProjectRow {
            id: Uuid::new_v4(),
            name: "p0".into(),
            path: "/tmp/p0".into(),
            discovered: false,
        }];
        app.project_selected = 1;
        let todo_id = app.todos[0].id;
        let project_id = app.projects[0].id;

        app.update(Msg::Key(Action::Char('p')));
        assert_eq!(app.screen, Screen::NewTicket);
        assert_eq!(app.form.title, "t0");
        assert_eq!(app.promoting_todo, Some(todo_id));

        let cmds = app.update(Msg::Key(Action::NextField));
        assert!(cmds.is_empty());
        for c in "un brief assez long pour passer la validation".chars() {
            app.update(Msg::Key(Action::Char(c)));
        }
        let cmds = app.update(Msg::Key(Action::Accept));
        assert!(matches!(
            cmds.as_slice(),
            [Command::PromoteTodo { todo_id: t, project_id: p, .. }]
                if *t == todo_id && *p == project_id
        ));
        assert!(app.promoting_todo.is_none());
    }

    #[test]
    fn a_promoted_todo_refreshes_todos_and_tickets() {
        let mut app = App::new();
        let e = Event::from_new(
            1,
            NewEvent::new(EventKind::TodoPromoted {
                title: "t0".into(),
                ticket_number: 1,
                project_name: "p0".into(),
            }),
        );
        let cmds = app.update(Msg::Event(Box::new(e)));
        assert!(cmds.iter().any(|c| matches!(c, Command::ListTodos)));
        assert!(cmds.iter().any(|c| matches!(c, Command::ListTickets { .. })));
    }

    #[test]
    fn todo_digest_relays_to_the_shared_core_rule() {
        // The counting rule itself is tested exhaustively in
        // orchestra-core (shared with the CLI's own notification); this
        // only checks the relay reads `self.todos`.
        let mut app = App::new();
        assert!(app.todo_digest().is_empty());
        app.todos = vec![Todo {
            id: Uuid::new_v4(),
            title: "x".into(),
            notes: String::new(),
            status: TodoStatus::Open,
            urgent: true,
            due_at: None,
            promoted_ticket_id: None,
            created_at: orchestra_core::now(),
            updated_at: orchestra_core::now(),
        }];
        assert_eq!(app.todo_digest().urgent, 1);
    }

    #[test]
    fn help_opens_and_any_key_closes_it() {
        let mut app = App::new();
        app.update(Msg::Key(Action::Help));
        assert!(app.show_help);
        app.update(Msg::Key(Action::Down));
        assert!(!app.show_help);
        assert!(!app.should_quit);
    }

    #[test]
    fn quit_works_from_the_help_overlay() {
        let mut app = App::new();
        app.update(Msg::Key(Action::Help));
        app.update(Msg::Key(Action::Quit));
        assert!(app.should_quit);
    }

    #[test]
    fn disconnection_is_visible() {
        let mut app = App::new();
        app.update(Msg::Reconnected("0.1.0".into()));
        assert!(app.connected);
        app.update(Msg::Disconnected);
        assert!(!app.connected);
        assert!(app.status.contains("déconnecté"));
    }

    #[test]
    fn activity_is_bounded() {
        let mut app = App::new();
        for i in 0..(ACTIVITY_MAX + 50) {
            let e = Event::from_new(
                i as i64,
                NewEvent::new(EventKind::Warning {
                    message: format!("w{i}"),
                }),
            );
            app.update(Msg::Event(Box::new(e)));
        }
        assert_eq!(app.activity.len(), ACTIVITY_MAX);
        assert!(app
            .activity
            .last()
            .unwrap()
            .contains(&format!("w{}", ACTIVITY_MAX + 49)));
    }

    #[test]
    fn chatty_events_stay_out_of_the_activity_strip() {
        let e = Event::from_new(1, NewEvent::new(EventKind::AgentText { text: "x".into() }));
        assert_eq!(describe(&e), None);
        let e = Event::from_new(
            2,
            NewEvent::new(EventKind::TicketCreated {
                number: 3,
                title: "T".into(),
            }),
        );
        assert!(describe(&e).unwrap().contains("#3"));
    }

    #[test]
    fn selection_survives_a_refresh_that_reorders_projects() {
        let mut app = app_with_projects(3);
        app.project_selected = 3;
        let keep = app.projects[2].clone();
        let projects = vec![
            orchestra_core::model::Project {
                id: keep.id,
                name: keep.name.clone(),
                path: keep.path.clone().into(),
                default_branch: "main".into(),
                zellij_tab: None,
                kind: orchestra_core::model::ProjectKind::Managed,
                created_at: orchestra_core::now(),
            },
            orchestra_core::model::Project {
                id: Uuid::new_v4(),
                name: "autre".into(),
                path: "/tmp/autre".into(),
                default_branch: "main".into(),
                zellij_tab: None,
                kind: orchestra_core::model::ProjectKind::Managed,
                created_at: orchestra_core::now(),
            },
        ];
        app.update(Msg::Reply(Box::new(Reply::Projects { projects })));
        assert_eq!(app.selected_project().unwrap().id, keep.id);
    }

    #[test]
    fn every_screen_exists_now() {
        // Nothing announces a future phase any more.
        for n in 1..=6 {
            let mut app = App::new();
            app.update(Msg::Key(Action::Screen(n)));
            assert!(app.screen.is_implemented(), "écran {n}");
            assert!(!app.status.contains("phase"), "écran {n} : {}", app.status);
        }
    }

    #[test]
    fn the_cost_screen_reacts_to_its_own_keys() {
        let mut app = App::new();
        app.update(Msg::Key(Action::Screen(4)));

        let cmds = app.update(Msg::Key(Action::Char('m')));
        assert_eq!(app.cost.group, GroupBy::Ticket, "m change le regroupement");
        assert_eq!(cmds.len(), 2, "le tableau et la courbe sont redemandés");

        app.update(Msg::Key(Action::Char('p')));
        assert_eq!(app.cost.period, Period::Month, "p change la période");

        assert!(app.cost.include_unmanaged);
        app.update(Msg::Key(Action::Char('u')));
        assert!(!app.cost.include_unmanaged, "u bascule les sessions libres");

        // Those keys do nothing on the board.
        app.update(Msg::Key(Action::Screen(1)));
        let before = app.cost.group;
        app.update(Msg::Key(Action::Char('m')));
        assert_eq!(app.cost.group, before);
    }

    #[test]
    fn the_period_defines_the_window_it_asks_for() {
        let mut app = App::new();
        app.cost.period = Period::All;
        let [table, trend] = app.cost.queries();
        assert!(table.range.since.is_none(), "« tout » ne borne pas");
        assert_eq!(trend.group_by, vec![GroupBy::Day]);

        app.cost.period = Period::Today;
        let [table, _] = app.cost.queries();
        let since = table.range.since.expect("aujourd'hui commence à minuit");
        assert_eq!(since.time(), time::Time::MIDNIGHT);
    }

    #[test]
    fn the_trend_reply_is_told_apart_from_the_table() {
        use std::collections::BTreeMap;
        let mut app = App::new();
        let mut daily_keys = BTreeMap::new();
        daily_keys.insert("day".to_string(), "2026-09-19".to_string());
        let daily = UsageRow {
            keys: daily_keys,
            tokens: Default::default(),
            cost_usd: Some(3.0),
            messages: 1,
            cost_estimated: false,
        };
        let mut table_keys = BTreeMap::new();
        table_keys.insert("project".to_string(), "orchestra".to_string());
        let table = UsageRow {
            keys: table_keys,
            tokens: Default::default(),
            cost_usd: Some(9.0),
            messages: 2,
            cost_estimated: false,
        };

        app.update(Msg::Reply(Box::new(Reply::Usage {
            rows: vec![daily],
            totals: UsageTotals::default(),
        })));
        app.update(Msg::Reply(Box::new(Reply::Usage {
            rows: vec![table],
            totals: UsageTotals {
                cost_usd: Some(9.0),
                ..Default::default()
            },
        })));

        assert_eq!(app.cost.daily.len(), 1, "la courbe est rangée à part");
        assert_eq!(app.cost.rows.len(), 1, "le tableau n'est pas écrasé");
        assert_eq!(app.cost.totals.cost_usd, Some(9.0));
    }

    #[test]
    fn every_screen_refreshes_itself_on_the_tick() {
        // The loop used to only repaint on the tick, without handing it to the
        // app: nothing was polled, so a status changed elsewhere was only seen
        // by leaving the screen and coming back.
        let mut app = App::new();
        let cmds = app.update(Msg::Tick);
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Command::ListTickets { .. })),
            "le tableau interroge sa liste à chaque seconde : {cmds:?}"
        );

        // And the counters that turn on their own advance with it.
        let before = app.ticks;
        app.update(Msg::Tick);
        assert_eq!(app.ticks, before + 1);
    }

    #[test]
    fn the_cost_view_is_polled_rather_than_event_driven() {
        let mut app = App::new();
        app.update(Msg::Key(Action::Screen(4)));
        // One tick is not enough; the second asks.
        assert!(app.update(Msg::Tick).is_empty());
        let cmds = app.update(Msg::Tick);
        assert_eq!(cmds.len(), 2);

        // A usage event from a managed agent does not trigger its own query.
        let e = Event::from_new(
            1,
            NewEvent::new(EventKind::Usage {
                sample: Box::new(orchestra_core::model::UsageSample {
                    message_id: "m".into(),
                    session_id: Uuid::new_v4(),
                    subagent_id: None,
                    agent_id: None,
                    ticket_id: None,
                    project_id: None,
                    model: "claude-opus-5".into(),
                    tokens: Default::default(),
                    ts: orchestra_core::now(),
                    source: orchestra_core::model::UsageSource::Stream,
                }),
            }),
        );
        assert!(app.update(Msg::Event(Box::new(e))).is_empty());
    }
}
