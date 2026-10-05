//! What a ticket waits on the user for, as the daemon reads it — the rule
//! itself is `orchestra_core::attention` — and the watch that tells the
//! desktop when a ticket starts waiting.

use std::collections::{HashMap, HashSet};

use orchestra_core::attention::{Attention, Facts};
use orchestra_core::events::{Event, EventFilter, EventKind, EventTag};
use orchestra_core::model::{Agent, AgentId, Ticket, TicketId, TicketStatus};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::bus::EventBus;
use crate::store::Store;
use crate::supervisor::Supervisor;

/// The one thing `ticket` waits on the user for. `team` is its agents;
/// `pr_open` and `merge_blocked` are passed in because the board already
/// reads them in bulk.
pub async fn of_ticket(
    store: &Store,
    ticket: &Ticket,
    team: &[Agent],
    stalled: &HashSet<AgentId>,
    pr_open: bool,
    merge_blocked: bool,
) -> Option<Attention> {
    // Verdict and checks only matter once the team handed back; those
    // tickets are few, so one read each stays cheap.
    let (verdict, checks_failed) = if ticket.status == TicketStatus::Review {
        (last_verdict(store, ticket.id).await, last_checks_failed(store, ticket.id).await)
    } else {
        (None, false)
    };
    orchestra_core::attention::of(&Facts {
        status: Some(ticket.status),
        has_proposal: ticket.proposal.is_some(),
        has_team: ticket.team.is_some(),
        agent_stalled: team.iter().any(|a| stalled.contains(&a.id)),
        verdict,
        checks_failed,
        merge_blocked,
        pull_request_open: pr_open,
    })
}

async fn last_verdict(store: &Store, ticket_id: TicketId) -> Option<orchestra_core::review::Verdict> {
    let filter = EventFilter {
        ticket_id: Some(ticket_id),
        tags: vec![EventTag::ReviewVerdict],
        ..Default::default()
    };
    let events = store.recent_events(filter, 1).await.ok()?;
    events.into_iter().rev().find_map(|e| match e.kind {
        EventKind::ReviewVerdict { verdict, .. } => Some(verdict),
        _ => None,
    })
}

async fn last_checks_failed(store: &Store, ticket_id: TicketId) -> bool {
    let filter = EventFilter {
        ticket_id: Some(ticket_id),
        tags: vec![EventTag::CheckFinished],
        ..Default::default()
    };
    // A pass is a handful of commands, and a ticket runs few passes.
    let events = store.recent_events(filter, 64).await.unwrap_or_default();
    crate::checks::last_pass(&events).is_some_and(|c| c.failed().is_some())
}

/// Events after which a ticket may have started — or stopped — waiting.
fn may_change_attention(e: &Event) -> bool {
    matches!(
        e.kind.tag(),
        EventTag::ProposalReady
            | EventTag::TicketStatusChanged
            | EventTag::ReviewVerdict
            | EventTag::CheckFinished
            | EventTag::MergeBlocked
            | EventTag::AgentStalled
            | EventTag::PullRequestOpened
    )
}

/// What a notification says: the ticket, and what it waits for.
pub fn notification(ticket: &Ticket, attention: Attention) -> (String, String) {
    (
        format!("Orchestra — #{} {}", ticket.number, ticket.title),
        attention.label_fr().to_string(),
    )
}

/// Which tickets wait for what, as last told: a notification goes out only
/// when that changes to something.
#[derive(Default)]
pub struct Watch {
    last: HashMap<TicketId, Option<Attention>>,
}

impl Watch {
    /// The notification `event` calls for, if any.
    pub async fn on_event(
        &mut self,
        store: &Store,
        supervisor: &Supervisor,
        event: &Event,
    ) -> Option<(String, String)> {
        let ticket_id = event.ticket_id.filter(|_| may_change_attention(event))?;
        let ticket = store.ticket(ticket_id).await.ok()??;
        let team = store.agents_of_ticket(ticket_id).await.unwrap_or_default();
        let stalled = supervisor.stalled_agents().await;
        let pr_open = crate::github::open_pull_request(store, ticket_id).await.is_some();
        let blocked = ticket.status == TicketStatus::Review
            && crate::integration::blocked_merge(store, ticket_id).await.is_some();
        let now = of_ticket(store, &ticket, &team, &stalled, pr_open, blocked).await;
        let before = self.last.insert(ticket_id, now).flatten();
        now.filter(|a| Some(*a) != before)
            .map(|attention| notification(&ticket, attention))
    }
}

/// Tell the desktop each time a ticket starts waiting on the user, or waits
/// for something new. Never anything else: a notification for every event
/// would be the first thing switched off.
pub async fn notify_on_attention(
    store: Store,
    bus: EventBus,
    supervisor: Supervisor,
    cancel: CancellationToken,
) {
    let mut rx = bus.subscribe();
    let mut watch = Watch::default();
    loop {
        let event = tokio::select! {
            _ = cancel.cancelled() => return,
            event = rx.recv() => event,
        };
        let event = match event {
            Ok(e) => e,
            // Missed events only delay a notification to the next one.
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        };
        if let Some((title, body)) = watch.on_event(&store, &supervisor, &event).await {
            crate::notify::send(&title, &body).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::config::Config;
    use orchestra_core::events::NewEvent;
    use orchestra_core::model::{Project, ProjectKind};
    use std::sync::Arc;
    use uuid::Uuid;

    #[tokio::test]
    async fn a_ticket_is_announced_when_it_starts_waiting_and_not_again() {
        let store = Store::open_memory().unwrap();
        let bus = EventBus::new(store.clone());
        let cfg = Arc::new(Config::default());
        let ledger = crate::ledger::UsageLedger::new(store.clone(), &cfg);
        let dir = tempfile::tempdir().unwrap();
        let sup = Supervisor::new(store.clone(), bus.clone(), ledger, cfg, dir.path().into(), dir.path().join("c"));
        let project = Project {
            id: Uuid::new_v4(),
            name: "p".into(),
            path: dir.path().into(),
            default_branch: "main".into(),
            zellij_tab: None,
            kind: ProjectKind::Managed,
            created_at: orchestra_core::now(),
        };
        store.insert_project(project.clone()).await.unwrap();
        let mut ticket = Ticket {
            id: Uuid::new_v4(),
            project_id: project.id,
            number: 4,
            title: "cache".into(),
            brief: "b".into(),
            status: TicketStatus::Review,
            branch: None,
            worktree_path: None,
            proposal: None,
            team: None,
            created_at: orchestra_core::now(),
            updated_at: orchestra_core::now(),
        };
        store.insert_ticket(ticket.clone()).await.unwrap();
        let verdict = |v| {
            NewEvent::new(EventKind::ReviewVerdict { round: 1, verdict: v, blocking: vec![], roles: vec![] })
                .project(project.id)
                .ticket(ticket.id)
        };

        let mut watch = Watch::default();
        let ev = bus.publish(verdict(orchestra_core::review::Verdict::Ready)).await.unwrap();
        let said = watch.on_event(&store, &sup, &ev).await;
        assert_eq!(
            said,
            Some(("Orchestra — #4 cache".to_string(), "prêt à intégrer".to_string()))
        );
        // The same state again: nothing new to say.
        let ev = bus.publish(verdict(orchestra_core::review::Verdict::Ready)).await.unwrap();
        assert_eq!(watch.on_event(&store, &sup, &ev).await, None);

        // An event that cannot change anything is not even looked at.
        let ev = bus
            .publish(NewEvent::new(EventKind::Warning { message: "x".into() }).ticket(ticket.id))
            .await
            .unwrap();
        assert_eq!(watch.on_event(&store, &sup, &ev).await, None);

        // Done: no longer waiting, so nothing to announce.
        ticket.status = TicketStatus::Done;
        store.update_ticket(ticket.clone()).await.unwrap();
        let ev = bus
            .publish(
                NewEvent::new(EventKind::TicketStatusChanged { from: TicketStatus::Review, to: TicketStatus::Done })
                    .ticket(ticket.id),
            )
            .await
            .unwrap();
        assert_eq!(watch.on_event(&store, &sup, &ev).await, None);
    }
}
