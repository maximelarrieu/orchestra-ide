//! Epics, as the daemon serves them: written down, split by the
//! orchestrator, read and accepted by the user, then worked through ticket by
//! ticket in the order the split gave.

use orchestra_core::epic::{Epic, EpicId, EpicProposal, EpicStatus};
use orchestra_core::protocol::{EpicDetail, EpicTicketRow};

use super::*;

impl Daemon {
    pub(super) async fn create_epic(
        &self,
        project_id: ProjectId,
        title: String,
        brief: String,
    ) -> Result<Reply, ApiError> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(ApiError::invalid("le titre de l'épopée est vide"));
        }
        if brief.trim().is_empty() {
            return Err(ApiError::invalid(
                "une épopée sans demande ne se découpe pas : écris ce qu'elle doit livrer",
            ));
        }
        if self.store.project(project_id).await.map_err(internal)?.is_none() {
            return Err(ApiError::not_found("projet"));
        }
        let now = orchestra_core::now();
        let epic = Epic {
            id: Uuid::new_v4(),
            project_id,
            title: title.clone(),
            brief,
            status: EpicStatus::Draft,
            proposal: None,
            created_at: now,
            updated_at: now,
        };
        self.store.insert_epic(epic.clone()).await.map_err(internal)?;
        self.bus
            .publish(NewEvent::new(EventKind::EpicCreated { epic_id: epic.id, title }).project(project_id))
            .await
            .map_err(internal)?;
        Ok(Reply::Epic {
            detail: Box::new(EpicDetail { epic, tickets: vec![] }),
        })
    }

    pub(super) async fn plan_epic(&mut self, epic_id: EpicId) -> Result<Reply, ApiError> {
        let epic = self.epic_or_404(epic_id).await?;
        if !matches!(epic.status, EpicStatus::Draft | EpicStatus::Split) {
            return Err(ApiError::invalid(format!(
                "une épopée « {} » ne se redécoupe pas",
                epic.status.label_fr()
            )));
        }
        // Same guard as a ticket's planning, same set: ids never collide.
        if self.planning.contains(&epic_id) {
            return Err(ApiError::conflict("le découpage de cette épopée est déjà en cours"));
        }
        let project = self
            .store
            .project(epic.project_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("projet de l'épopée"))?;

        self.planning.insert(epic_id);
        let (bus, store, cfg) = (self.bus.clone(), self.store.clone(), Arc::clone(&self.cfg));
        let (cache_dir, ledger, done) = (self.paths.cache_dir.clone(), self.ledger.clone(), self.planning_done.clone());
        tokio::spawn(async move {
            match crate::orchestrator::split(&epic, &project, &cfg, &cache_dir, &bus, &ledger).await {
                Ok(proposal) => {
                    if let Err(e) = proposal.waves() {
                        bus.warn(format!("découpage à corriger : {e}")).await;
                    }
                    let tickets = proposal.tickets.len() as u32;
                    let mut updated = epic.clone();
                    updated.proposal = Some(proposal);
                    updated.status = EpicStatus::Split;
                    updated.updated_at = orchestra_core::now();
                    if let Err(e) = store.update_epic(updated).await {
                        tracing::error!("découpage non enregistré : {e:#}");
                    }
                    let _ = bus
                        .publish(NewEvent::new(EventKind::EpicSplitReady { epic_id, tickets }).project(project.id))
                        .await;
                }
                Err(e) => {
                    let _ = bus
                        .publish(
                            NewEvent::new(EventKind::EpicSplitFailed { epic_id, error: format!("{e:#}") })
                                .project(project.id),
                        )
                        .await;
                }
            }
            let _ = done.send(epic_id).await;
        });
        Ok(Reply::Ack)
    }

    pub(super) async fn list_epics(&self, project_id: Option<ProjectId>) -> Result<Reply, ApiError> {
        let epics = self.store.epics(project_id).await.map_err(internal)?;
        Ok(Reply::Epics { epics })
    }

    pub(super) async fn get_epic(&self, epic_id: EpicId) -> Result<Reply, ApiError> {
        let epic = self.epic_or_404(epic_id).await?;
        let tickets = self.epic_rows(epic_id).await?;
        Ok(Reply::Epic {
            detail: Box::new(EpicDetail { epic, tickets }),
        })
    }

    /// The tickets an accepted epic became, in order.
    async fn epic_rows(&self, epic_id: EpicId) -> Result<Vec<EpicTicketRow>, ApiError> {
        let members = self.store.epic_members(epic_id).await.map_err(internal)?;
        let mut numbers: HashMap<TicketId, i64> = HashMap::new();
        let mut rows = Vec::with_capacity(members.len());
        let mut found = Vec::with_capacity(members.len());
        for m in &members {
            if let Some(t) = self.store.ticket(m.ticket_id).await.map_err(internal)? {
                numbers.insert(t.id, t.number);
                found.push((m, t));
            }
        }
        for (m, t) in found {
            rows.push(EpicTicketRow {
                ticket_id: t.id,
                number: t.number,
                title: t.title,
                status: t.status,
                depends_on: m.depends_on.iter().filter_map(|d| numbers.get(d).copied()).collect(),
            });
        }
        Ok(rows)
    }

    /// Turn the split, as the user left it, into tickets. Those that depend
    /// on nothing are planned at once (`epic.auto_plan`); the others wait
    /// for their turn.
    pub(super) async fn accept_epic(
        &mut self,
        epic_id: EpicId,
        proposal: EpicProposal,
    ) -> Result<Reply, ApiError> {
        let mut epic = self.epic_or_404(epic_id).await?;
        if epic.status != EpicStatus::Split {
            return Err(ApiError::invalid(format!(
                "une épopée « {} » ne s'accepte pas : il faut d'abord un découpage",
                epic.status.label_fr()
            )));
        }
        proposal
            .waves()
            .map_err(|e| ApiError::invalid(format!("découpage refusé : {e}")))?;

        let now = orchestra_core::now();
        let tickets: Vec<Ticket> = proposal
            .tickets
            .iter()
            .map(|t| Ticket {
                id: Uuid::new_v4(),
                project_id: epic.project_id,
                // Numbered inside the transaction that inserts them.
                number: 0,
                title: t.title.trim().to_string(),
                brief: t.ticket_brief(),
                status: TicketStatus::Draft,
                branch: None,
                worktree_path: None,
                proposal: None,
                team: None,
                created_at: now,
                updated_at: now,
            })
            .collect();
        let deps: Vec<Vec<usize>> = proposal.tickets.iter().map(|t| t.depends_on.clone()).collect();
        epic.proposal = Some(proposal);
        epic.status = EpicStatus::Active;
        epic.updated_at = now;
        let tickets = self
            .store
            .accept_epic(epic.clone(), tickets, deps.clone())
            .await
            .map_err(internal)?;

        for t in &tickets {
            let _ = self
                .bus
                .publish(
                    NewEvent::new(EventKind::TicketCreated { number: t.number, title: t.title.clone() })
                        .project(epic.project_id)
                        .ticket(t.id),
                )
                .await;
        }
        let _ = self
            .bus
            .publish(
                NewEvent::new(EventKind::EpicAccepted {
                    epic_id,
                    tickets: tickets.iter().map(|t| t.id).collect(),
                })
                .project(epic.project_id),
            )
            .await;

        if self.cfg.epic.auto_plan {
            for (t, d) in tickets.iter().zip(&deps) {
                if d.is_empty() {
                    if let Err(e) = self.plan_ticket(t.id).await {
                        self.bus
                            .warn(format!("#{} n'a pas pu être planifié : {}", t.number, e.message))
                            .await;
                    }
                }
            }
        }
        self.get_epic(epic_id).await
    }

    async fn epic_or_404(&self, epic_id: EpicId) -> Result<Epic, ApiError> {
        self.store
            .epic(epic_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("épopée"))
    }

    /// Why `ticket` cannot start yet: the tickets of its epic it waits for,
    /// not merged. Empty when it is free, or in no epic.
    pub(super) async fn unmet_dependencies(&self, ticket_id: TicketId) -> Result<Vec<Ticket>, ApiError> {
        let members = self.store.all_epic_members().await.map_err(internal)?;
        let Some((_, me)) = members.into_iter().find(|(_, m)| m.ticket_id == ticket_id) else {
            return Ok(Vec::new());
        };
        let mut unmet = Vec::new();
        for dep in me.depends_on {
            if let Some(t) = self.store.ticket(dep).await.map_err(internal)? {
                if t.status != TicketStatus::Done {
                    unmet.push(t);
                }
            }
        }
        Ok(unmet)
    }

    /// For each ticket of `tickets` that belongs to an epic: the epic, and
    /// the numbers of the tickets it still waits for.
    pub(super) async fn epic_links(
        &self,
        tickets: &[Ticket],
    ) -> Result<HashMap<TicketId, orchestra_core::epic::EpicLink>, ApiError> {
        let members = self.store.all_epic_members().await.map_err(internal)?;
        if members.is_empty() {
            return Ok(HashMap::new());
        }
        let titles: HashMap<EpicId, String> = self
            .store
            .epics(None)
            .await
            .map_err(internal)?
            .into_iter()
            .map(|e| (e.id, e.title))
            .collect();
        let mut known: HashMap<TicketId, (i64, TicketStatus)> =
            tickets.iter().map(|t| (t.id, (t.number, t.status))).collect();
        let mut out = HashMap::new();
        for (epic_id, m) in members {
            if !tickets.iter().any(|t| t.id == m.ticket_id) {
                continue;
            }
            let mut waiting_on = Vec::new();
            for dep in &m.depends_on {
                if !known.contains_key(dep) {
                    if let Some(t) = self.store.ticket(*dep).await.map_err(internal)? {
                        known.insert(t.id, (t.number, t.status));
                    }
                }
                if let Some((number, status)) = known.get(dep) {
                    if *status != TicketStatus::Done {
                        waiting_on.push(*number);
                    }
                }
            }
            out.insert(
                m.ticket_id,
                orchestra_core::epic::EpicLink {
                    epic_id,
                    epic_title: titles.get(&epic_id).cloned().unwrap_or_default(),
                    waiting_on,
                },
            );
        }
        Ok(out)
    }
}
