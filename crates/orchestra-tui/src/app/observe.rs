//! The observer screen: series, alerts and the trace of one ticket. Its keys,
//! its requests, and what the replies change.

use orchestra_core::observe::{AlertEntry, Firing, Series, SeriesQuery, Trace};

use super::*;

/// The three readings of the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserveTab {
    Metrics,
    Alerts,
    Trace,
}

impl ObserveTab {
    pub const ALL: [ObserveTab; 3] = [ObserveTab::Metrics, ObserveTab::Alerts, ObserveTab::Trace];

    pub fn label_fr(self) -> &'static str {
        match self {
            ObserveTab::Metrics => "métriques",
            ObserveTab::Alerts => "alertes",
            ObserveTab::Trace => "trace",
        }
    }
}

/// How far back the series reaches, and how wide a bucket is.
pub const WINDOWS: [(&str, u32, u32); 4] = [
    ("1 h", 1, 1),
    ("6 h", 6, 5),
    ("24 h", 24, 30),
    ("7 jours", 168, 180),
];

#[derive(Debug, Clone)]
pub struct ObserveView {
    pub tab: ObserveTab,
    pub window: usize,
    pub series: Option<Series>,
    pub active: Vec<Firing>,
    pub history: Vec<AlertEntry>,
    /// How many monitors `config.toml` defines; none explains an empty tab.
    pub monitors: u32,
    /// The ticket whose steps the trace tab reads.
    pub trace_for: Option<TicketId>,
    pub trace: Option<Trace>,
}

impl Default for ObserveView {
    fn default() -> Self {
        ObserveView {
            tab: ObserveTab::Metrics,
            window: 2,
            series: None,
            active: Vec::new(),
            history: Vec::new(),
            monitors: 0,
            trace_for: None,
            trace: None,
        }
    }
}

impl ObserveView {
    pub fn window_label(&self) -> &'static str {
        WINDOWS[self.window % WINDOWS.len()].0
    }

    pub fn series_query(&self, project_id: Option<orchestra_core::model::ProjectId>) -> SeriesQuery {
        let (_, hours, step_minutes) = WINDOWS[self.window % WINDOWS.len()];
        SeriesQuery { hours, step_minutes, project_id }
    }
}

impl App {
    pub(super) fn on_observe_char(&mut self, c: char) {
        match c {
            'm' => self.observe.tab = ObserveTab::Metrics,
            'a' => self.observe.tab = ObserveTab::Alerts,
            't' => {
                self.observe.tab = ObserveTab::Trace;
                self.observe_follow_ticket();
            }
            'p' => {
                self.observe.window = (self.observe.window + 1) % WINDOWS.len();
                self.observe.series = None;
            }
            _ => return,
        }
        self.request_observe();
    }

    /// Open the observer on a tab. From a ticket, the trace is that
    /// ticket's; from anywhere else it is the one the board has under the
    /// cursor.
    pub(super) fn open_observer(&mut self, tab: ObserveTab) {
        self.observe.tab = tab;
        self.go(Screen::Observe);
        if tab == ObserveTab::Trace {
            self.observe_follow_ticket();
        }
        self.request_observe();
    }

    fn observe_follow_ticket(&mut self) {
        let id = self
            .ticket
            .as_ref()
            .map(|d| d.ticket.id)
            .or_else(|| self.selected_ticket_id());
        // Nothing in view keeps the last ticket read rather than blanking it.
        let Some(id) = id else { return };
        if Some(id) != self.observe.trace_for {
            self.observe.trace = None;
        }
        self.observe.trace_for = Some(id);
    }

    /// What the open tab needs. The monitors are asked for on every tab:
    /// the header says how many stand.
    pub(super) fn request_observe(&mut self) {
        let project_id = self.selected_project().map(|p| p.id);
        self.outbox.push(Command::GetAlerts);
        match self.observe.tab {
            ObserveTab::Metrics => self.outbox.push(Command::GetSeries {
                query: self.observe.series_query(project_id),
            }),
            ObserveTab::Alerts => {}
            ObserveTab::Trace => match self.observe.trace_for {
                Some(ticket_id) => self.outbox.push(Command::GetTrace { ticket_id }),
                None => self.status = "ouvre un ticket pour lire sa trace".into(),
            },
        }
    }

    pub(super) fn adopt_alerts(&mut self, active: Vec<Firing>, recent: Vec<AlertEntry>, monitors: u32) {
        self.observe.active = active;
        self.observe.history = recent;
        self.observe.monitors = monitors;
    }

    pub(super) fn adopt_trace(&mut self, ticket_id: TicketId, trace: Trace) {
        // A reply for a ticket the user has since left is not what they look at.
        if self.observe.trace_for == Some(ticket_id) {
            self.observe.trace = Some(trace);
        }
    }
}
