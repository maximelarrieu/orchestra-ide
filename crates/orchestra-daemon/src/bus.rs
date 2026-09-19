//! Event bus: persist first, then broadcast.
//!
//! Persisting first is what lets a client resume without a gap: every event a
//! subscriber sees already has a `seq` it can come back to.

use std::sync::Arc;

use anyhow::Result;
use orchestra_core::events::{Event, EventKind, NewEvent};
use tokio::sync::broadcast;

use crate::store::Store;

/// How many events a slow subscriber may fall behind before it is told to
/// resync from the store.
const CHANNEL_CAPACITY: usize = 2048;

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Arc<Event>>,
    store: Store,
}

impl std::fmt::Debug for EventBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventBus")
            .field("subscribers", &self.tx.receiver_count())
            .finish()
    }
}

impl EventBus {
    pub fn new(store: Store) -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        EventBus { tx, store }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Persist then broadcast. The event is returned with its `seq`.
    pub async fn publish(&self, e: NewEvent) -> Result<Arc<Event>> {
        let stored = Arc::new(self.store.append_event(e).await?);
        // An error here only means nobody is listening.
        let _ = self.tx.send(Arc::clone(&stored));
        Ok(stored)
    }

    /// Publish without a scope. For daemon-level notices.
    pub async fn publish_kind(&self, kind: EventKind) -> Result<Arc<Event>> {
        self.publish(NewEvent::new(kind)).await
    }

    /// Log a warning both to tracing and to the event log, so it shows up in
    /// the TUI rather than only in a file the user never opens.
    pub async fn warn(&self, message: impl Into<String>) {
        let message = message.into();
        tracing::warn!("{message}");
        if let Err(e) = self.publish_kind(EventKind::Warning { message }).await {
            tracing::error!("impossible d'enregistrer l'avertissement : {e}");
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Event>> {
        self.tx.subscribe()
    }

    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::events::EventFilter;

    async fn bus() -> EventBus {
        EventBus::new(Store::open_memory().unwrap())
    }

    #[tokio::test]
    async fn published_events_are_numbered_and_stored() {
        let bus = bus().await;
        let a = bus
            .publish_kind(EventKind::Warning {
                message: "a".into(),
            })
            .await
            .unwrap();
        let b = bus
            .publish_kind(EventKind::Warning {
                message: "b".into(),
            })
            .await
            .unwrap();
        assert_eq!(a.seq, 1);
        assert_eq!(b.seq, 2);

        let stored = bus
            .store()
            .events_since(0, EventFilter::all(), 10)
            .await
            .unwrap();
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].seq, 1);
    }

    #[tokio::test]
    async fn subscribers_see_events_published_after_they_subscribe() {
        let bus = bus().await;
        bus.publish_kind(EventKind::Warning {
            message: "avant".into(),
        })
        .await
        .unwrap();

        let mut rx = bus.subscribe();
        bus.publish_kind(EventKind::Warning {
            message: "après".into(),
        })
        .await
        .unwrap();

        let got = rx.recv().await.unwrap();
        assert_eq!(got.seq, 2);
        assert!(matches!(&got.kind, EventKind::Warning { message } if message == "après"));
        // The earlier event is only in the store, which is what `since_seq` is for.
        assert_eq!(bus.store().last_seq().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn publishing_without_subscribers_is_not_an_error() {
        let bus = bus().await;
        assert_eq!(bus.subscriber_count(), 0);
        bus.publish_kind(EventKind::DaemonStarted {
            version: "0.1.0".into(),
        })
        .await
        .unwrap();
    }
}
