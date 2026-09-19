//! Orchestra daemon.
//!
//! Owns the SQLite store, the event bus and (from phase 3) the agent
//! supervisor. Clients talk to it over a Unix socket; see
//! `orchestra-core::protocol`.

pub mod bus;
pub mod daemon;
pub mod hooks;
pub mod init;
pub mod ledger;
pub mod orchestrator;
pub mod repo_summary;
pub mod server;
pub mod store;
pub mod supervisor;
pub mod watcher;
pub mod worker;
pub mod worktree;

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
    let core = tokio::spawn(daemon.run());
    bus.publish_kind(EventKind::DaemonStarted {
        version: orchestra_core::VERSION.to_string(),
    })
    .await?;

    // Account for every Claude Code session on the machine, ours or not.
    let cancel = CancellationToken::new();
    let watcher = TranscriptWatcher::new(paths.transcripts_dir.clone(), store, ledger, bus.clone());
    let watching = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            if let Err(e) = watcher.run(cancel).await {
                tracing::error!("surveillance des transcripts arrêtée : {e:#}");
            }
        }
    });

    let result = server::serve(guard, handle, started_at, shutdown).await;
    cancel.cancel();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), watching).await;
    // Dropping the last handle ends the core loop.
    drop(bus);
    core.abort();
    result
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
