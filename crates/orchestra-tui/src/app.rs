//! Application state, Elm style: `App` is pure data, `update` folds messages
//! into it, and rendering only reads it. No ratatui type appears here, so the
//! whole state machine is testable without a terminal.

use orchestra_core::events::{Event, EventKind};
use orchestra_core::model::{ProjectId, RoleDefinition, TicketId};
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
}

impl Screen {
    pub const ALL: [Screen; 6] = [
        Screen::Board,
        Screen::Ticket,
        Screen::Agent,
        Screen::Cost,
        Screen::NewTicket,
        Screen::Proposal,
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
    pub form: NewTicketForm,
    pub form_field: TicketField,
    pub editor: TeamEditor,
    /// Index of the agent being watched, within the open ticket.
    pub agent_selected: usize,
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
    pub tickets: Vec<TicketSummary>,
    pub ticket_selected: usize,
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
            form: NewTicketForm::default(),
            form_field: TicketField::Title,
            editor: TeamEditor::default(),
            agent_selected: 0,
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
            tickets: Vec::new(),
            ticket_selected: 0,
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

    pub fn selected_project(&self) -> Option<&ProjectRow> {
        self.projects.get(self.project_selected)
    }

    pub fn selected_ticket(&self) -> Option<&TicketSummary> {
        self.tickets.get(self.ticket_selected)
    }

    pub fn selected_ticket_id(&self) -> Option<TicketId> {
        self.selected_ticket().map(|t| t.ticket.id)
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
        if !self.ticks.is_multiple_of(2) {
            return;
        }
        match self.screen {
            Screen::Cost => self.request_usage(),
            Screen::Board => {
                self.request_usage();
                self.request_tickets();
            }
            // While the orchestrator works there is nothing to stream, so the
            // ticket is polled until its proposal lands.
            Screen::Ticket if self.planning => self.refresh_open_ticket(None),
            // The log streams, but the agent's status and cost come from the
            // ticket: without this the header stays on "démarrage" while the
            // agent is plainly working.
            Screen::Agent => self.refresh_open_ticket(None),
            Screen::Ticket => self.refresh_open_ticket(None),
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
            Action::Left => self.board_pane = BoardPane::Projects,
            Action::Right => self.board_pane = BoardPane::Tickets,
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
                self.outbox.push(Command::CreateTicket {
                    project_id: project,
                    title,
                    brief,
                });
                self.form.clear();
                self.screen = Screen::Board;
                self.status = "ticket créé".into();
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
            _ => {}
        }
    }

    fn on_board_char(&mut self, c: char) {
        if c == 'n' {
            self.open_new_ticket();
        }
    }

    fn open_new_ticket(&mut self) {
        if self.selected_project().is_none() {
            self.status = "ajoute d'abord un projet : `:project add <chemin>`".into();
            return;
        }
        self.form.clear();
        self.form_field = TicketField::Title;
        self.screen = Screen::NewTicket;
    }

    fn on_ticket_char(&mut self, c: char) {
        let Some(detail) = self.ticket.as_ref() else {
            return;
        };
        let ticket_id = detail.ticket.id;
        match c {
            'p' => {
                self.outbox.push(Command::PlanTicket { ticket_id });
                self.planning = true;
                self.status = "l'orchestrateur compose l'équipe…".into();
            }
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
            _ => {}
        }
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
        // An agent the user picked deliberately is left alone.
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
                if let Some(detail) = self.ticket.as_ref() {
                    self.outbox.push(Command::PlanTicket {
                        ticket_id: detail.ticket.id,
                    });
                    self.planning = true;
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
        self.screen = screen;
        if entering_agent && self.watched_agent().is_none() {
            // Reached from the tab strip rather than from a ticket: find the
            // agent that is working, wherever it is.
            self.outbox.push(Command::ListAgents { only_active: true });
            self.status = "recherche d'un agent en cours…".into();
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
                (self.projects.len(), &mut self.project_selected)
            }
            (Screen::Board, BoardPane::Tickets) => (self.tickets.len(), &mut self.ticket_selected),
            (Screen::Cost, _) => (self.cost.rows.len(), &mut self.cost.selected),
            (Screen::Proposal, _) => {
                self.editor.move_selection(delta);
                return;
            }
            (Screen::Ticket, _) => {
                let count = self.ticket.as_ref().map(|d| d.agents.len()).unwrap_or(0);
                if count > 0 {
                    let next = (self.agent_selected as isize + delta).clamp(0, count as isize - 1);
                    self.agent_selected = next as usize;
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
                (self.projects.len(), &mut self.project_selected)
            }
            (Screen::Board, BoardPane::Tickets) => (self.tickets.len(), &mut self.ticket_selected),
            (Screen::Cost, _) => (self.cost.rows.len(), &mut self.cost.selected),
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
                if sub == "add" && !arg.is_empty() {
                    self.outbox.push(Command::AddProject {
                        path: arg.into(),
                        name: None,
                    });
                    self.status = format!("ajout du projet {arg}…");
                } else {
                    self.status = "usage : :project add <chemin>".into();
                }
            }
            "usage" | "cout" | "coût" => self.go(Screen::Cost),
            "refresh" => self.refresh(),
            "q" | "quit" => self.should_quit = true,
            "" => {}
            other => self.status = format!("commande inconnue : {other}"),
        }
    }

    /// Take the commands queued by the last update.
    pub fn take_outbox(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.outbox)
    }

    /// Ask the daemon for everything the current screen shows.
    pub fn refresh(&mut self) {
        self.outbox.push(Command::ListProjects);
        self.outbox.push(Command::ListRoles {
            project_id: self.selected_project().map(|p| p.id),
        });
        self.request_tickets();
        self.request_usage();
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
                self.projects = projects
                    .into_iter()
                    .map(|p| ProjectRow {
                        id: p.id,
                        name: p.name,
                        path: p.path.display().to_string(),
                        discovered: p.kind == orchestra_core::model::ProjectKind::Discovered,
                    })
                    .collect();
                // Keep the highlight on the same project across refreshes.
                self.project_selected = previous
                    .and_then(|id| self.projects.iter().position(|p| p.id == id))
                    .unwrap_or(0)
                    .min(self.projects.len().saturating_sub(1));
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
                self.planning = false;
                let previous = self.watched_agent_id();
                self.ticket = Some(detail);
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
                // While watching, follow the team: when one agent hands over to
                // the next, the screen moves with it.
                if self.screen == Screen::Agent
                    && !self
                        .watched_agent()
                        .is_some_and(|a| a.agent.status.is_active())
                {
                    let previous = self.watched_agent_id();
                    self.select_liveliest_agent();
                    if self.watched_agent_id() != previous {
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
            Reply::Roles { roles } => {
                self.roles = roles;
            }
            Reply::Agents { agents } => self.adopt_live_agent(agents),
            Reply::Status { status } => {
                self.daemon_version = Some(status.version.clone());
            }
            Reply::Pong | Reply::Ack | Reply::Subscribed { .. } => {}
            _ => {}
        }
    }

    fn on_event(&mut self, e: Event) {
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
            EventKind::ProjectAdded { .. } | EventKind::UnmanagedSessionSeen { .. } => {
                self.outbox.push(Command::ListProjects);
            }
            EventKind::TicketCreated { .. }
            | EventKind::TicketStatusChanged { .. }
            | EventKind::AgentStatusChanged { .. } => {
                self.request_tickets();
                self.refresh_open_ticket(e.ticket_id);
            }
            EventKind::ProposalReady { .. } => {
                self.planning = false;
                self.status = "proposition prête — « a » pour la relire".into();
                self.refresh_open_ticket(e.ticket_id);
            }
            EventKind::ProposalFailed { error } => {
                self.planning = false;
                self.status = format!("planification échouée : {error}");
            }
            // A managed agent reports every response. The cost view is polled
            // instead, so a busy agent cannot spin the loop with queries.
            EventKind::Usage { .. } => {}
            _ => {}
        }
    }
}

/// One activity line for an event, or `None` when it is not worth showing.
pub fn describe(e: &Event) -> Option<String> {
    let t = e.ts.time();
    let stamp = format!("{:02}:{:02}:{:02}", t.hour(), t.minute(), t.second());
    let body = match &e.kind {
        EventKind::DaemonStarted { version } => format!("daemon {version} démarré"),
        EventKind::ProjectAdded { name, .. } => format!("projet « {name} » ajouté"),
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
        EventKind::HookBlocked { tool, reason } => format!("{tool} bloqué : {reason}"),
        EventKind::UnmanagedSessionSeen { cwd, .. } => {
            format!("session libre détectée dans {}", cwd.display())
        }
        EventKind::Warning { message } => format!("attention : {message}"),
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
        assert_eq!(Screen::Board.prev(), Screen::Proposal);
        assert_eq!(Screen::from_number(1), Some(Screen::Board));
        assert_eq!(Screen::from_number(6), Some(Screen::Proposal));
        assert_eq!(Screen::from_number(0), None);
        assert_eq!(Screen::from_number(7), None);
    }

    #[test]
    fn selection_stays_in_bounds() {
        let mut app = app_with_projects(3);
        app.update(Msg::Key(Action::Up));
        assert_eq!(app.project_selected, 0);
        for _ in 0..10 {
            app.update(Msg::Key(Action::Down));
        }
        assert_eq!(app.project_selected, 2);
        app.update(Msg::Key(Action::Top));
        assert_eq!(app.project_selected, 0);
        app.update(Msg::Key(Action::Bottom));
        assert_eq!(app.project_selected, 2);
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
        app.project_selected = 2;
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
