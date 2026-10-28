//! Orchestra daemon.
//!
//! Owns the SQLite store, the event bus and (from phase 3) the agent
//! supervisor. Clients talk to it over a Unix socket; see
//! `orchestra-core::protocol`.

pub mod bus;
pub mod checks;
pub mod daemon;
pub mod epic_flow;
pub mod github;
pub mod hooks;
pub mod account;
pub mod attention;
pub mod integration;
pub mod init;
pub mod ledger;
pub mod monitor;
pub mod notify;
pub mod orchestrator;
pub mod repo_summary;
pub mod roles;
pub mod sandbox;
pub mod rules;
pub mod server;
pub mod store;
pub mod supervisor;
pub mod watcher;
pub mod worker;
pub mod worktree;
pub mod zellij;

use std::path::Path;

use anyhow::{Context, Result};
use orchestra_core::config::{Config, ResolvedPaths};
use orchestra_core::events::EventKind;
use tokio_util::sync::CancellationToken;

pub use bus::EventBus;
pub use daemon::{Daemon, DaemonHandle};
pub use ledger::UsageLedger;
pub use store::Store;
pub use watcher::TranscriptWatcher;

/// Boot everything and serve until `shutdown` resolves.
pub async fn run(cfg: Config, shutdown: impl std::future::Future<Output = ()>) -> Result<()> {
    cfg.validate().context("configuration invalide")?;
    let paths = ResolvedPaths::from_config(&cfg);
    prepare_dirs(&paths)?;

    let store = Store::open(&paths.db_file)?;
    let notify_attention = cfg.notify.attention;
    let epic_auto_plan = cfg.epic.auto_plan;
    let monitors = cfg.monitors.clone();
    let notify_store = store.clone();
    let roots = transcript_roots(&cfg, &paths, &store).await;
    let daemon = Daemon::new(cfg, store.clone());
    let handle = daemon.handle();
    let bus = daemon.bus().clone();
    let ledger = daemon.ledger().clone();
    let started_at = orchestra_core::now();

    // Bind before spawning the core so a second instance fails fast.
    let guard = server::bind(&paths.socket, &paths.lock_file)?;

    if let Err(e) = daemon.recover_on_boot().await {
        tracing::warn!("reprise des agents orphelins impossible : {e:#}");
    }
    let supervisor = daemon.supervisor().clone();
    let core = tokio::spawn(daemon.run());
    bus.publish_kind(EventKind::DaemonStarted {
        version: orchestra_core::VERSION.to_string(),
    })
    .await?;

    // Account for every Claude Code session on the machine, ours or not, under
    // every account a project uses.
    let cancel = CancellationToken::new();
    let watchers: Vec<_> = roots
        .into_iter()
        .map(|root| {
            let watcher = TranscriptWatcher::new(root, store.clone(), ledger.clone(), bus.clone());
            let cancel = cancel.clone();
            async move {
                if let Err(e) = watcher.run(cancel).await {
                    tracing::error!("surveillance des transcripts arrêtée : {e:#}");
                }
            }
        })
        .collect();
    let watching = tokio::spawn(futures::future::join_all(watchers));

    // Pull requests are answered on GitHub, not here: something has to notice.
    let pull_requests = tokio::spawn({
        let supervisor = supervisor.clone();
        let cancel = cancel.clone();
        async move { supervisor.watch_pull_requests(cancel).await }
    });

    // An accepted epic moves on when one of its tickets is merged.
    let epics = tokio::spawn(crate::epic_flow::advance_epics(
        notify_store.clone(),
        bus.clone(),
        handle.clone(),
        epic_auto_plan,
        cancel.clone(),
    ));

    // The thresholds of `[[monitors]]`; nothing runs when there are none.
    let monitoring = tokio::spawn(crate::monitor::watch(
        monitors,
        notify_store.clone(),
        bus.clone(),
        ledger.clone(),
        supervisor.clone(),
        cancel.clone(),
    ));

    // Off unless asked: a desktop notification reaches outside the tool.
    let notifying = notify_attention.then(|| {
        tokio::spawn(crate::attention::notify_on_attention(
            notify_store,
            bus.clone(),
            supervisor.clone(),
            cancel.clone(),
        ))
    });

    let result = server::serve(guard, handle, started_at, shutdown).await;
    cancel.cancel();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), watching).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), pull_requests).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), epics).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), monitoring).await;
    if let Some(task) = notifying {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), task).await;
    }
    // Dropping the last handle ends the core loop.
    drop(bus);
    core.abort();
    result
}

/// The transcript roots to tail: one per account the known projects use.
///
/// A `daemon.transcripts_dir` set by hand is taken as the whole answer. The
/// accounts are read at boot: a project moved to another account is followed
/// from the next start.
async fn transcript_roots(cfg: &Config, paths: &ResolvedPaths, store: &Store) -> Vec<std::path::PathBuf> {
    let mut roots = std::collections::BTreeSet::from([paths.transcripts_dir.clone()]);
    if cfg.daemon.transcripts_dir.is_some() {
        return roots.into_iter().collect();
    }
    let projects = store.list_projects().await.unwrap_or_else(|e| {
        tracing::warn!("projets illisibles, seuls les transcripts par défaut sont suivis : {e:#}");
        Vec::new()
    });
    for project in projects {
        let root = match account::claude_config_dir(&cfg.daemon, &project.path).await {
            Some(dir) => dir.join("projects"),
            None => orchestra_core::config::Paths::transcripts_dir(),
        };
        roots.insert(root);
    }
    roots.into_iter().collect()
}

fn prepare_dirs(paths: &ResolvedPaths) -> Result<()> {
    for dir in [
        &paths.data_dir,
        &paths.state_dir,
        &paths.cache_dir,
        &paths.worktrees_dir,
    ] {
        create_private_dir(dir)?;
    }
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).with_context(|| format!("création de {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}
