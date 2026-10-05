//! SQLite store. One connection, guarded by a mutex, reached from async code
//! through `spawn_blocking`.
//!
//! Only the daemon writes here, so a single connection is enough and keeps the
//! rules in one place. Every public method is async and never blocks the
//! runtime.

pub mod migrate;
mod rows;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use orchestra_core::events::{Event, EventFilter, EventKind, NewEvent};
use orchestra_core::model::{
    Agent, AgentId, AgentStatus, Project, ProjectId, Ticket, TicketId, TicketStatus,
    Todo, TodoId, Tokens, UsageSample,
};
use orchestra_core::protocol::{GroupBy, UsageQuery};
use rusqlite::{Connection, OpenFlags};
use uuid::Uuid;

pub use rows::{KeyedUsage, Recorded, SessionRow, UsageBreakdown};

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
    path: PathBuf,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").field("path", &self.path).finish()
    }
}

impl Store {
    /// Open (creating it if needed), apply migrations, return the store.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("création de {}", parent.display()))?;
        }
        let mut conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .with_context(|| format!("ouverture de {}", path.display()))?;
        Self::prepare(&mut conn)?;
        // The database may hold prompts and tool summaries: keep it private.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(Store {
            conn: Arc::new(Mutex::new(conn)),
            path: path.to_path_buf(),
        })
    }

    /// In-memory store, for tests.
    pub fn open_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        Self::prepare(&mut conn)?;
        Ok(Store {
            conn: Arc::new(Mutex::new(conn)),
            path: PathBuf::from(":memory:"),
        })
    }

    fn prepare(conn: &mut Connection) -> Result<()> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrate::migrate(conn)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run a closure against the connection on the blocking pool.
    pub async fn with<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let mut guard = conn
                .lock()
                .expect("le mutex du store n'est jamais empoisonné");
            f(&mut guard)
        })
        .await
        .context("tâche bloquante du store annulée")?
    }

    // -- events -------------------------------------------------------------

    /// Persist an event and return it with its assigned `seq`.
    pub async fn append_event(&self, e: NewEvent) -> Result<Event> {
        self.with(move |c| rows::insert_event(c, e)).await
    }

    /// Events after `since_seq`, oldest first, at most `limit`.
    pub async fn events_since(
        &self,
        since_seq: i64,
        filter: EventFilter,
        limit: u32,
    ) -> Result<Vec<Event>> {
        self.with(move |c| rows::select_events_since(c, since_seq, &filter, limit))
            .await
    }

    /// The `limit` most recent matching events, oldest first.
    pub async fn recent_events(&self, filter: EventFilter, limit: u32) -> Result<Vec<Event>> {
        self.with(move |c| rows::select_recent_events(c, &filter, limit))
            .await
    }

    pub async fn last_seq(&self) -> Result<i64> {
        self.with(|c| {
            let v: i64 =
                c.query_row("SELECT COALESCE(MAX(seq), 0) FROM events", [], |r| r.get(0))?;
            Ok(v)
        })
        .await
    }

    // -- projects -----------------------------------------------------------

    pub async fn insert_project(&self, p: Project) -> Result<()> {
        self.with(move |c| rows::insert_project(c, &p)).await
    }

    pub async fn list_projects(&self) -> Result<Vec<Project>> {
        self.with(rows::select_projects).await
    }

    pub async fn project(&self, id: ProjectId) -> Result<Option<Project>> {
        self.with(move |c| rows::select_project(c, id)).await
    }

    /// Forget a project, keeping its usage attached to nothing.
    pub async fn delete_project(&self, id: ProjectId) -> Result<()> {
        self.with(move |c| rows::delete_project(c, id)).await
    }

    pub async fn project_by_path(&self, path: PathBuf) -> Result<Option<Project>> {
        self.with(move |c| rows::select_project_by_path(c, &path))
            .await
    }

    /// The managed or discovered project whose path is the longest prefix of
    /// `cwd`. Used to attribute an unmanaged session to a repository.
    pub async fn project_containing(&self, cwd: PathBuf) -> Result<Option<Project>> {
        self.with(move |c| rows::select_project_containing(c, &cwd))
            .await
    }

    // -- tickets ------------------------------------------------------------

    pub async fn insert_ticket(&self, t: Ticket) -> Result<()> {
        self.with(move |c| rows::insert_ticket(c, &t)).await
    }

    pub async fn update_ticket(&self, t: Ticket) -> Result<()> {
        self.with(move |c| rows::update_ticket(c, &t)).await
    }

    pub async fn ticket(&self, id: TicketId) -> Result<Option<Ticket>> {
        self.with(move |c| rows::select_ticket(c, id)).await
    }

    /// The ticket whose worktree contains this directory.
    pub async fn ticket_by_worktree(&self, cwd: PathBuf) -> Result<Option<Ticket>> {
        self.with(move |c| rows::select_ticket_by_worktree(c, &cwd))
            .await
    }

    pub async fn list_tickets(
        &self,
        project_id: Option<ProjectId>,
        status: Option<Vec<TicketStatus>>,
    ) -> Result<Vec<Ticket>> {
        self.with(move |c| rows::select_tickets(c, project_id, status.as_deref()))
            .await
    }

    /// Next `#n` for a project.
    pub async fn next_ticket_number(&self, project_id: ProjectId) -> Result<i64> {
        self.with(move |c| {
            let n: i64 = c.query_row(
                "SELECT COALESCE(MAX(number), 0) + 1 FROM tickets WHERE project_id = ?1",
                [project_id.to_string()],
                |r| r.get(0),
            )?;
            Ok(n)
        })
        .await
    }

    // -- todos ----------------------------------------------------------------

    pub async fn insert_todo(&self, t: Todo) -> Result<()> {
        self.with(move |c| rows::insert_todo(c, &t)).await
    }

    pub async fn update_todo(&self, t: Todo) -> Result<()> {
        self.with(move |c| rows::update_todo(c, &t)).await
    }

    pub async fn delete_todo(&self, id: TodoId) -> Result<()> {
        self.with(move |c| rows::delete_todo(c, id)).await
    }

    pub async fn todo(&self, id: TodoId) -> Result<Option<Todo>> {
        self.with(move |c| rows::select_todo(c, id)).await
    }

    pub async fn list_todos(&self) -> Result<Vec<Todo>> {
        self.with(rows::select_todos).await
    }

    // -- agents -------------------------------------------------------------

    pub async fn insert_agent(&self, a: Agent) -> Result<()> {
        self.with(move |a_conn| rows::insert_agent(a_conn, &a))
            .await
    }

    pub async fn update_agent(&self, a: Agent) -> Result<()> {
        self.with(move |c| rows::update_agent(c, &a)).await
    }

    pub async fn agent(&self, id: AgentId) -> Result<Option<Agent>> {
        self.with(move |c| rows::select_agent(c, id)).await
    }

    pub async fn agent_by_session(&self, session_id: Uuid) -> Result<Option<Agent>> {
        self.with(move |c| rows::select_agent_by_session(c, session_id))
            .await
    }

    pub async fn agents_of_ticket(&self, ticket_id: TicketId) -> Result<Vec<Agent>> {
        self.with(move |c| rows::select_agents_of_ticket(c, ticket_id))
            .await
    }

    pub async fn agents_of_tickets(&self, project_id: Option<ProjectId>) -> Result<Vec<Agent>> {
        self.with(move |c| rows::select_agents_of_tickets(c, project_id))
            .await
    }

    pub async fn usage_per_ticket(&self, project_id: Option<ProjectId>) -> Result<Vec<KeyedUsage>> {
        self.with(move |c| rows::usage_per_ticket(c, project_id)).await
    }

    pub async fn usage_per_agent(&self, ticket_id: TicketId) -> Result<Vec<KeyedUsage>> {
        self.with(move |c| rows::usage_per_agent(c, ticket_id)).await
    }

    // -- epics --------------------------------------------------------------

    pub async fn insert_epic(&self, e: orchestra_core::epic::Epic) -> Result<()> {
        self.with(move |c| rows::insert_epic(c, &e)).await
    }

    pub async fn update_epic(&self, e: orchestra_core::epic::Epic) -> Result<()> {
        self.with(move |c| rows::update_epic(c, &e)).await
    }

    pub async fn epic(&self, id: orchestra_core::epic::EpicId) -> Result<Option<orchestra_core::epic::Epic>> {
        self.with(move |c| rows::select_epic(c, id)).await
    }

    pub async fn epics(&self, project_id: Option<ProjectId>) -> Result<Vec<orchestra_core::epic::Epic>> {
        self.with(move |c| rows::select_epics(c, project_id)).await
    }

    /// Create an accepted epic's tickets, numbered, linked and saved in one
    /// transaction; returns them with their numbers.
    pub async fn accept_epic(
        &self,
        epic: orchestra_core::epic::Epic,
        mut tickets: Vec<Ticket>,
        deps: Vec<Vec<usize>>,
    ) -> Result<Vec<Ticket>> {
        self.with(move |c| {
            rows::accept_epic(c, &epic, &mut tickets, &deps)?;
            Ok(tickets)
        })
        .await
    }

    pub async fn epic_members(
        &self,
        epic_id: orchestra_core::epic::EpicId,
    ) -> Result<Vec<orchestra_core::epic::EpicMember>> {
        self.with(move |c| rows::epic_members(c, epic_id)).await
    }

    pub async fn all_epic_members(
        &self,
    ) -> Result<Vec<(orchestra_core::epic::EpicId, orchestra_core::epic::EpicMember)>> {
        self.with(rows::all_epic_members).await
    }

    pub async fn agents_with_status(&self, statuses: Vec<AgentStatus>) -> Result<Vec<Agent>> {
        self.with(move |c| rows::select_agents_with_status(c, &statuses))
            .await
    }

    // -- sessions -----------------------------------------------------------

    /// Insert or refresh a session row. Returns true the first time we see it.
    pub async fn touch_session(&self, row: SessionRow) -> Result<bool> {
        self.with(move |c| rows::upsert_session(c, &row)).await
    }

    pub async fn session(&self, session_id: Uuid) -> Result<Option<SessionRow>> {
        self.with(move |c| rows::select_session(c, session_id))
            .await
    }

    // -- usage --------------------------------------------------------------

    /// Record one API response, merging it with what we already knew.
    pub async fn record_usage(&self, s: UsageSample) -> Result<Recorded> {
        self.with(move |c| rows::record_usage(c, &s)).await
    }

    /// Record many responses in one transaction. Returns how many rows were
    /// new or updated.
    pub async fn record_usage_batch(&self, samples: Vec<UsageSample>) -> Result<usize> {
        self.with(move |c| {
            let tx = c.transaction()?;
            let mut n = 0;
            for s in &samples {
                if rows::record_usage(&tx, s)?.changed() {
                    n += 1;
                }
            }
            tx.commit()?;
            Ok(n)
        })
        .await
    }

    /// Aggregated usage, split by model so the caller can price each one
    /// exactly rather than averaging rates.
    pub async fn usage_breakdown(&self, q: UsageQuery) -> Result<Vec<UsageBreakdown>> {
        self.with(move |c| rows::usage_breakdown(c, &q)).await
    }

    /// Token totals for one agent, plus how many assistant turns it took.
    pub async fn agent_usage(&self, agent_id: AgentId) -> Result<(Tokens, u32)> {
        self.with(move |c| rows::agent_usage(c, agent_id)).await
    }

    pub async fn ticket_usage(&self, ticket_id: TicketId) -> Result<Tokens> {
        self.with(move |c| rows::ticket_usage(c, ticket_id)).await
    }

    // -- transcript cursors -------------------------------------------------

    pub async fn transcript_cursor(&self, path: PathBuf) -> Result<Option<(u64, u64)>> {
        self.with(move |c| rows::select_cursor(c, &path)).await
    }

    pub async fn save_transcript_cursor(
        &self,
        path: PathBuf,
        inode: u64,
        offset: u64,
        session_id: Option<Uuid>,
        subagent_id: Option<String>,
    ) -> Result<()> {
        self.with(move |c| {
            rows::upsert_cursor(c, &path, inode, offset, session_id, subagent_id.as_deref())
        })
        .await
    }

    pub async fn transcript_file_count(&self) -> Result<usize> {
        self.with(|c| {
            let n: i64 = c.query_row("SELECT COUNT(*) FROM transcript_files", [], |r| r.get(0))?;
            Ok(n as usize)
        })
        .await
    }
}

/// Group key helpers shared by the rollup and the CLI.
pub fn group_key_label(g: GroupBy) -> &'static str {
    g.as_str()
}

/// Mirrors the `Usage` event so callers can persist and broadcast in one step.
pub fn usage_event(sample: UsageSample) -> NewEvent {
    let project_id = sample.project_id;
    let ticket_id = sample.ticket_id;
    let agent_id = sample.agent_id;
    let mut e = NewEvent::new(EventKind::Usage {
        sample: Box::new(sample),
    });
    e.project_id = project_id;
    e.ticket_id = ticket_id;
    e.agent_id = agent_id;
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::model::{ProjectKind, Tokens, UsageSource};

    async fn store_with_project() -> (Store, Project) {
        let store = Store::open_memory().unwrap();
        let project = Project {
            id: Uuid::new_v4(),
            name: "decouvert".into(),
            path: PathBuf::from("/tmp/disparu"),
            default_branch: "main".into(),
            zellij_tab: None,
            kind: ProjectKind::Discovered,
            created_at: orchestra_core::now(),
        };
        store.insert_project(project.clone()).await.unwrap();
        (store, project)
    }

    fn sample(project: ProjectId) -> UsageSample {
        UsageSample {
            message_id: "msg_1".into(),
            session_id: Uuid::new_v4(),
            subagent_id: None,
            agent_id: None,
            ticket_id: None,
            project_id: Some(project),
            model: "claude-opus-5".into(),
            tokens: Tokens {
                input: 1,
                output: 2,
                cache_read: 3,
                cache_creation: 0,
                thinking: 0,
            },
            ts: orchestra_core::now(),
            source: UsageSource::Transcript,
        }
    }

    #[tokio::test]
    async fn forgetting_a_project_keeps_what_it_cost() {
        // The tokens were really spent: dropping them would make the totals lie.
        let (store, project) = store_with_project().await;
        store.record_usage(sample(project.id)).await.unwrap();

        store.delete_project(project.id).await.unwrap();
        assert!(store.project(project.id).await.unwrap().is_none());

        let q = orchestra_core::protocol::UsageQuery::default();
        let breakdown = store.usage_breakdown(q).await.unwrap();
        assert_eq!(breakdown.len(), 1, "l'échantillon est toujours là");
        assert_eq!(breakdown[0].tokens.output, 2);
        assert_eq!(
            breakdown[0].keys.get("project").map(String::as_str),
            Some("(hors projet)"),
            "il n'appartient simplement plus à aucun projet"
        );
    }

    #[tokio::test]
    async fn a_project_with_tickets_is_not_forgotten_by_accident() {
        let (store, project) = store_with_project().await;
        let now = orchestra_core::now();
        store
            .insert_ticket(Ticket {
                id: Uuid::new_v4(),
                project_id: project.id,
                number: 1,
                title: "t".into(),
                brief: "b".into(),
                status: TicketStatus::Draft,
                branch: None,
                worktree_path: None,
                proposal: None,
                team: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let err = store.delete_project(project.id).await.unwrap_err();
        assert!(err.to_string().contains("ticket"), "{err}");
        assert!(store.project(project.id).await.unwrap().is_some());
    }
}
