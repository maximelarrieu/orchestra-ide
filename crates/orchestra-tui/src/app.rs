//! Application state, Elm style: `App` is pure data, `update` folds messages
//! into it, and rendering only reads it. No ratatui type appears here, so the
//! whole state machine is testable without a terminal.

use orchestra_core::events::{Event, EventKind};
use orchestra_core::model::{ProjectId, TicketId};
use orchestra_core::protocol::{Reply, TicketSummary, UsageRow, UsageTotals};

use crate::keymap::Action;

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

    /// Phase 0 ships the board only; the rest announce themselves.
    pub fn is_implemented(self) -> bool {
        matches!(self, Screen::Board)
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

#[derive(Debug, Clone, Default)]
pub struct CostView {
    pub rows: Vec<UsageRow>,
    pub totals: UsageTotals,
    pub selected: usize,
}

pub struct App {
    pub screen: Screen,
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
    pub outbox: Vec<orchestra_core::protocol::Command>,
}

const ACTIVITY_MAX: usize = 200;

impl Default for App {
    fn default() -> Self {
        App {
            screen: Screen::Board,
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

    /// Fold one message into the state. Returns commands to send, if any.
    pub fn update(&mut self, msg: Msg) -> Vec<orchestra_core::protocol::Command> {
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
            Msg::Tick => {}
        }
        std::mem::take(&mut self.outbox)
    }

    fn on_key(&mut self, action: Action) {
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

        match action {
            Action::Quit => self.should_quit = true,
            Action::Back => self.should_quit = true,
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
            Action::Bottom => self.set_selection(usize::MAX),
            Action::Select => self.open_selection(),
            _ => {}
        }
    }

    fn go(&mut self, screen: Screen) {
        self.screen = screen;
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
            (Screen::Board, BoardPane::Tickets) if self.selected_ticket().is_some() => {
                self.go(Screen::Ticket);
            }
            _ => {}
        }
    }

    fn run_palette(&mut self, line: &str) {
        use orchestra_core::protocol::Command;
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
    pub fn take_outbox(&mut self) -> Vec<orchestra_core::protocol::Command> {
        std::mem::take(&mut self.outbox)
    }

    /// Ask the daemon for everything the current screen shows.
    pub fn refresh(&mut self) {
        use orchestra_core::protocol::{Command, UsageQuery};
        self.outbox.push(Command::ListProjects);
        self.request_tickets();
        self.outbox.push(Command::GetUsage {
            query: UsageQuery::default(),
        });
    }

    fn request_tickets(&mut self) {
        use orchestra_core::protocol::Command;
        self.outbox.push(Command::ListTickets {
            project_id: self.selected_project().map(|p| p.id),
            status: None,
        });
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
                self.outbox
                    .push(orchestra_core::protocol::Command::ListProjects);
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
                self.cost.rows = rows;
                self.cost.totals = totals;
                self.cost.selected = self
                    .cost
                    .selected
                    .min(self.cost.rows.len().saturating_sub(1));
            }
            Reply::Status { status } => {
                self.daemon_version = Some(status.version.clone());
            }
            Reply::Pong | Reply::Ack | Reply::Subscribed { .. } => {}
            _ => {}
        }
    }

    fn on_event(&mut self, e: Event) {
        if let Some(line) = describe(&e) {
            self.activity.push(line);
            if self.activity.len() > ACTIVITY_MAX {
                self.activity.drain(..self.activity.len() - ACTIVITY_MAX);
            }
        }
        // Anything that changes a board number triggers a targeted refresh.
        match &e.kind {
            EventKind::ProjectAdded { .. } => {
                self.outbox
                    .push(orchestra_core::protocol::Command::ListProjects);
            }
            EventKind::TicketCreated { .. }
            | EventKind::TicketStatusChanged { .. }
            | EventKind::AgentStatusChanged { .. } => self.request_tickets(),
            EventKind::Usage { .. } => {
                self.outbox
                    .push(orchestra_core::protocol::Command::GetUsage {
                        query: orchestra_core::protocol::UsageQuery::default(),
                    });
            }
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
    fn unimplemented_screens_say_so() {
        let mut app = App::new();
        app.update(Msg::Key(Action::Screen(4)));
        assert_eq!(app.screen, Screen::Cost);
        assert!(app.status.contains("phase"), "{}", app.status);
    }
}
