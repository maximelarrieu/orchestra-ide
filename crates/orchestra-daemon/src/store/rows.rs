//! Row mapping: SQL in, domain types out. Nothing here is async.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use orchestra_core::claude::merge_tokens;
use orchestra_core::events::{Event, EventFilter, EventKind, NewEvent};
use orchestra_core::model::{
    Agent, AgentId, AgentStatus, Effort, ExitReason, Project, ProjectId, ProjectKind, Team,
    TeamProposal, Ticket, TicketId, TicketStatus, Tokens, UsageSample,
};
use orchestra_core::protocol::{GroupBy, UsageQuery};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Scalar helpers
// ---------------------------------------------------------------------------

fn ts(v: OffsetDateTime) -> String {
    v.format(&Rfc3339)
        .expect("un OffsetDateTime est formatable")
}

fn uuid_of(row: &Row<'_>, idx: &str) -> rusqlite::Result<Uuid> {
    let s: String = row.get(idx)?;
    Uuid::parse_str(&s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn opt_uuid(row: &Row<'_>, idx: &str) -> rusqlite::Result<Option<Uuid>> {
    let s: Option<String> = row.get(idx)?;
    Ok(s.and_then(|s| Uuid::parse_str(&s).ok()))
}

fn time_of(row: &Row<'_>, idx: &str) -> rusqlite::Result<OffsetDateTime> {
    let s: String = row.get(idx)?;
    OffsetDateTime::parse(&s, &Rfc3339).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn opt_time(row: &Row<'_>, idx: &str) -> rusqlite::Result<Option<OffsetDateTime>> {
    let s: Option<String> = row.get(idx)?;
    Ok(s.and_then(|s| OffsetDateTime::parse(&s, &Rfc3339).ok()))
}

fn conv_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

pub fn insert_event(conn: &mut Connection, e: NewEvent) -> Result<Event> {
    let payload = serde_json::to_string(&e.kind)?;
    let tag = e.kind.tag().as_str();
    conn.execute(
        "INSERT INTO events (ts, project_id, ticket_id, agent_id, kind, payload)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            ts(e.ts),
            e.project_id.map(|v| v.to_string()),
            e.ticket_id.map(|v| v.to_string()),
            e.agent_id.map(|v| v.to_string()),
            tag,
            payload,
        ],
    )?;
    Ok(Event::from_new(conn.last_insert_rowid(), e))
}

fn event_from_row(row: &Row<'_>) -> rusqlite::Result<Event> {
    let payload: String = row.get("payload")?;
    let kind: EventKind = serde_json::from_str(&payload).map_err(conv_err)?;
    Ok(Event {
        seq: row.get("seq")?,
        ts: time_of(row, "ts")?,
        project_id: opt_uuid(row, "project_id")?,
        ticket_id: opt_uuid(row, "ticket_id")?,
        agent_id: opt_uuid(row, "agent_id")?,
        kind,
    })
}

/// Build the `WHERE` fragment and its parameters for a filter.
fn filter_sql(f: &EventFilter) -> (String, Vec<String>) {
    let mut clauses = Vec::new();
    let mut args: Vec<String> = Vec::new();
    if let Some(p) = f.project_id {
        args.push(p.to_string());
        clauses.push(format!("project_id = ?{}", args.len()));
    }
    if let Some(t) = f.ticket_id {
        args.push(t.to_string());
        clauses.push(format!("ticket_id = ?{}", args.len()));
    }
    if let Some(a) = f.agent_id {
        args.push(a.to_string());
        clauses.push(format!("agent_id = ?{}", args.len()));
    }
    if !f.tags.is_empty() {
        let mut placeholders = Vec::new();
        for tag in &f.tags {
            args.push(tag.as_str().to_string());
            placeholders.push(format!("?{}", args.len()));
        }
        clauses.push(format!("kind IN ({})", placeholders.join(", ")));
    }
    if f.exclude_verbose {
        clauses.push(
            "kind NOT IN ('agent_text', 'agent_thinking', 'tool_started', 'tool_finished')".into(),
        );
    }
    (clauses.join(" AND "), args)
}

pub fn select_events_since(
    conn: &mut Connection,
    since_seq: i64,
    filter: &EventFilter,
    limit: u32,
) -> Result<Vec<Event>> {
    let (where_sql, mut args) = filter_sql(filter);
    let mut sql = String::from("SELECT * FROM events WHERE seq > ?");
    // `since_seq` is bound last so the filter's ?N indices stay valid.
    let seq_idx = args.len() + 1;
    sql = sql.replace("seq > ?", &format!("seq > ?{seq_idx}"));
    args.push(since_seq.to_string());
    if !where_sql.is_empty() {
        sql.push_str(" AND ");
        sql.push_str(&where_sql);
    }
    sql.push_str(&format!(" ORDER BY seq ASC LIMIT {limit}"));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), event_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn select_recent_events(
    conn: &mut Connection,
    filter: &EventFilter,
    limit: u32,
) -> Result<Vec<Event>> {
    let (where_sql, args) = filter_sql(filter);
    let mut sql = String::from("SELECT * FROM events");
    if !where_sql.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_sql);
    }
    sql.push_str(&format!(" ORDER BY seq DESC LIMIT {limit}"));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), event_from_row)?;
    let mut out = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    out.reverse();
    Ok(out)
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

pub fn insert_project(conn: &mut Connection, p: &Project) -> Result<()> {
    conn.execute(
        "INSERT INTO projects (id, name, path, default_branch, zellij_tab, kind, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            p.id.to_string(),
            p.name,
            p.path.to_string_lossy(),
            p.default_branch,
            p.zellij_tab,
            p.kind.as_str(),
            ts(p.created_at),
        ],
    )?;
    Ok(())
}

fn project_from_row(row: &Row<'_>) -> rusqlite::Result<Project> {
    let kind: String = row.get("kind")?;
    let path: String = row.get("path")?;
    Ok(Project {
        id: uuid_of(row, "id")?,
        name: row.get("name")?,
        path: PathBuf::from(path),
        default_branch: row.get("default_branch")?,
        zellij_tab: row.get("zellij_tab")?,
        kind: ProjectKind::parse(&kind).map_err(conv_err)?,
        created_at: time_of(row, "created_at")?,
    })
}

pub fn select_projects(conn: &mut Connection) -> Result<Vec<Project>> {
    let mut stmt = conn.prepare("SELECT * FROM projects ORDER BY name")?;
    let rows = stmt.query_map([], project_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn select_project(conn: &mut Connection, id: ProjectId) -> Result<Option<Project>> {
    let mut stmt = conn.prepare("SELECT * FROM projects WHERE id = ?1")?;
    Ok(stmt
        .query_row([id.to_string()], project_from_row)
        .optional()?)
}

pub fn select_project_by_path(conn: &mut Connection, path: &Path) -> Result<Option<Project>> {
    let mut stmt = conn.prepare("SELECT * FROM projects WHERE path = ?1")?;
    Ok(stmt
        .query_row([path.to_string_lossy()], project_from_row)
        .optional()?)
}

/// Longest project path that is a prefix of `cwd`, comparing whole components
/// so `/a/bc` never matches `/a/b`.
/// Forget a project, keeping what it cost.
///
/// The tokens were really spent, so the samples stay and simply lose their
/// project: dropping them would make the totals lie.
pub fn delete_project(conn: &mut Connection, id: ProjectId) -> Result<()> {
    let tx = conn.transaction()?;
    let tickets: i64 = tx.query_row(
        "SELECT COUNT(*) FROM tickets WHERE project_id = ?1",
        [id.to_string()],
        |r| r.get(0),
    )?;
    anyhow::ensure!(
        tickets == 0,
        "ce projet a {tickets} ticket(s) : supprime-les d'abord"
    );
    tx.execute(
        "UPDATE usage_samples SET project_id = NULL WHERE project_id = ?1",
        [id.to_string()],
    )?;
    tx.execute(
        "UPDATE sessions SET project_id = NULL WHERE project_id = ?1",
        [id.to_string()],
    )?;
    let removed = tx.execute("DELETE FROM projects WHERE id = ?1", [id.to_string()])?;
    anyhow::ensure!(removed == 1, "projet introuvable");
    tx.commit()?;
    Ok(())
}

pub fn select_project_containing(conn: &mut Connection, cwd: &Path) -> Result<Option<Project>> {
    let all = select_projects(conn)?;
    Ok(all
        .into_iter()
        .filter(|p| cwd.starts_with(&p.path))
        .max_by_key(|p| p.path.components().count()))
}

// ---------------------------------------------------------------------------
// Tickets
// ---------------------------------------------------------------------------

pub fn insert_ticket(conn: &mut Connection, t: &Ticket) -> Result<()> {
    conn.execute(
        "INSERT INTO tickets
           (id, project_id, number, title, brief, status, branch, worktree_path,
            proposal_json, team_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            t.id.to_string(),
            t.project_id.to_string(),
            t.number,
            t.title,
            t.brief,
            t.status.as_str(),
            t.branch,
            t.worktree_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            t.proposal.as_ref().map(serde_json::to_string).transpose()?,
            t.team.as_ref().map(serde_json::to_string).transpose()?,
            ts(t.created_at),
            ts(t.updated_at),
        ],
    )?;
    Ok(())
}

pub fn update_ticket(conn: &mut Connection, t: &Ticket) -> Result<()> {
    let n = conn.execute(
        "UPDATE tickets SET title = ?2, brief = ?3, status = ?4, branch = ?5,
             worktree_path = ?6, proposal_json = ?7, team_json = ?8, updated_at = ?9
         WHERE id = ?1",
        params![
            t.id.to_string(),
            t.title,
            t.brief,
            t.status.as_str(),
            t.branch,
            t.worktree_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            t.proposal.as_ref().map(serde_json::to_string).transpose()?,
            t.team.as_ref().map(serde_json::to_string).transpose()?,
            ts(t.updated_at),
        ],
    )?;
    anyhow::ensure!(n == 1, "ticket {} introuvable", t.id);
    Ok(())
}

fn ticket_from_row(row: &Row<'_>) -> rusqlite::Result<Ticket> {
    let status: String = row.get("status")?;
    let worktree: Option<String> = row.get("worktree_path")?;
    let proposal: Option<String> = row.get("proposal_json")?;
    let team: Option<String> = row.get("team_json")?;
    Ok(Ticket {
        id: uuid_of(row, "id")?,
        project_id: uuid_of(row, "project_id")?,
        number: row.get("number")?,
        title: row.get("title")?,
        brief: row.get("brief")?,
        status: TicketStatus::parse(&status).map_err(conv_err)?,
        branch: row.get("branch")?,
        worktree_path: worktree.map(PathBuf::from),
        proposal: proposal
            .map(|s| serde_json::from_str::<TeamProposal>(&s))
            .transpose()
            .map_err(conv_err)?,
        team: team
            .map(|s| serde_json::from_str::<Team>(&s))
            .transpose()
            .map_err(conv_err)?,
        created_at: time_of(row, "created_at")?,
        updated_at: time_of(row, "updated_at")?,
    })
}

pub fn select_ticket(conn: &mut Connection, id: TicketId) -> Result<Option<Ticket>> {
    let mut stmt = conn.prepare("SELECT * FROM tickets WHERE id = ?1")?;
    Ok(stmt
        .query_row([id.to_string()], ticket_from_row)
        .optional()?)
}

/// The ticket whose worktree contains `cwd`, if any.
///
/// A worktree is a git repository in its own right, so without this every
/// ticket would appear as a separate project in the cost view.
pub fn select_ticket_by_worktree(conn: &mut Connection, cwd: &Path) -> Result<Option<Ticket>> {
    let mut stmt = conn.prepare("SELECT * FROM tickets WHERE worktree_path IS NOT NULL")?;
    let rows = stmt.query_map([], ticket_from_row)?;
    let mut best: Option<Ticket> = None;
    for ticket in rows {
        let ticket = ticket?;
        let Some(path) = ticket.worktree_path.as_ref() else {
            continue;
        };
        if cwd.starts_with(path) {
            let longer = best
                .as_ref()
                .and_then(|b| b.worktree_path.as_ref())
                .map(|p| path.components().count() > p.components().count())
                .unwrap_or(true);
            if longer {
                best = Some(ticket);
            }
        }
    }
    Ok(best)
}

pub fn select_tickets(
    conn: &mut Connection,
    project_id: Option<ProjectId>,
    status: Option<&[TicketStatus]>,
) -> Result<Vec<Ticket>> {
    let mut clauses: Vec<String> = Vec::new();
    let mut args: Vec<String> = Vec::new();
    if let Some(p) = project_id {
        args.push(p.to_string());
        clauses.push(format!("project_id = ?{}", args.len()));
    }
    if let Some(sts) = status.filter(|s| !s.is_empty()) {
        let mut ph = Vec::new();
        for s in sts {
            args.push(s.as_str().to_string());
            ph.push(format!("?{}", args.len()));
        }
        clauses.push(format!("status IN ({})", ph.join(", ")));
    }
    let mut sql = String::from("SELECT * FROM tickets");
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY number DESC");

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), ticket_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// ---------------------------------------------------------------------------
// Agents
// ---------------------------------------------------------------------------

pub fn insert_agent(conn: &mut Connection, a: &Agent) -> Result<()> {
    conn.execute(
        "INSERT INTO agents
           (id, ticket_id, project_id, role, objective, stage, session_id, model, effort,
            max_budget_usd, status, exit_reason, pid, pane_id, attempt, handoff,
            started_at, ended_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            a.id.to_string(),
            a.ticket_id.to_string(),
            a.project_id.to_string(),
            a.role,
            a.objective,
            a.stage,
            a.session_id.to_string(),
            a.model,
            a.effort.as_str(),
            a.max_budget_usd,
            a.status.as_str(),
            a.exit_reason
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
            a.pid,
            a.pane_id,
            a.attempt,
            a.handoff,
            a.started_at.map(ts),
            a.ended_at.map(ts),
        ],
    )?;
    Ok(())
}

pub fn update_agent(conn: &mut Connection, a: &Agent) -> Result<()> {
    let n = conn.execute(
        "UPDATE agents SET role = ?2, objective = ?3, stage = ?4, model = ?5, effort = ?6,
             max_budget_usd = ?7, status = ?8, exit_reason = ?9, pid = ?10, pane_id = ?11,
             attempt = ?12, handoff = ?13, started_at = ?14, ended_at = ?15
         WHERE id = ?1",
        params![
            a.id.to_string(),
            a.role,
            a.objective,
            a.stage,
            a.model,
            a.effort.as_str(),
            a.max_budget_usd,
            a.status.as_str(),
            a.exit_reason
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
            a.pid,
            a.pane_id,
            a.attempt,
            a.handoff,
            a.started_at.map(ts),
            a.ended_at.map(ts),
        ],
    )?;
    anyhow::ensure!(n == 1, "agent {} introuvable", a.id);
    Ok(())
}

fn agent_from_row(row: &Row<'_>) -> rusqlite::Result<Agent> {
    let status: String = row.get("status")?;
    let effort: String = row.get("effort")?;
    let exit: Option<String> = row.get("exit_reason")?;
    Ok(Agent {
        id: uuid_of(row, "id")?,
        ticket_id: uuid_of(row, "ticket_id")?,
        project_id: uuid_of(row, "project_id")?,
        role: row.get("role")?,
        objective: row.get("objective")?,
        stage: row.get("stage")?,
        session_id: uuid_of(row, "session_id")?,
        model: row.get("model")?,
        effort: Effort::parse(&effort).map_err(conv_err)?,
        max_budget_usd: row.get("max_budget_usd")?,
        status: AgentStatus::parse(&status).map_err(conv_err)?,
        exit_reason: exit
            .map(|s| serde_json::from_str::<ExitReason>(&s))
            .transpose()
            .map_err(conv_err)?,
        pid: row.get("pid")?,
        pane_id: row.get("pane_id")?,
        attempt: row.get("attempt")?,
        handoff: row.get("handoff")?,
        started_at: opt_time(row, "started_at")?,
        ended_at: opt_time(row, "ended_at")?,
    })
}

pub fn select_agent(conn: &mut Connection, id: AgentId) -> Result<Option<Agent>> {
    let mut stmt = conn.prepare("SELECT * FROM agents WHERE id = ?1")?;
    Ok(stmt
        .query_row([id.to_string()], agent_from_row)
        .optional()?)
}

pub fn select_agent_by_session(conn: &mut Connection, session_id: Uuid) -> Result<Option<Agent>> {
    let mut stmt = conn.prepare("SELECT * FROM agents WHERE session_id = ?1")?;
    Ok(stmt
        .query_row([session_id.to_string()], agent_from_row)
        .optional()?)
}

pub fn select_agents_of_ticket(conn: &mut Connection, ticket_id: TicketId) -> Result<Vec<Agent>> {
    let mut stmt =
        conn.prepare("SELECT * FROM agents WHERE ticket_id = ?1 ORDER BY stage, role")?;
    let rows = stmt.query_map([ticket_id.to_string()], agent_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn select_agents_with_status(
    conn: &mut Connection,
    statuses: &[AgentStatus],
) -> Result<Vec<Agent>> {
    if statuses.is_empty() {
        return Ok(Vec::new());
    }
    let ph: Vec<String> = (1..=statuses.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "SELECT * FROM agents WHERE status IN ({}) ORDER BY started_at",
        ph.join(", ")
    );
    let args: Vec<String> = statuses.iter().map(|s| s.as_str().to_string()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), agent_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub session_id: Uuid,
    pub cwd: PathBuf,
    pub project_id: Option<ProjectId>,
    pub agent_id: Option<AgentId>,
    pub managed: bool,
    pub name: Option<String>,
    pub first_seen: OffsetDateTime,
    pub last_seen: OffsetDateTime,
    pub claude_version: Option<String>,
}

/// Returns true when the row did not exist yet.
pub fn upsert_session(conn: &mut Connection, s: &SessionRow) -> Result<bool> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sessions WHERE session_id = ?1",
            [s.session_id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    if existing.is_some() {
        conn.execute(
            "UPDATE sessions SET cwd = ?2, project_id = COALESCE(?3, project_id),
                 agent_id = COALESCE(?4, agent_id), managed = ?5,
                 name = COALESCE(?6, name), last_seen = ?7,
                 claude_version = COALESCE(?8, claude_version)
             WHERE session_id = ?1",
            params![
                s.session_id.to_string(),
                s.cwd.to_string_lossy(),
                s.project_id.map(|v| v.to_string()),
                s.agent_id.map(|v| v.to_string()),
                s.managed as i64,
                s.name,
                ts(s.last_seen),
                s.claude_version,
            ],
        )?;
        Ok(false)
    } else {
        conn.execute(
            "INSERT INTO sessions
               (session_id, cwd, project_id, agent_id, managed, name, first_seen, last_seen, claude_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                s.session_id.to_string(),
                s.cwd.to_string_lossy(),
                s.project_id.map(|v| v.to_string()),
                s.agent_id.map(|v| v.to_string()),
                s.managed as i64,
                s.name,
                ts(s.first_seen),
                ts(s.last_seen),
                s.claude_version,
            ],
        )?;
        Ok(true)
    }
}

pub fn select_session(conn: &mut Connection, session_id: Uuid) -> Result<Option<SessionRow>> {
    let mut stmt = conn.prepare("SELECT * FROM sessions WHERE session_id = ?1")?;
    Ok(stmt
        .query_row([session_id.to_string()], |row| {
            let cwd: String = row.get("cwd")?;
            let managed: i64 = row.get("managed")?;
            Ok(SessionRow {
                session_id: uuid_of(row, "session_id")?,
                cwd: PathBuf::from(cwd),
                project_id: opt_uuid(row, "project_id")?,
                agent_id: opt_uuid(row, "agent_id")?,
                managed: managed != 0,
                name: row.get("name")?,
                first_seen: time_of(row, "first_seen")?,
                last_seen: time_of(row, "last_seen")?,
                claude_version: row.get("claude_version")?,
            })
        })
        .optional()?)
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

/// What happened when a sample was recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    /// First time we hear about this API response.
    New,
    /// We already had it, and this copy was more complete.
    Updated,
    /// Nothing new; the usual outcome for the second source reporting the
    /// same response.
    Unchanged,
}

impl Recorded {
    pub fn changed(self) -> bool {
        !matches!(self, Recorded::Unchanged)
    }
}

/// Record one API response.
///
/// The same `message_id` reaches us repeatedly: once per content block in the
/// transcript, sometimes twice for the same block, and again on the stdout of a
/// managed agent. Those copies are not equal, because the early ones carry a
/// partial `output_tokens` while the response is still streaming. Keeping the
/// first would under-report the cost badly, so each field keeps the largest
/// value seen. The merge is commutative, so the order the two sources arrive in
/// does not matter.
pub fn record_usage(conn: &Connection, s: &UsageSample) -> Result<Recorded> {
    let existing: Option<Tokens> = conn
        .query_row(
            "SELECT input, output, cache_read, cache_creation, thinking
             FROM usage_samples WHERE message_id = ?1",
            [&s.message_id],
            |row| {
                Ok(Tokens {
                    input: row.get::<_, i64>(0)? as u64,
                    output: row.get::<_, i64>(1)? as u64,
                    cache_read: row.get::<_, i64>(2)? as u64,
                    cache_creation: row.get::<_, i64>(3)? as u64,
                    thinking: row.get::<_, i64>(4)? as u64,
                })
            },
        )
        .optional()?;

    match existing {
        None => {
            conn.execute(
                "INSERT INTO usage_samples
                   (message_id, session_id, subagent_id, agent_id, ticket_id, project_id, model,
                    ts, source, input, output, cache_read, cache_creation, thinking)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    s.message_id,
                    s.session_id.to_string(),
                    s.subagent_id,
                    s.agent_id.map(|v| v.to_string()),
                    s.ticket_id.map(|v| v.to_string()),
                    s.project_id.map(|v| v.to_string()),
                    s.model,
                    ts(s.ts),
                    s.source.as_str(),
                    s.tokens.input as i64,
                    s.tokens.output as i64,
                    s.tokens.cache_read as i64,
                    s.tokens.cache_creation as i64,
                    s.tokens.thinking as i64,
                ],
            )?;
            Ok(Recorded::New)
        }
        Some(old) => {
            let merged = merge_tokens(old, s.tokens);
            if merged == old {
                return Ok(Recorded::Unchanged);
            }
            conn.execute(
                "UPDATE usage_samples SET
                     input = ?2, output = ?3, cache_read = ?4, cache_creation = ?5, thinking = ?6,
                     agent_id = COALESCE(agent_id, ?7),
                     ticket_id = COALESCE(ticket_id, ?8),
                     project_id = COALESCE(project_id, ?9),
                     subagent_id = COALESCE(subagent_id, ?10),
                     model = CASE WHEN model = '' THEN ?11 ELSE model END
                 WHERE message_id = ?1",
                params![
                    s.message_id,
                    merged.input as i64,
                    merged.output as i64,
                    merged.cache_read as i64,
                    merged.cache_creation as i64,
                    merged.thinking as i64,
                    s.agent_id.map(|v| v.to_string()),
                    s.ticket_id.map(|v| v.to_string()),
                    s.project_id.map(|v| v.to_string()),
                    s.subagent_id,
                    s.model,
                ],
            )?;
            Ok(Recorded::Updated)
        }
    }
}

fn tokens_from_row(row: &Row<'_>) -> rusqlite::Result<Tokens> {
    Ok(Tokens {
        input: row.get::<_, i64>("input")? as u64,
        output: row.get::<_, i64>("output")? as u64,
        cache_read: row.get::<_, i64>("cache_read")? as u64,
        cache_creation: row.get::<_, i64>("cache_creation")? as u64,
        thinking: row.get::<_, i64>("thinking")? as u64,
    })
}

pub fn agent_usage(conn: &mut Connection, agent_id: AgentId) -> Result<(Tokens, u32)> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(SUM(input), 0) AS input, COALESCE(SUM(output), 0) AS output,
                COALESCE(SUM(cache_read), 0) AS cache_read,
                COALESCE(SUM(cache_creation), 0) AS cache_creation,
                COALESCE(SUM(thinking), 0) AS thinking, COUNT(*) AS n
         FROM usage_samples WHERE agent_id = ?1",
    )?;
    let out = stmt.query_row([agent_id.to_string()], |row| {
        Ok((tokens_from_row(row)?, row.get::<_, i64>("n")? as u32))
    })?;
    Ok(out)
}

pub fn ticket_usage(conn: &mut Connection, ticket_id: TicketId) -> Result<Tokens> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(SUM(input), 0) AS input, COALESCE(SUM(output), 0) AS output,
                COALESCE(SUM(cache_read), 0) AS cache_read,
                COALESCE(SUM(cache_creation), 0) AS cache_creation,
                COALESCE(SUM(thinking), 0) AS thinking
         FROM usage_samples WHERE ticket_id = ?1",
    )?;
    Ok(stmt.query_row([ticket_id.to_string()], tokens_from_row)?)
}

/// SQL expression and label for one grouping dimension.
fn group_expr(g: GroupBy) -> (&'static str, &'static str) {
    match g {
        // Project, ticket and agent resolve to human labels via a sub-select.
        GroupBy::Project => (
            "COALESCE((SELECT name FROM projects WHERE projects.id = u.project_id), '(hors projet)')",
            "project",
        ),
        GroupBy::Ticket => (
            "COALESCE((SELECT '#' || number || ' ' || title FROM tickets WHERE tickets.id = u.ticket_id), '(hors ticket)')",
            "ticket",
        ),
        GroupBy::Agent | GroupBy::Role => (
            "COALESCE((SELECT role FROM agents WHERE agents.id = u.agent_id), '(session libre)')",
            "role",
        ),
        GroupBy::Model => ("u.model", "model"),
        GroupBy::Day => ("date(u.ts)", "day"),
    }
}

/// One group, split by model.
///
/// The split matters: a project that used two models cannot be priced by a
/// single rate, and averaging them produces a number that does not match the
/// sum of its parts. The caller prices each model exactly and adds them up.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageBreakdown {
    pub keys: BTreeMap<String, String>,
    pub model: String,
    pub tokens: Tokens,
    pub messages: u64,
}

/// Hard ceiling on returned groups, so a pathological query cannot blow up
/// memory. Far above anything a single machine produces.
const MAX_GROUPS: u32 = 20_000;

pub fn usage_breakdown(conn: &mut Connection, q: &UsageQuery) -> Result<Vec<UsageBreakdown>> {
    let mut clauses: Vec<String> = Vec::new();
    let mut args: Vec<String> = Vec::new();
    if let Some(since) = q.range.since {
        args.push(ts(since));
        clauses.push(format!("u.ts >= ?{}", args.len()));
    }
    if let Some(until) = q.range.until {
        args.push(ts(until));
        clauses.push(format!("u.ts < ?{}", args.len()));
    }
    if let Some(p) = q.project_id {
        args.push(p.to_string());
        clauses.push(format!("u.project_id = ?{}", args.len()));
    }
    if let Some(t) = q.ticket_id {
        args.push(t.to_string());
        clauses.push(format!("u.ticket_id = ?{}", args.len()));
    }
    if let Some(a) = q.agent_id {
        args.push(a.to_string());
        clauses.push(format!("u.agent_id = ?{}", args.len()));
    }
    if !q.include_unmanaged {
        clauses.push("u.agent_id IS NOT NULL".into());
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };

    let groups: Vec<GroupBy> = if q.group_by.is_empty() {
        vec![GroupBy::Project]
    } else {
        q.group_by.clone()
    };
    let mut selects: Vec<String> = groups
        .iter()
        .enumerate()
        .map(|(i, g)| format!("{} AS k{i}", group_expr(*g).0))
        .collect();
    let mut group_by: Vec<String> = (0..groups.len()).map(|i| format!("k{i}")).collect();
    // The model is always a grouping dimension, whether or not it is displayed.
    selects.push("u.model AS model".into());
    group_by.push("model".into());

    let sql = format!(
        "SELECT {}, SUM(u.input) AS input, SUM(u.output) AS output,
                SUM(u.cache_read) AS cache_read, SUM(u.cache_creation) AS cache_creation,
                SUM(u.thinking) AS thinking, COUNT(*) AS messages
         FROM usage_samples u{where_sql}
         GROUP BY {}
         LIMIT {MAX_GROUPS}",
        selects.join(", "),
        group_by.join(", "),
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), |row| {
        let mut keys = BTreeMap::new();
        for (i, g) in groups.iter().enumerate() {
            let v: Option<String> = row.get(format!("k{i}").as_str())?;
            keys.insert(
                group_expr(*g).1.to_string(),
                v.unwrap_or_else(|| "(inconnu)".into()),
            );
        }
        Ok(UsageBreakdown {
            keys,
            model: row.get::<_, Option<String>>("model")?.unwrap_or_default(),
            tokens: tokens_from_row(row)?,
            messages: row.get::<_, i64>("messages")? as u64,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// ---------------------------------------------------------------------------
// Transcript cursors
// ---------------------------------------------------------------------------

pub fn select_cursor(conn: &mut Connection, path: &Path) -> Result<Option<(u64, u64)>> {
    let mut stmt = conn.prepare("SELECT inode, offset FROM transcript_files WHERE path = ?1")?;
    Ok(stmt
        .query_row([path.to_string_lossy()], |r| {
            Ok((
                r.get::<_, Option<i64>>(0)?.unwrap_or(0) as u64,
                r.get::<_, i64>(1)? as u64,
            ))
        })
        .optional()?)
}

pub fn upsert_cursor(
    conn: &mut Connection,
    path: &Path,
    inode: u64,
    offset: u64,
    session_id: Option<Uuid>,
    subagent_id: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO transcript_files (path, inode, offset, session_id, subagent_id, last_seen)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(path) DO UPDATE SET inode = ?2, offset = ?3,
             session_id = COALESCE(?4, session_id), subagent_id = COALESCE(?5, subagent_id),
             last_seen = ?6",
        params![
            path.to_string_lossy(),
            inode as i64,
            offset as i64,
            session_id.map(|v| v.to_string()),
            subagent_id,
            ts(orchestra_core::now()),
        ],
    )?;
    Ok(())
}
