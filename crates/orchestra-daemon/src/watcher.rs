//! Tails Claude Code's own transcripts so Orchestra accounts for every session
//! on the machine, including the ones the user starts by hand.
//!
//! Layout is `~/.claude/projects/<slug-du-cwd>/<session>.jsonl`, plus
//! `<session>/subagents/agent-<id>.jsonl`. Files only grow, so a byte offset
//! per file is enough to resume; the offset is persisted, so a daemon restart
//! does not re-read everything.
//!
//! Detection is by polling rather than inotify. With a handful of files a
//! one-second scan costs almost nothing, and it avoids inotify watch limits on
//! a directory that grows with every session, plus a dependency that is still
//! a release candidate. For a managed agent the low-latency path is its own
//! stdout anyway; this watcher is the safety net that also covers everyone else.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use orchestra_core::claude::{is_assistant_line, TranscriptLine, TranscriptPath};
use orchestra_core::events::{EventKind, NewEvent};
use orchestra_core::model::{ProjectId, UsageSample, UsageSource};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::bus::EventBus;
use crate::ledger::UsageLedger;
use crate::store::{discovered_project, SessionRow, Store};

/// How often the transcript tree is rescanned.
const POLL: Duration = Duration::from_millis(1000);

/// A line longer than this is not a transcript line we can use; it is skipped
/// so a corrupt file cannot exhaust memory.
const MAX_LINE: usize = 8 * 1024 * 1024;

/// Where we stopped reading one file.
///
/// The offset always sits on a line boundary: a line Claude Code has not
/// finished writing is left for the next pass rather than buffered. That costs
/// a re-read of a few hundred bytes and buys an invariant worth having, since
/// the offset is persisted and a daemon restart must never resume mid-line.
#[derive(Debug, Clone)]
struct Cursor {
    inode: u64,
    offset: u64,
    session_id: Option<Uuid>,
    subagent_id: Option<String>,
}

pub struct TranscriptWatcher {
    root: PathBuf,
    store: Store,
    ledger: UsageLedger,
    bus: EventBus,
    cursors: HashMap<PathBuf, Cursor>,
    /// Sessions already resolved, so attribution does not hit the database on
    /// every line.
    attributed: HashMap<Uuid, Attribution>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Attribution {
    project_id: Option<ProjectId>,
    ticket_id: Option<Uuid>,
    agent_id: Option<Uuid>,
    managed: bool,
}

/// What one scan pass did, for logs and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub files: usize,
    pub lines: usize,
    pub samples_new: usize,
    pub samples_updated: usize,
    pub unreadable_lines: usize,
}

impl ScanReport {
    fn merge(&mut self, other: ScanReport) {
        self.files += other.files;
        self.lines += other.lines;
        self.samples_new += other.samples_new;
        self.samples_updated += other.samples_updated;
        self.unreadable_lines += other.unreadable_lines;
    }

    pub fn changed(&self) -> bool {
        self.samples_new > 0 || self.samples_updated > 0
    }
}

impl TranscriptWatcher {
    pub fn new(root: PathBuf, store: Store, ledger: UsageLedger, bus: EventBus) -> Self {
        TranscriptWatcher {
            root,
            store,
            ledger,
            bus,
            cursors: HashMap::new(),
            attributed: HashMap::new(),
        }
    }

    /// Catch up on what is already on disk, then poll until cancelled.
    pub async fn run(mut self, cancel: CancellationToken) -> Result<()> {
        if !self.root.exists() {
            tracing::info!(
                root = %self.root.display(),
                "aucun transcript Claude Code pour l'instant ; surveillance quand même active"
            );
        }
        let started = std::time::Instant::now();
        let report = self.scan_all().await;
        tracing::info!(
            files = report.files,
            lines = report.lines,
            nouveaux = report.samples_new,
            mis_a_jour = report.samples_updated,
            illisibles = report.unreadable_lines,
            duree_ms = started.elapsed().as_millis() as u64,
            "rattrapage des transcripts terminé"
        );
        if started.elapsed() > Duration::from_secs(5) {
            self.bus
                .warn(format!(
                    "rattrapage des transcripts : {} fichiers en {} s",
                    report.files,
                    started.elapsed().as_secs()
                ))
                .await;
        }

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!("arrêt du surveillant de transcripts");
                    return Ok(());
                }
                _ = tokio::time::sleep(POLL) => {
                    let report = self.scan_all().await;
                    if report.changed() {
                        tracing::debug!(
                            nouveaux = report.samples_new,
                            mis_a_jour = report.samples_updated,
                            "transcripts"
                        );
                    }
                }
            }
        }
    }

    /// One pass over every transcript file.
    pub async fn scan_all(&mut self) -> ScanReport {
        let mut report = ScanReport::default();
        let files = match list_transcripts(&self.root) {
            Ok(f) => f,
            Err(e) => {
                tracing::debug!("lecture de {} impossible : {e}", self.root.display());
                return report;
            }
        };
        report.files = files.len();
        for path in files {
            match self.scan_file(&path).await {
                Ok(r) => report.merge(ScanReport { files: 0, ..r }),
                Err(e) => tracing::debug!("{} : {e}", path.display()),
            }
        }
        report
    }

    /// Read whatever is new in one file.
    async fn scan_file(&mut self, path: &Path) -> Result<ScanReport> {
        let meta = std::fs::metadata(path)?;
        let inode = inode_of(&meta);
        let size = meta.len();

        let cursor = match self.cursors.get(path) {
            Some(c) => c.clone(),
            None => {
                // First sight in this process: resume from the stored offset.
                let stored = self
                    .store
                    .transcript_cursor(path.to_path_buf())
                    .await
                    .unwrap_or(None);
                let parsed = TranscriptPath::parse(path);
                let (stored_inode, offset) = stored.unwrap_or((inode, 0));
                Cursor {
                    inode: stored_inode,
                    // A different inode means a different file at the same path.
                    offset: if stored_inode == inode { offset } else { 0 },
                    session_id: parsed.session_id,
                    subagent_id: parsed.subagent_id,
                }
            }
        };

        let mut cursor = cursor;
        if cursor.inode != inode {
            cursor.inode = inode;
            cursor.offset = 0;
        }
        // Truncated or replaced in place: start over rather than read garbage.
        if size < cursor.offset {
            cursor.offset = 0;
        }
        if size == cursor.offset {
            self.cursors.insert(path.to_path_buf(), cursor);
            return Ok(ScanReport::default());
        }

        let mut file = std::fs::File::open(path)?;
        file.seek(SeekFrom::Start(cursor.offset))?;
        let mut data = Vec::new();
        file.take(size - cursor.offset)
            .read_to_end(&mut data)
            .with_context(|| format!("lecture de {}", path.display()))?;

        let mut report = ScanReport::default();
        let mut consumed = 0usize;
        for line in data.split_inclusive(|b| *b == b'\n') {
            if !line.ends_with(b"\n") {
                // Incomplete tail: keep it for the next pass.
                break;
            }
            consumed += line.len();
            report.lines += 1;
            let line = &line[..line.len() - 1];
            if line.len() > MAX_LINE || !is_assistant_line(line) {
                continue;
            }
            match self.handle_line(line, &cursor).await {
                Ok(Some(recorded)) => {
                    if recorded {
                        report.samples_new += 1;
                    } else {
                        report.samples_updated += 1;
                    }
                }
                Ok(None) => {}
                Err(_) => report.unreadable_lines += 1,
            }
        }

        // Everything after the last newline is a line still being written.
        cursor.offset += consumed as u64;
        let _ = self
            .store
            .save_transcript_cursor(
                path.to_path_buf(),
                cursor.inode,
                cursor.offset,
                cursor.session_id,
                cursor.subagent_id.clone(),
            )
            .await;
        self.cursors.insert(path.to_path_buf(), cursor);
        Ok(report)
    }

    /// Returns `Some(true)` when the sample was new, `Some(false)` when it
    /// completed one we already had, `None` when there was nothing to record.
    async fn handle_line(&mut self, line: &[u8], cursor: &Cursor) -> Result<Option<bool>> {
        let text = std::str::from_utf8(line).context("ligne non UTF-8")?;
        let parsed = TranscriptLine::parse(text).context("JSON illisible")?;
        let Some(a) = parsed.as_assistant() else {
            return Ok(None);
        };
        let Some(record) = a.usage_record() else {
            return Ok(None);
        };
        // Locally synthesised messages (errors, notices) cost nothing.
        if a.message.is_synthetic() {
            return Ok(None);
        }

        let session_id = a.session_id;
        let cwd = a.cwd.clone().unwrap_or_else(|| self.root.clone());
        let attribution = self
            .attribution(session_id, &cwd, a.version.as_deref())
            .await;

        let sample = UsageSample {
            message_id: record.message_id,
            session_id,
            subagent_id: a.agent_id.clone().or_else(|| cursor.subagent_id.clone()),
            agent_id: attribution.agent_id,
            ticket_id: attribution.ticket_id,
            project_id: attribution.project_id,
            model: record.model,
            tokens: record.tokens,
            ts: record.ts.unwrap_or_else(orchestra_core::now),
            source: UsageSource::Transcript,
        };

        let recorded = self.ledger.record(sample.clone()).await?;
        if !recorded.changed() {
            return Ok(None);
        }
        // Managed agents need their cost live on the agent screen; unmanaged
        // sessions would flood the log, and the cost view reads the base.
        if attribution.managed {
            let mut ev = NewEvent::new(EventKind::Usage {
                sample: Box::new(sample),
            });
            ev.project_id = attribution.project_id;
            ev.ticket_id = attribution.ticket_id;
            ev.agent_id = attribution.agent_id;
            let _ = self.bus.publish(ev).await;
        }
        Ok(Some(recorded == crate::store::Recorded::New))
    }

    /// Resolve a session once, then remember it.
    async fn attribution(
        &mut self,
        session_id: Uuid,
        cwd: &Path,
        version: Option<&str>,
    ) -> Attribution {
        if let Some(a) = self.attributed.get(&session_id) {
            return *a;
        }
        let mut attribution = Attribution::default();

        // A session we started: it carries the full ticket context.
        if let Ok(Some(agent)) = self.store.agent_by_session(session_id).await {
            attribution = Attribution {
                project_id: Some(agent.project_id),
                ticket_id: Some(agent.ticket_id),
                agent_id: Some(agent.id),
                managed: true,
            };
        } else if let Ok(Some(ticket)) = self.store.ticket_by_worktree(cwd.to_path_buf()).await {
            // A session run by hand inside a ticket's worktree belongs to that
            // ticket, not to a project of its own.
            attribution.project_id = Some(ticket.project_id);
            attribution.ticket_id = Some(ticket.id);
        } else {
            attribution.project_id = self.project_for(cwd).await;
        }

        let now = orchestra_core::now();
        let row = SessionRow {
            session_id,
            cwd: cwd.to_path_buf(),
            project_id: attribution.project_id,
            agent_id: attribution.agent_id,
            managed: attribution.managed,
            name: None,
            first_seen: now,
            last_seen: now,
            claude_version: version.map(str::to_string),
        };
        let first_time = self.store.touch_session(row).await.unwrap_or(false);
        if first_time && !attribution.managed {
            let mut ev = NewEvent::new(EventKind::UnmanagedSessionSeen {
                session_id,
                cwd: cwd.to_path_buf(),
            });
            ev.project_id = attribution.project_id;
            let _ = self.bus.publish(ev).await;
        }

        self.attributed.insert(session_id, attribution);
        attribution
    }

    /// The project a working directory belongs to.
    ///
    /// A git repository is the strongest signal, and it is consulted **before**
    /// any path-prefix match: without that, a single session started in the
    /// home directory creates a project there, and every later session then
    /// matches it by prefix and every repository disappears into one row.
    async fn project_for(&self, cwd: &Path) -> Option<ProjectId> {
        if let Some(root) = git_toplevel(cwd) {
            if let Ok(Some(p)) = self.store.project_by_path(root.clone()).await {
                return Some(p.id);
            }
            return self.create_discovered(&root).await;
        }
        // Outside a repository, an existing project that contains it will do.
        if let Ok(Some(p)) = self.store.project_containing(cwd.to_path_buf()).await {
            return Some(p.id);
        }
        // A home or system directory is not a project; its sessions are real
        // and counted, they simply belong to no repository.
        if is_too_broad(cwd, home_dir().as_deref()) {
            return None;
        }
        self.create_discovered(cwd).await
    }

    async fn create_discovered(&self, path: &Path) -> Option<ProjectId> {
        let project = discovered_project(path, orchestra_core::now());
        let id = project.id;
        match self.store.insert_project(project).await {
            Ok(()) => Some(id),
            // Another pass created it first.
            Err(_) => self
                .store
                .project_by_path(path.to_path_buf())
                .await
                .ok()
                .flatten()
                .map(|p| p.id),
        }
    }
}

/// Directories that must never become a project, because everything else lives
/// underneath them. A session there is still counted; it simply belongs to no
/// repository.
fn is_too_broad(path: &Path, home: Option<&Path>) -> bool {
    // `/`, `/tmp`, `/home` and the like.
    if path.components().count() <= 2 {
        return true;
    }
    home.is_some_and(|h| path == h)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Every `*.jsonl` under the root, sub-agent directories included.
fn list_transcripts(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(t) if t.is_file() && path.extension().is_some_and(|e| e == "jsonl") => {
                    out.push(path);
                }
                _ => {}
            }
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(unix)]
fn inode_of(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.ino()
}

#[cfg(not(unix))]
fn inode_of(_meta: &std::fs::Metadata) -> u64 {
    0
}

/// `git rev-parse --show-toplevel`, or `None` outside a repository.
fn git_toplevel(cwd: &Path) -> Option<PathBuf> {
    if !cwd.is_dir() {
        return None;
    }
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::config::Config;
    use orchestra_core::protocol::UsageQuery;
    use std::io::Write;

    struct Harness {
        _dir: tempfile::TempDir,
        root: PathBuf,
        store: Store,
        ledger: UsageLedger,
        bus: EventBus,
    }

    fn harness() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open_memory().unwrap();
        let ledger = UsageLedger::new(store.clone(), &Config::default());
        let bus = EventBus::new(store.clone());
        Harness {
            _dir: dir,
            root,
            store,
            ledger,
            bus,
        }
    }

    impl Harness {
        fn watcher(&self) -> TranscriptWatcher {
            TranscriptWatcher::new(
                self.root.clone(),
                self.store.clone(),
                self.ledger.clone(),
                self.bus.clone(),
            )
        }

        /// Write a session file the way Claude Code lays it out.
        fn session_file(&self, cwd: &str, session: Uuid) -> PathBuf {
            let slug: String = cwd
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '-' })
                .collect();
            let dir = self.root.join(slug);
            std::fs::create_dir_all(&dir).unwrap();
            dir.join(format!("{session}.jsonl"))
        }
    }

    fn assistant_line(session: Uuid, cwd: &str, msg: &str, block: u32, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"{session}","cwd":"{cwd}","apiBlockIndex":{block},"timestamp":"2026-09-19T10:00:0{}Z","version":"2.1.276","isSidechain":false,"message":{{"id":"{msg}","model":"claude-opus-5","role":"assistant","content":[{{"type":"text","text":"x"}}],"usage":{{"input_tokens":2,"output_tokens":{output},"cache_read_input_tokens":1000,"cache_creation_input_tokens":0,"output_tokens_details":{{"thinking_tokens":1}}}}}}}}"#,
            block.min(9)
        )
    }

    fn append(path: &Path, line: &str) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(f, "{line}").unwrap();
    }

    async fn totals(h: &Harness) -> orchestra_core::protocol::UsageTotals {
        h.ledger.rollup(UsageQuery::default()).await.unwrap().1
    }

    /// Every response counted, whatever it is attached to.
    async fn all_messages(h: &Harness) -> u64 {
        totals(h).await.messages
    }

    #[tokio::test]
    async fn a_response_written_as_several_blocks_is_billed_once() {
        let h = harness();
        let session = Uuid::new_v4();
        let path = h.session_file("/tmp/proj", session);
        // The real shape: partial output on the early blocks, complete on the last.
        for (block, out) in [(0, 5), (1, 5), (2, 787)] {
            append(
                &path,
                &assistant_line(session, "/tmp/proj", "msg_1", block, out),
            );
        }

        let mut w = h.watcher();
        let report = w.scan_all().await;
        assert_eq!(report.lines, 3);

        let t = totals(&h).await;
        assert_eq!(t.messages, 1, "une réponse API, une ligne de coût");
        assert_eq!(t.tokens.output, 787, "le total complet, pas le premier vu");
        assert_eq!(t.tokens.cache_read, 1000, "le cache n'est pas multiplié");
    }

    #[tokio::test]
    async fn a_second_pass_reads_only_what_was_appended() {
        let h = harness();
        let session = Uuid::new_v4();
        let path = h.session_file("/tmp/proj", session);
        append(
            &path,
            &assistant_line(session, "/tmp/proj", "msg_1", 0, 100),
        );

        let mut w = h.watcher();
        let first = w.scan_all().await;
        assert_eq!(first.lines, 1);
        assert_eq!(first.samples_new, 1);

        // Nothing new to read.
        let idle = w.scan_all().await;
        assert_eq!(idle.lines, 0);

        append(
            &path,
            &assistant_line(session, "/tmp/proj", "msg_2", 0, 200),
        );
        let second = w.scan_all().await;
        assert_eq!(second.lines, 1, "seule la ligne ajoutée est relue");
        assert_eq!(second.samples_new, 1);
        assert_eq!(totals(&h).await.messages, 2);
    }

    #[tokio::test]
    async fn a_line_split_across_two_passes_is_not_lost() {
        let h = harness();
        let session = Uuid::new_v4();
        let path = h.session_file("/tmp/proj", session);
        let line = assistant_line(session, "/tmp/proj", "msg_1", 0, 300);
        let (head, tail) = line.split_at(line.len() / 2);

        // Claude Code is mid-write: the line has no newline yet.
        std::fs::write(&path, head).unwrap();
        let mut w = h.watcher();
        w.scan_all().await;
        assert_eq!(totals(&h).await.messages, 0, "rien n'est compté à moitié");

        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "{tail}").unwrap();
        drop(f);
        w.scan_all().await;
        assert_eq!(
            totals(&h).await.messages,
            1,
            "la ligne complète est comptée"
        );
        assert_eq!(totals(&h).await.tokens.output, 300);
    }

    #[tokio::test]
    async fn the_cursor_survives_a_restart() {
        let h = harness();
        let session = Uuid::new_v4();
        let path = h.session_file("/tmp/proj", session);
        append(
            &path,
            &assistant_line(session, "/tmp/proj", "msg_1", 0, 100),
        );

        let mut w = h.watcher();
        w.scan_all().await;

        // A fresh watcher, as after a daemon restart: the offset is on disk.
        let mut w2 = h.watcher();
        let report = w2.scan_all().await;
        assert_eq!(report.lines, 0, "rien n'est relu après redémarrage");
        assert_eq!(totals(&h).await.messages, 1);
    }

    #[tokio::test]
    async fn a_replaced_file_is_read_from_the_start() {
        let h = harness();
        let session = Uuid::new_v4();
        let path = h.session_file("/tmp/proj", session);
        append(
            &path,
            &assistant_line(session, "/tmp/proj", "msg_1", 0, 100),
        );
        let mut w = h.watcher();
        w.scan_all().await;

        // Truncated and rewritten with different content.
        std::fs::write(&path, "").unwrap();
        append(&path, &assistant_line(session, "/tmp/proj", "msg_2", 0, 50));
        let report = w.scan_all().await;
        assert_eq!(report.lines, 1);
        assert_eq!(totals(&h).await.messages, 2);
    }

    #[tokio::test]
    async fn unreadable_lines_do_not_stop_the_scan() {
        let h = harness();
        let session = Uuid::new_v4();
        let path = h.session_file("/tmp/proj", session);
        append(&path, r#"{"type":"assistant","message":{"tronqué"#);
        append(
            &path,
            &assistant_line(session, "/tmp/proj", "msg_ok", 0, 42),
        );
        append(&path, r#"{"type":"attachment","content":"…"}"#);

        let mut w = h.watcher();
        let report = w.scan_all().await;
        assert_eq!(report.unreadable_lines, 1);
        assert_eq!(
            totals(&h).await.messages,
            1,
            "la bonne ligne passe quand même"
        );
    }

    #[tokio::test]
    async fn subagent_files_are_read_and_attributed_to_their_parent() {
        let h = harness();
        let session = Uuid::new_v4();
        let dir = h
            .session_file("/tmp/proj", session)
            .parent()
            .unwrap()
            .join(session.to_string())
            .join("subagents");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent-abc123.jsonl");
        let line = format!(
            r#"{{"type":"assistant","sessionId":"{session}","cwd":"/tmp/proj","agentId":"abc123","isSidechain":true,"apiBlockIndex":0,"message":{{"id":"msg_sub","model":"claude-sonnet-5","content":[],"usage":{{"input_tokens":1,"output_tokens":9}}}}}}"#
        );
        append(&path, &line);

        let mut w = h.watcher();
        w.scan_all().await;
        let t = totals(&h).await;
        assert_eq!(t.messages, 1);
        assert_eq!(t.tokens.output, 9);
    }

    #[tokio::test]
    async fn an_unknown_working_directory_becomes_a_discovered_project() {
        let h = harness();
        let session = Uuid::new_v4();
        let cwd = h._dir.path().join("mon-depot");
        std::fs::create_dir_all(&cwd).unwrap();
        let cwd_str = cwd.to_string_lossy().to_string();
        let path = h.session_file(&cwd_str, session);
        append(&path, &assistant_line(session, &cwd_str, "msg_1", 0, 10));

        let mut w = h.watcher();
        w.scan_all().await;

        let projects = h.store.list_projects().await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "mon-depot");
        assert_eq!(
            projects[0].kind,
            orchestra_core::model::ProjectKind::Discovered
        );
        // And the cost is attributed to it.
        let q = UsageQuery {
            project_id: Some(projects[0].id),
            ..Default::default()
        };
        assert_eq!(h.ledger.rollup(q).await.unwrap().1.messages, 1);
    }

    #[tokio::test]
    async fn sessions_of_a_known_project_do_not_create_a_second_one() {
        let h = harness();
        let cwd = h._dir.path().join("connu");
        std::fs::create_dir_all(&cwd).unwrap();
        let project = orchestra_core::model::Project {
            id: Uuid::new_v4(),
            name: "connu".into(),
            path: cwd.clone(),
            default_branch: "main".into(),
            zellij_tab: None,
            kind: orchestra_core::model::ProjectKind::Managed,
            created_at: orchestra_core::now(),
        };
        h.store.insert_project(project.clone()).await.unwrap();

        // A session in a sub-directory of the project still belongs to it.
        let sub = cwd.join("crates/x");
        std::fs::create_dir_all(&sub).unwrap();
        let session = Uuid::new_v4();
        let sub_str = sub.to_string_lossy().to_string();
        let path = h.session_file(&sub_str, session);
        append(&path, &assistant_line(session, &sub_str, "msg_1", 0, 10));

        let mut w = h.watcher();
        w.scan_all().await;
        assert_eq!(h.store.list_projects().await.unwrap().len(), 1);
        let q = UsageQuery {
            project_id: Some(project.id),
            ..Default::default()
        };
        assert_eq!(h.ledger.rollup(q).await.unwrap().1.messages, 1);
    }

    #[tokio::test]
    async fn an_unmanaged_session_is_announced_once() {
        let h = harness();
        let session = Uuid::new_v4();
        let path = h.session_file("/tmp/proj", session);
        for i in 0..3 {
            append(
                &path,
                &assistant_line(session, "/tmp/proj", &format!("msg_{i}"), 0, 10),
            );
        }
        let mut w = h.watcher();
        w.scan_all().await;

        let events = h
            .store
            .recent_events(orchestra_core::events::EventFilter::all(), 100)
            .await
            .unwrap();
        let announced = events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::UnmanagedSessionSeen { .. }))
            .count();
        assert_eq!(announced, 1, "une seule annonce pour une session");
        // And no per-sample noise for a session we do not drive.
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e.kind, EventKind::Usage { .. }))
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn synthetic_messages_cost_nothing() {
        let h = harness();
        let session = Uuid::new_v4();
        let path = h.session_file("/tmp/proj", session);
        let line = format!(
            r#"{{"type":"assistant","sessionId":"{session}","cwd":"/tmp/proj","message":{{"id":"msg_synth","model":"<synthetic>","content":[],"usage":{{"input_tokens":0,"output_tokens":0}}}}}}"#
        );
        append(&path, &line);
        let mut w = h.watcher();
        w.scan_all().await;
        assert_eq!(totals(&h).await.messages, 0);
    }

    #[tokio::test]
    async fn a_session_in_the_home_directory_does_not_swallow_every_repository() {
        // Regression: a single session started in $HOME created a project
        // there, and every later session matched it by prefix, so all the real
        // repositories collapsed into one row.
        let h = harness();
        let home = h._dir.path().join("faux-home");
        let repo = home.join("dev/mon-depot");
        std::fs::create_dir_all(&repo).unwrap();
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .arg("init")
            .output()
            .unwrap();

        let home_str = home.to_string_lossy().to_string();
        let repo_str = repo.to_string_lossy().to_string();

        // A session in the home directory comes first, as it did in practice.
        let s1 = Uuid::new_v4();
        let p1 = h.session_file(&home_str, s1);
        append(&p1, &assistant_line(s1, &home_str, "msg_home", 0, 10));
        // Then one inside a real repository.
        let s2 = Uuid::new_v4();
        let p2 = h.session_file(&repo_str, s2);
        append(&p2, &assistant_line(s2, &repo_str, "msg_repo", 0, 20));

        let mut w = h.watcher();
        w.scan_all().await;

        let projects = h.store.list_projects().await.unwrap();
        let repo_project = projects
            .iter()
            .find(|p| p.name == "mon-depot")
            .expect("le dépôt doit être son propre projet");

        // The point of the fix: the repository's tokens belong to the
        // repository, not to the directory that happens to contain it.
        let q = UsageQuery {
            project_id: Some(repo_project.id),
            ..Default::default()
        };
        let totals = h.ledger.rollup(q).await.unwrap().1;
        assert_eq!(totals.messages, 1);
        assert_eq!(totals.tokens.output, 20);

        // Both sessions are still counted.
        assert_eq!(all_messages(&h).await, 2);
    }

    #[tokio::test]
    async fn a_subdirectory_of_a_repository_joins_that_repository() {
        let h = harness();
        let repo = h._dir.path().join("depot");
        let sub = repo.join("crates/inner");
        std::fs::create_dir_all(&sub).unwrap();
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .arg("init")
            .output()
            .unwrap();

        let sub_str = sub.to_string_lossy().to_string();
        let session = Uuid::new_v4();
        let path = h.session_file(&sub_str, session);
        append(&path, &assistant_line(session, &sub_str, "msg_1", 0, 10));

        let mut w = h.watcher();
        w.scan_all().await;
        let projects = h.store.list_projects().await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(
            projects[0].name, "depot",
            "la racine du dépôt, pas le sous-dossier"
        );
    }

    #[tokio::test]
    async fn a_row_costs_the_sum_of_its_models() {
        // Regression: a row covering several models was priced at the average
        // of their rates, so the rows did not add up to the total.
        let h = harness();
        let session = Uuid::new_v4();
        let path = h.session_file("/tmp/proj", session);
        for (msg, model) in [("a", "claude-opus-5"), ("b", "claude-haiku-4-5")] {
            let line = format!(
                r#"{{"type":"assistant","sessionId":"{session}","cwd":"/tmp/proj","apiBlockIndex":0,"message":{{"id":"msg_{msg}","model":"{model}","content":[],"usage":{{"input_tokens":0,"output_tokens":1000000,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
            );
            append(&path, &line);
        }
        let mut w = h.watcher();
        w.scan_all().await;

        // One million output tokens each: 25 USD on opus-5, 5 on haiku-4.5.
        let (rows, totals) = h.ledger.rollup(UsageQuery::default()).await.unwrap();
        assert_eq!(rows.len(), 1, "un seul projet, donc une ligne");
        let row_cost = rows[0].cost_usd.unwrap();
        assert!(
            (row_cost - 30.0).abs() < 1e-6,
            "la ligne doit coûter la somme de ses modèles, obtenu {row_cost}"
        );
        assert!(
            (row_cost - totals.cost_usd.unwrap()).abs() < 1e-6,
            "les lignes doivent totaliser le total"
        );
    }

    #[test]
    fn broad_directories_never_become_projects() {
        let home = Path::new("/home/moi");
        assert!(is_too_broad(Path::new("/"), Some(home)));
        assert!(is_too_broad(Path::new("/tmp"), Some(home)));
        assert!(is_too_broad(Path::new("/home"), Some(home)));
        assert!(
            is_too_broad(home, Some(home)),
            "le dossier personnel non plus"
        );
        assert!(!is_too_broad(
            Path::new("/home/moi/projets/app"),
            Some(home)
        ));
        assert!(!is_too_broad(Path::new("/srv/app"), Some(home)));
        // Without a home directory the rule still holds for the shallow ones.
        assert!(is_too_broad(Path::new("/tmp"), None));
        assert!(!is_too_broad(Path::new("/home/moi"), None));
    }

    #[tokio::test]
    async fn a_session_inside_a_worktree_belongs_to_its_ticket() {
        // Regression: a worktree is a git repository too, so it became its own
        // project and every ticket appeared as a separate line in the costs.
        let h = harness();
        let project = orchestra_core::model::Project {
            id: Uuid::new_v4(),
            name: "depot".into(),
            path: h._dir.path().join("depot"),
            default_branch: "main".into(),
            zellij_tab: None,
            kind: orchestra_core::model::ProjectKind::Managed,
            created_at: orchestra_core::now(),
        };
        h.store.insert_project(project.clone()).await.unwrap();

        let worktree = h._dir.path().join("worktrees/depot/1-cache");
        std::fs::create_dir_all(&worktree).unwrap();
        let now = orchestra_core::now();
        let ticket = orchestra_core::model::Ticket {
            id: Uuid::new_v4(),
            project_id: project.id,
            number: 1,
            title: "cache".into(),
            brief: "b".into(),
            status: orchestra_core::model::TicketStatus::Running,
            branch: Some("orch/1-cache".into()),
            worktree_path: Some(worktree.clone()),
            proposal: None,
            team: None,
            created_at: now,
            updated_at: now,
        };
        h.store.insert_ticket(ticket.clone()).await.unwrap();

        let cwd = worktree.to_string_lossy().to_string();
        let session = Uuid::new_v4();
        let path = h.session_file(&cwd, session);
        append(&path, &assistant_line(session, &cwd, "msg_1", 0, 42));

        let mut w = h.watcher();
        w.scan_all().await;

        assert_eq!(
            h.store.list_projects().await.unwrap().len(),
            1,
            "le worktree ne devient pas un projet"
        );
        let q = UsageQuery {
            ticket_id: Some(ticket.id),
            ..Default::default()
        };
        assert_eq!(h.ledger.rollup(q).await.unwrap().1.messages, 1);
    }

    #[tokio::test]
    async fn a_missing_root_is_not_an_error() {
        let h = harness();
        std::fs::remove_dir_all(&h.root).unwrap();
        let mut w = h.watcher();
        let report = w.scan_all().await;
        assert_eq!(report, ScanReport::default());
    }
}
