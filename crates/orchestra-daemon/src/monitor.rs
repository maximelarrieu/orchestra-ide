//! The monitors of `config.toml`: the daemon measures, compares to the
//! thresholds, and writes an event when a crossing happens — one when a
//! measure goes over, one when it comes back (`orchestra_core::observe`).

use std::time::Duration;

use anyhow::Result;
use orchestra_core::events::{EventFilter, EventKind, EventTag, NewEvent};
use orchestra_core::observe::{
    diff, evaluate, replay_alerts, Firing, MonitorRule, Observation, SeriesQuery,
};
use tokio_util::sync::CancellationToken;

use crate::bus::EventBus;
use crate::ledger::UsageLedger;
use crate::store::Store;
use crate::supervisor::Supervisor;

const TICK: Duration = Duration::from_secs(30);

/// Events a trace or an alert history is read back from; far above what one
/// ticket or one machine writes.
pub const ALERT_HISTORY: u32 = 5_000;

pub fn alert_filter() -> EventFilter {
    EventFilter {
        tags: vec![EventTag::AlertRaised, EventTag::AlertResolved],
        ..Default::default()
    }
}

/// The measures of the last hour. Taken from the same ledger as the cost
/// screen, so an alert and the screen never disagree.
pub async fn observe(
    store: &Store,
    ledger: &UsageLedger,
    supervisor: &Supervisor,
) -> Result<Observation> {
    let series = ledger
        .series(&SeriesQuery { hours: 1, step_minutes: 1, project_id: None })
        .await?;
    let (tokens, cost) = series.tail_sum(series.tokens.len());
    let since = orchestra_core::now() - time::Duration::hours(1);
    Ok(Observation {
        tokens_last_hour: tokens,
        cost_last_hour: Some(cost),
        stalled_agents: supervisor.stalled_agents().await.len() as u32,
        failures_last_hour: store.agent_failures_since(since).await?,
    })
}

/// Off when no monitor is defined: nothing to measure, nothing to wake for.
pub async fn watch(
    monitors: Vec<MonitorRule>,
    store: Store,
    bus: EventBus,
    ledger: UsageLedger,
    supervisor: Supervisor,
    cancel: CancellationToken,
) {
    if monitors.is_empty() {
        return;
    }
    // A restart neither raises again what already stands nor forgets it.
    let past = store
        .events_since(0, alert_filter(), ALERT_HISTORY)
        .await
        .unwrap_or_default();
    let (mut standing, _) = replay_alerts(&past);
    let mut ticker = tokio::time::interval(TICK);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = ticker.tick() => {}
        }
        let obs = match observe(&store, &ledger, &supervisor).await {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!("mesure des alertes impossible : {e:#}");
                continue;
            }
        };
        let (raised, resolved) = diff(&standing, &evaluate(&monitors, &obs));
        for f in raised {
            if publish_raise(&bus, &f).await {
                standing.insert(f.monitor.clone(), f);
            }
        }
        for name in resolved {
            let event = NewEvent::new(EventKind::AlertResolved { monitor: name.clone() });
            if bus.publish(event).await.is_ok() {
                standing.remove(&name);
            }
        }
    }
}

async fn publish_raise(bus: &EventBus, f: &Firing) -> bool {
    let event = NewEvent::new(EventKind::AlertRaised {
        monitor: f.monitor.clone(),
        monitor_kind: f.kind,
        value: f.value,
        threshold: f.threshold,
    });
    bus.publish(event).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::model::{AgentStatus, ExitReason};

    fn status(status: AgentStatus) -> NewEvent {
        NewEvent::new(EventKind::AgentStatusChanged { status, reason: None::<ExitReason> })
    }

    #[tokio::test]
    async fn only_failed_and_crashed_agents_count_as_failures() {
        let store = Store::open_memory().unwrap();
        for s in [
            AgentStatus::Running,
            AgentStatus::Done,
            AgentStatus::Cancelled,
            AgentStatus::Failed,
            AgentStatus::Crashed,
        ] {
            store.append_event(status(s)).await.unwrap();
        }
        let hour_ago = orchestra_core::now() - time::Duration::hours(1);
        assert_eq!(store.agent_failures_since(hour_ago).await.unwrap(), 2);
        let tomorrow = orchestra_core::now() + time::Duration::days(1);
        assert_eq!(store.agent_failures_since(tomorrow).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn alerts_read_back_from_the_store_as_they_were_written() {
        let store = Store::open_memory().unwrap();
        let bus = EventBus::new(store.clone());
        let firing = Firing {
            monitor: "budget".into(),
            kind: orchestra_core::observe::MonitorKind::CostPerHour,
            value: 4.0,
            threshold: 3.0,
        };
        assert!(publish_raise(&bus, &firing).await);
        let events = store.events_since(0, alert_filter(), ALERT_HISTORY).await.unwrap();
        let (standing, history) = replay_alerts(&events);
        assert_eq!(standing["budget"], firing);
        assert_eq!(history.len(), 1);
    }
}
