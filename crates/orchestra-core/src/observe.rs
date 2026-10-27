//! Observability: series over time, monitors, and the trace of a ticket.
//!
//! Everything here is a pure reading of what the daemon already recorded —
//! usage samples and events. Nothing is sampled on the side: a figure shown
//! by the observer is either an `Event` or a query on the database (rule 7).
//! The daemon fetches, these functions decide.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::{Event, EventKind};
use crate::model::{AgentStatus, ProjectId};

// ---------------------------------------------------------------------------
// Series
// ---------------------------------------------------------------------------

/// Most buckets one series may hold: two weeks of hours.
pub const MAX_BUCKETS: u32 = 336;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SeriesQuery {
    /// How far back the series reaches, ending now.
    pub hours: u32,
    /// Width of one bucket.
    pub step_minutes: u32,
    pub project_id: Option<ProjectId>,
}

impl Default for SeriesQuery {
    fn default() -> Self {
        SeriesQuery {
            hours: 24,
            step_minutes: 60,
            project_id: None,
        }
    }
}

impl SeriesQuery {
    /// The query with its numbers brought into range: a step of zero would
    /// divide by it, and a thousand buckets would not fit on any screen.
    pub fn clamped(&self) -> SeriesQuery {
        let step_minutes = self.step_minutes.max(1);
        let hours = self.hours.max(1);
        let buckets = (u64::from(hours) * 60)
            .div_ceil(u64::from(step_minutes))
            .min(u64::from(MAX_BUCKETS));
        SeriesQuery {
            hours: (buckets * u64::from(step_minutes)).div_ceil(60) as u32,
            step_minutes,
            project_id: self.project_id,
        }
    }

    pub fn step_secs(&self) -> i64 {
        i64::from(self.step_minutes.max(1)) * 60
    }

    pub fn buckets(&self) -> usize {
        let n = (u64::from(self.hours.max(1)) * 60).div_ceil(u64::from(self.step_minutes.max(1)));
        n.min(u64::from(MAX_BUCKETS)) as usize
    }
}

/// What was spent, bucket by bucket, oldest first. The last bucket is the
/// one in progress. Tokens are the truth; `cost_usd` is indicative.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Series {
    /// Unix time at which the first bucket starts.
    pub start_unix: i64,
    pub step_secs: i64,
    pub tokens: Vec<u64>,
    pub cost_usd: Vec<f64>,
    pub messages: Vec<u64>,
}

impl Series {
    /// Empty buckets covering `query`, the last one holding `now`.
    pub fn empty(query: &SeriesQuery, now: OffsetDateTime) -> Self {
        let step = query.step_secs();
        let n = query.buckets();
        let last = now.unix_timestamp().div_euclid(step) * step;
        Series {
            start_unix: last - step * (n as i64 - 1),
            step_secs: step,
            tokens: vec![0; n],
            cost_usd: vec![0.0; n],
            messages: vec![0; n],
        }
    }

    /// Where `unix` falls, or `None` when the series does not reach it.
    pub fn index_of(&self, unix: i64) -> Option<usize> {
        if self.step_secs <= 0 || unix < self.start_unix {
            return None;
        }
        let i = ((unix - self.start_unix) / self.step_secs) as usize;
        (i < self.tokens.len()).then_some(i)
    }

    pub fn add(&mut self, unix: i64, tokens: u64, cost_usd: f64, messages: u64) {
        if let Some(i) = self.index_of(unix) {
            self.tokens[i] += tokens;
            self.cost_usd[i] += cost_usd;
            self.messages[i] += messages;
        }
    }

    /// The last `n` buckets summed — « the last hour » for a step of 15 min.
    pub fn tail_sum(&self, n: usize) -> (u64, f64) {
        let from = self.tokens.len().saturating_sub(n);
        (
            self.tokens[from..].iter().sum(),
            self.cost_usd[from..].iter().sum(),
        )
    }
}

// ---------------------------------------------------------------------------
// Monitors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorKind {
    /// Indicative dollars spent over the last hour.
    CostPerHour,
    /// Tokens, input and output and cache, over the last hour.
    TokensPerHour,
    /// Agents silent right now (`AgentStalled`, not yet spoken again).
    Stalled,
    /// Agents that failed or crashed over the last hour.
    Failures,
}

impl MonitorKind {
    pub fn label_fr(self) -> &'static str {
        match self {
            MonitorKind::CostPerHour => "coût sur l'heure",
            MonitorKind::TokensPerHour => "tokens sur l'heure",
            MonitorKind::Stalled => "agents silencieux",
            MonitorKind::Failures => "échecs sur l'heure",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            MonitorKind::CostPerHour => "$",
            MonitorKind::TokensPerHour => "tokens",
            MonitorKind::Stalled | MonitorKind::Failures => "",
        }
    }
}

/// One `[[monitors]]` entry of `config.toml`. It fires when the measure is
/// strictly above `threshold`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonitorRule {
    pub name: String,
    pub kind: MonitorKind,
    pub threshold: f64,
}

/// The measures a monitor reads, taken at one instant.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Observation {
    pub tokens_last_hour: u64,
    /// `None` when no model of the hour is in the price grid.
    pub cost_last_hour: Option<f64>,
    pub stalled_agents: u32,
    pub failures_last_hour: u32,
}

impl Observation {
    pub fn measure(&self, kind: MonitorKind) -> Option<f64> {
        match kind {
            MonitorKind::CostPerHour => self.cost_last_hour,
            MonitorKind::TokensPerHour => Some(self.tokens_last_hour as f64),
            MonitorKind::Stalled => Some(f64::from(self.stalled_agents)),
            MonitorKind::Failures => Some(f64::from(self.failures_last_hour)),
        }
    }
}

/// A monitor over its threshold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Firing {
    pub monitor: String,
    pub kind: MonitorKind,
    pub value: f64,
    pub threshold: f64,
}

impl Firing {
    /// `coût sur l'heure : 4,20 $ pour un seuil de 3,00 $`
    pub fn line_fr(&self) -> String {
        let unit = match self.kind.unit() {
            "" => String::new(),
            u => format!(" {u}"),
        };
        let figure = |v: f64| match self.kind {
            MonitorKind::CostPerHour => format!("{v:.2}").replace('.', ","),
            _ => format!("{v:.0}"),
        };
        format!(
            "{} : {}{unit} pour un seuil de {}{unit}",
            self.kind.label_fr(),
            figure(self.value),
            figure(self.threshold)
        )
    }
}

/// The rules the observation puts over their threshold. A measure that cannot
/// be taken (a price grid without the model) never fires: silence is not an
/// alert, and the rule stays quiet until it can be read.
pub fn evaluate(rules: &[MonitorRule], obs: &Observation) -> Vec<Firing> {
    rules
        .iter()
        .filter_map(|r| {
            let value = obs.measure(r.kind)?;
            (value > r.threshold).then(|| Firing {
                monitor: r.name.clone(),
                kind: r.kind,
                value,
                threshold: r.threshold,
            })
        })
        .collect()
}

/// What changed between the alerts standing and the ones firing now:
/// those to raise, and the names of those that cleared. A rule that stays
/// over its threshold raises nothing twice.
pub fn diff(
    standing: &BTreeMap<String, Firing>,
    now: &[Firing],
) -> (Vec<Firing>, Vec<String>) {
    let raised = now
        .iter()
        .filter(|f| !standing.contains_key(&f.monitor))
        .cloned()
        .collect();
    let resolved = standing
        .keys()
        .filter(|name| !now.iter().any(|f| &f.monitor == *name))
        .cloned()
        .collect();
    (raised, resolved)
}

/// One line of an alert's history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertEntry {
    #[serde(with = "time::serde::rfc3339")]
    pub ts: OffsetDateTime,
    pub monitor: String,
    /// True when it was raised, false when it cleared.
    pub raised: bool,
    /// The reading at the time, for a raise.
    #[serde(default)]
    pub firing: Option<Firing>,
}

/// The standing alerts and the history, read back from the events — the
/// daemon keeps no state of its own about them, so a restart forgets nothing.
pub fn replay_alerts(events: &[Event]) -> (BTreeMap<String, Firing>, Vec<AlertEntry>) {
    let mut standing = BTreeMap::new();
    let mut history = Vec::new();
    for e in events {
        match &e.kind {
            EventKind::AlertRaised {
                monitor,
                monitor_kind,
                value,
                threshold,
            } => {
                let firing = Firing {
                    monitor: monitor.clone(),
                    kind: *monitor_kind,
                    value: *value,
                    threshold: *threshold,
                };
                standing.insert(monitor.clone(), firing.clone());
                history.push(AlertEntry {
                    ts: e.ts,
                    monitor: monitor.clone(),
                    raised: true,
                    firing: Some(firing),
                });
            }
            EventKind::AlertResolved { monitor } => {
                standing.remove(monitor);
                history.push(AlertEntry {
                    ts: e.ts,
                    monitor: monitor.clone(),
                    raised: false,
                    firing: None,
                });
            }
            _ => {}
        }
    }
    (standing, history)
}

// ---------------------------------------------------------------------------
// Trace
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanKind {
    Planning,
    Agent,
    Check,
    Review,
    Merge,
}

impl SpanKind {
    pub fn label_fr(self) -> &'static str {
        match self {
            SpanKind::Planning => "proposition",
            SpanKind::Agent => "agent",
            SpanKind::Check => "vérification",
            SpanKind::Review => "relecture",
            SpanKind::Merge => "fusion",
        }
    }
}

/// One step of a ticket's life. `end` is `None` while it runs; a step that is
/// a moment (a verdict, a merge) starts and ends together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub kind: SpanKind,
    pub label: String,
    #[serde(with = "time::serde::rfc3339")]
    pub start: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub end: Option<OffsetDateTime>,
    /// `None` while it runs.
    #[serde(default)]
    pub ok: Option<bool>,
}

impl Span {
    pub fn duration_ms(&self, now: OffsetDateTime) -> u64 {
        let end = self.end.unwrap_or(now);
        (end - self.start).whole_milliseconds().max(0) as u64
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Trace {
    pub spans: Vec<Span>,
}

impl Trace {
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// First start and last end (or `now` for what still runs).
    pub fn bounds(&self, now: OffsetDateTime) -> Option<(OffsetDateTime, OffsetDateTime)> {
        let start = self.spans.iter().map(|s| s.start).min()?;
        let end = self
            .spans
            .iter()
            .map(|s| s.end.unwrap_or(now))
            .max()
            .unwrap_or(start);
        Some((start, end))
    }
}

fn terminal(status: AgentStatus) -> Option<bool> {
    match status {
        AgentStatus::Done => Some(true),
        AgentStatus::Failed | AgentStatus::Crashed | AgentStatus::Cancelled => Some(false),
        _ => None,
    }
}

/// The steps of one ticket, in the order they began, from its events.
///
/// An agent that is resumed after a crash opens a new step: the gap between
/// the two is the time lost, and the trace is there to show it.
pub fn build_trace(events: &[Event]) -> Trace {
    let mut spans: Vec<Span> = Vec::new();
    let mut created: Option<OffsetDateTime> = None;
    // Index in `spans` of the step an agent or a check has open.
    let mut open_agent: BTreeMap<crate::model::AgentId, usize> = BTreeMap::new();
    let mut open_check: Vec<(u32, String, usize)> = Vec::new();

    for e in events {
        match &e.kind {
            EventKind::TicketCreated { .. } => created = created.or(Some(e.ts)),
            EventKind::ProposalReady { .. } => {
                if let Some(start) = created.take() {
                    spans.push(Span {
                        kind: SpanKind::Planning,
                        label: "équipe proposée".into(),
                        start,
                        end: Some(e.ts),
                        ok: Some(true),
                    });
                }
            }
            EventKind::ProposalFailed { .. } => {
                if let Some(start) = created.take() {
                    spans.push(Span {
                        kind: SpanKind::Planning,
                        label: "proposition échouée".into(),
                        start,
                        end: Some(e.ts),
                        ok: Some(false),
                    });
                }
            }
            EventKind::AgentSpawned { role, .. } => {
                if let Some(agent) = e.agent_id {
                    open_agent.insert(agent, spans.len());
                    spans.push(Span {
                        kind: SpanKind::Agent,
                        label: role.clone(),
                        start: e.ts,
                        end: None,
                        ok: None,
                    });
                }
            }
            EventKind::AgentStatusChanged { status, .. } => {
                if let (Some(agent), Some(ok)) = (e.agent_id, terminal(*status)) {
                    if let Some(i) = open_agent.remove(&agent) {
                        spans[i].end = Some(e.ts);
                        spans[i].ok = Some(ok);
                    }
                }
            }
            EventKind::CheckStarted { round, command } => {
                open_check.push((*round, command.clone(), spans.len()));
                spans.push(Span {
                    kind: SpanKind::Check,
                    label: command.clone(),
                    start: e.ts,
                    end: None,
                    ok: None,
                });
            }
            EventKind::CheckFinished { round, run } => {
                let found = open_check
                    .iter()
                    .position(|(r, c, _)| r == round && c == &run.command);
                if let Some(p) = found {
                    let (_, _, i) = open_check.remove(p);
                    spans[i].end = Some(e.ts);
                    spans[i].ok = Some(run.ok);
                }
            }
            EventKind::ReviewVerdict { round, verdict, .. } => spans.push(Span {
                kind: SpanKind::Review,
                label: format!("tour {round} : {}", verdict.label_fr()),
                start: e.ts,
                end: Some(e.ts),
                ok: Some(verdict.is_ready()),
            }),
            EventKind::TicketMerged { into, .. } => spans.push(Span {
                kind: SpanKind::Merge,
                label: format!("dans {into}"),
                start: e.ts,
                end: Some(e.ts),
                ok: Some(true),
            }),
            EventKind::MergeBlocked { reason, .. } => spans.push(Span {
                kind: SpanKind::Merge,
                label: format!("bloquée : {reason}"),
                start: e.ts,
                end: Some(e.ts),
                ok: Some(false),
            }),
            _ => {}
        }
    }
    Trace { spans }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::CheckRun;
    use crate::events::NewEvent;
    use crate::model::AgentId;
    use crate::review::Verdict;
    use time::Duration;
    use uuid::Uuid;

    fn at(secs: i64) -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH + Duration::seconds(secs)
    }

    fn ev(secs: i64, agent: Option<AgentId>, kind: EventKind) -> Event {
        let mut n = NewEvent::new(kind);
        n.ts = at(secs);
        n.agent_id = agent;
        Event::from_new(secs, n)
    }

    #[test]
    fn a_query_is_brought_into_range() {
        let q = SeriesQuery { hours: 0, step_minutes: 0, project_id: None }.clamped();
        assert_eq!((q.hours, q.step_minutes), (1, 1));
        let huge = SeriesQuery { hours: 100_000, step_minutes: 1, project_id: None }.clamped();
        assert_eq!(huge.buckets() as u32, MAX_BUCKETS);
        assert_eq!(SeriesQuery::default().buckets(), 24);
    }

    #[test]
    fn a_series_puts_samples_in_their_bucket_and_drops_the_rest() {
        let q = SeriesQuery { hours: 3, step_minutes: 60, project_id: None };
        let mut s = Series::empty(&q, at(3 * 3600 + 120));
        assert_eq!(s.tokens.len(), 3);
        // The last bucket holds `now`; the first starts two hours before it.
        assert_eq!(s.start_unix, 3600);
        s.add(3600 + 5, 10, 0.5, 1);
        s.add(3 * 3600 + 10, 7, 0.25, 2);
        s.add(3 * 3600 + 11, 1, 0.0, 1);
        s.add(0, 999, 9.0, 9); // before the window
        s.add(4 * 3600 + 5, 999, 9.0, 9); // after it
        assert_eq!(s.tokens, vec![10, 0, 8]);
        assert_eq!(s.messages, vec![1, 0, 3]);
        assert_eq!(s.tail_sum(1), (8, 0.25));
        assert_eq!(s.tail_sum(10).0, 18);
    }

    fn rules() -> Vec<MonitorRule> {
        vec![
            MonitorRule { name: "budget".into(), kind: MonitorKind::CostPerHour, threshold: 3.0 },
            MonitorRule { name: "silence".into(), kind: MonitorKind::Stalled, threshold: 0.0 },
        ]
    }

    #[test]
    fn a_monitor_fires_strictly_above_its_threshold() {
        let mut obs = Observation { cost_last_hour: Some(3.0), ..Default::default() };
        assert!(evaluate(&rules(), &obs).is_empty(), "égal au seuil : rien");
        obs.cost_last_hour = Some(4.2);
        obs.stalled_agents = 1;
        let fired = evaluate(&rules(), &obs);
        assert_eq!(fired.len(), 2);
        assert_eq!(fired[0].line_fr(), "coût sur l'heure : 4,20 $ pour un seuil de 3,00 $");
    }

    #[test]
    fn an_unpriced_hour_never_fires_a_cost_monitor() {
        let obs = Observation { cost_last_hour: None, ..Default::default() };
        assert!(evaluate(&rules(), &obs).is_empty());
    }

    #[test]
    fn a_standing_alert_is_not_raised_twice_and_clears_once() {
        let obs = Observation { cost_last_hour: Some(5.0), ..Default::default() };
        let fired = evaluate(&rules(), &obs);
        let mut standing = BTreeMap::new();
        let (raised, resolved) = diff(&standing, &fired);
        assert_eq!((raised.len(), resolved.len()), (1, 0));
        standing.insert("budget".to_string(), raised[0].clone());
        let (raised, resolved) = diff(&standing, &fired);
        assert!(raised.is_empty() && resolved.is_empty());
        let (raised, resolved) = diff(&standing, &[]);
        assert!(raised.is_empty());
        assert_eq!(resolved, vec!["budget".to_string()]);
    }

    #[test]
    fn the_alerts_are_replayed_from_the_events() {
        let raise = |secs, value| {
            ev(secs, None, EventKind::AlertRaised {
                monitor: "budget".into(),
                monitor_kind: MonitorKind::CostPerHour,
                value,
                threshold: 3.0,
            })
        };
        let events = vec![
            raise(1, 4.0),
            ev(2, None, EventKind::AlertResolved { monitor: "budget".into() }),
            raise(3, 6.0),
        ];
        let (standing, history) = replay_alerts(&events);
        assert_eq!(standing["budget"].value, 6.0);
        assert_eq!(history.len(), 3);
        assert!(!history[1].raised);
        let (standing, _) = replay_alerts(&events[..2]);
        assert!(standing.is_empty());
    }

    #[test]
    fn a_trace_follows_the_ticket_from_proposal_to_merge() {
        let a = Uuid::new_v4();
        let run = |ok| Box::new(CheckRun {
            command: "cargo test".into(),
            ok,
            code: Some(if ok { 0 } else { 101 }),
            duration_ms: 1000,
            tail: String::new(),
        });
        let events = vec![
            ev(0, None, EventKind::TicketCreated { number: 1, title: "t".into() }),
            ev(10, None, EventKind::ProposalFailed { error: "x".into() }),
            ev(20, Some(a), EventKind::AgentSpawned {
                role: "dev".into(), session_id: Uuid::new_v4(), pid: 1, cmdline: String::new(),
            }),
            ev(30, Some(a), EventKind::AgentStatusChanged { status: AgentStatus::Running, reason: None }),
            ev(80, Some(a), EventKind::AgentStatusChanged { status: AgentStatus::Done, reason: None }),
            ev(81, None, EventKind::CheckStarted { round: 1, command: "cargo test".into() }),
            ev(90, None, EventKind::CheckFinished { round: 1, run: run(false) }),
            ev(95, None, EventKind::ReviewVerdict {
                round: 1, verdict: Verdict::Changes, blocking: vec![], roles: vec![],
            }),
            ev(99, None, EventKind::TicketMerged {
                branch: "orch/x".into(), into: "main".into(), commits: 1, pushed_to: None,
            }),
        ];
        let t = build_trace(&events);
        let kinds: Vec<_> = t.spans.iter().map(|s| (s.kind, s.ok)).collect();
        assert_eq!(kinds, vec![
            (SpanKind::Planning, Some(false)),
            (SpanKind::Agent, Some(true)),
            (SpanKind::Check, Some(false)),
            (SpanKind::Review, Some(false)),
            (SpanKind::Merge, Some(true)),
        ]);
        // Running is not an end: the agent's step runs from spawn to done.
        assert_eq!(t.spans[1].duration_ms(at(1000)), 60_000);
        assert_eq!(t.bounds(at(1000)), Some((at(0), at(99))));
    }

    #[test]
    fn a_step_still_running_ends_at_now_and_has_no_verdict() {
        let a = Uuid::new_v4();
        let events = vec![ev(5, Some(a), EventKind::AgentSpawned {
            role: "dev".into(), session_id: Uuid::new_v4(), pid: 1, cmdline: String::new(),
        })];
        let t = build_trace(&events);
        assert_eq!(t.spans[0].ok, None);
        assert_eq!(t.spans[0].duration_ms(at(15)), 10_000);
        assert!(build_trace(&[]).is_empty());
    }
}
