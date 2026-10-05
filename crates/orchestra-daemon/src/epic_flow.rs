//! An accepted epic moving along: when one of its tickets is merged, the
//! tickets it freed are planned (`epic.auto_plan`), and once all are merged
//! the epic is done. Planning is only a proposal: no team starts here.

use std::collections::HashMap;

use orchestra_core::epic::{EpicId, EpicMember, EpicStatus};
use orchestra_core::events::{EventKind, NewEvent};
use orchestra_core::model::{Ticket, TicketId, TicketStatus};
use orchestra_core::protocol::Command;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::bus::EventBus;
use crate::daemon::DaemonHandle;
use crate::store::Store;

/// What a merged ticket changes for its epic.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Next {
    /// Every ticket of the epic is merged.
    pub finished: bool,
    /// Tickets now free that have no team proposal yet: plan them.
    pub to_plan: Vec<TicketId>,
}

/// Decide, from the epic's members and their tickets as they stand.
pub fn next_steps(members: &[EpicMember], tickets: &HashMap<TicketId, Ticket>) -> Next {
    let done = |id: &TicketId| tickets.get(id).is_some_and(|t| t.status == TicketStatus::Done);
    let finished = !members.is_empty() && members.iter().all(|m| done(&m.ticket_id));
    let to_plan = members
        .iter()
        .filter(|m| m.depends_on.iter().all(done))
        .filter_map(|m| tickets.get(&m.ticket_id))
        .filter(|t| t.status == TicketStatus::Draft && t.proposal.is_none())
        .map(|t| t.id)
        .collect();
    Next { finished, to_plan }
}

pub async fn advance_epics(
    store: Store,
    bus: EventBus,
    handle: DaemonHandle,
    auto_plan: bool,
    cancel: CancellationToken,
) {
    let mut rx = bus.subscribe();
    loop {
        let event = tokio::select! {
            _ = cancel.cancelled() => return,
            event = rx.recv() => event,
        };
        let event = match event {
            Ok(e) => e,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        };
        let EventKind::TicketStatusChanged { to: TicketStatus::Done, .. } = event.kind else {
            continue;
        };
        let Some(ticket_id) = event.ticket_id else { continue };
        if let Err(e) = on_merged(&store, &bus, &handle, auto_plan, ticket_id).await {
            tracing::warn!("avancement de l'épopée : {e:#}");
        }
    }
}

async fn on_merged(
    store: &Store,
    bus: &EventBus,
    handle: &DaemonHandle,
    auto_plan: bool,
    ticket_id: TicketId,
) -> anyhow::Result<()> {
    let all = store.all_epic_members().await?;
    let Some(epic_id) = all.iter().find(|(_, m)| m.ticket_id == ticket_id).map(|(e, _)| *e) else {
        return Ok(());
    };
    let members: Vec<EpicMember> = all
        .into_iter()
        .filter(|(e, _)| *e == epic_id)
        .map(|(_, m)| m)
        .collect();
    let mut tickets = HashMap::new();
    for m in &members {
        if let Some(t) = store.ticket(m.ticket_id).await? {
            tickets.insert(t.id, t);
        }
    }
    let next = next_steps(&members, &tickets);
    if next.finished {
        finish(store, bus, epic_id).await?;
        return Ok(());
    }
    if auto_plan {
        for id in next.to_plan {
            if let Err(e) = handle.call(Command::PlanTicket { ticket_id: id }).await {
                bus.warn(format!("ticket libéré mais non planifié : {}", e.message)).await;
            }
        }
    }
    Ok(())
}

async fn finish(store: &Store, bus: &EventBus, epic_id: EpicId) -> anyhow::Result<()> {
    let Some(mut epic) = store.epic(epic_id).await? else {
        return Ok(());
    };
    if epic.status == EpicStatus::Done {
        return Ok(());
    }
    epic.status = EpicStatus::Done;
    epic.updated_at = orchestra_core::now();
    let project_id = epic.project_id;
    store.update_epic(epic).await?;
    bus.publish(NewEvent::new(EventKind::EpicFinished { epic_id }).project(project_id))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn ticket(status: TicketStatus) -> Ticket {
        Ticket {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            number: 1,
            title: "t".into(),
            brief: "b".into(),
            status,
            branch: None,
            worktree_path: None,
            proposal: None,
            team: None,
            created_at: orchestra_core::now(),
            updated_at: orchestra_core::now(),
        }
    }

    #[test]
    fn a_merge_frees_what_depended_on_it_and_only_that() {
        let schema = ticket(TicketStatus::Done);
        let api = ticket(TicketStatus::Draft);
        let screen = ticket(TicketStatus::Draft);
        let member = |t: &Ticket, deps: &[&Ticket], position| EpicMember {
            ticket_id: t.id,
            position,
            depends_on: deps.iter().map(|d| d.id).collect(),
        };
        let members = vec![member(&schema, &[], 0), member(&api, &[&schema], 1), member(&screen, &[&api], 2)];
        let map: HashMap<_, _> = [&schema, &api, &screen].iter().map(|t| (t.id, (*t).clone())).collect();

        let next = next_steps(&members, &map);
        assert!(!next.finished);
        assert_eq!(next.to_plan, vec![api.id], "l'écran attend encore l'api");

        // A ticket already planned is not planned again.
        let mut planned = map.clone();
        planned.get_mut(&api.id).unwrap().status = TicketStatus::Planned;
        assert!(next_steps(&members, &planned).to_plan.is_empty());

        // All merged: the epic is done.
        let all_done: HashMap<_, _> = map
            .into_iter()
            .map(|(id, mut t)| {
                t.status = TicketStatus::Done;
                (id, t)
            })
            .collect();
        assert!(next_steps(&members, &all_done).finished);
    }
}
