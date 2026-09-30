//! A ready branch the daemon could not merge, read back from the events.
//!
//! Kept out of a column for the same reason as the open pull request: the
//! event stream already says it. A refused merge is a `MergeBlocked`, and
//! anything that moves the ticket on — a new integrator, a status change, a
//! merge, a request opened — is a later event that takes the wait back.

use orchestra_core::events::{Event, EventFilter, EventKind, EventTag};
use orchestra_core::model::TicketId;

use crate::store::Store;

/// What a refused merge left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedMerge {
    pub branch: String,
    pub head: String,
    pub reason: String,
}

/// The events that decide whether a ticket is waiting on its merge.
const TAGS: [EventTag; 5] = [
    EventTag::MergeBlocked,
    EventTag::AgentSpawned,
    EventTag::TicketStatusChanged,
    EventTag::TicketMerged,
    EventTag::PullRequestOpened,
];

/// The merge a ticket is waiting on, if the last word was a refusal.
pub async fn blocked_merge(store: &Store, ticket_id: TicketId) -> Option<BlockedMerge> {
    let filter = EventFilter {
        ticket_id: Some(ticket_id),
        tags: TAGS.to_vec(),
        ..Default::default()
    };
    let events = store.recent_events(filter, 1).await.unwrap_or_default();
    last_refusal(&events)
}

/// The refusal still standing in a ticket's events, oldest first.
fn last_refusal(events: &[Event]) -> Option<BlockedMerge> {
    match &events.last()?.kind {
        EventKind::MergeBlocked {
            branch,
            head,
            reason,
        } => Some(BlockedMerge {
            branch: branch.clone(),
            head: head.clone(),
            reason: reason.clone(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::events::NewEvent;
    use orchestra_core::model::TicketStatus;

    fn event(seq: i64, kind: EventKind) -> Event {
        Event::from_new(seq, NewEvent::new(kind))
    }

    fn refused() -> EventKind {
        EventKind::MergeBlocked {
            branch: "orch/2-cockpit".into(),
            head: "abc".into(),
            reason: "le dépôt principal a 1 fichier(s) modifié(s)".into(),
        }
    }

    #[test]
    fn a_refused_merge_is_waiting_until_something_moves_the_ticket() {
        let waiting = last_refusal(&[event(1, refused())]).expect("en attente");
        assert_eq!(waiting.head, "abc");

        for later in [
            EventKind::TicketStatusChanged {
                from: TicketStatus::Review,
                to: TicketStatus::Done,
            },
            EventKind::TicketMerged {
                branch: "orch/2-cockpit".into(),
                into: "main".into(),
                commits: 2,
                pushed_to: None,
            },
        ] {
            assert_eq!(last_refusal(&[event(1, refused()), event(2, later)]), None);
        }
        assert_eq!(last_refusal(&[]), None);
    }
}
