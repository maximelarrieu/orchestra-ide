//! Unix socket server: one task per connection, JSON per line.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use orchestra_core::events::{Event, EventFilter};
use orchestra_core::protocol::{
    ApiError, Command, Frame, Reply, Request, Response, PROTOCOL_VERSION,
};
use time::OffsetDateTime;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;
use tokio_util::codec::{Framed, LinesCodec, LinesCodecError};

use crate::daemon::DaemonHandle;

/// A single line may carry a whole event backlog entry; 4 MiB is generous and
/// still bounds a runaway payload.
const MAX_LINE: usize = 4 * 1024 * 1024;

/// Holds the socket file and removes it on drop.
pub struct SocketGuard {
    path: PathBuf,
    listener: UnixListener,
    _lock: LockFile,
}

impl SocketGuard {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// An flock'd file that guarantees a single daemon per socket path.
pub struct LockFile {
    file: std::fs::File,
}

impl LockFile {
    fn acquire(path: &Path) -> Result<Self> {
        use nix::fcntl::{Flock, FlockArg};
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .with_context(|| format!("ouverture du verrou {}", path.display()))?;
        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(flock) => {
                // Keep the descriptor alive; dropping it releases the lock.
                let file = unsafe {
                    use std::os::fd::{AsRawFd, FromRawFd};
                    let fd = flock.as_raw_fd();
                    std::mem::forget(flock);
                    std::fs::File::from_raw_fd(fd)
                };
                Ok(LockFile { file })
            }
            Err(_) => anyhow::bail!(
                "un autre daemon Orchestra tourne déjà (verrou {})",
                path.display()
            ),
        }
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        use std::io::Write;
        let _ = self.file.flush();
    }
}

/// Bind the socket, taking the lock and clearing a stale file first.
pub fn bind(socket: &Path, lock: &Path) -> Result<SocketGuard> {
    let lock = LockFile::acquire(lock)?;
    // We hold the lock, so any socket file left behind is stale.
    if socket.exists() {
        std::fs::remove_file(socket)
            .with_context(|| format!("suppression du socket périmé {}", socket.display()))?;
    }
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener =
        UnixListener::bind(socket).with_context(|| format!("écoute sur {}", socket.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(SocketGuard {
        path: socket.to_path_buf(),
        listener,
        _lock: lock,
    })
}

/// Accept connections until `shutdown` resolves.
pub async fn serve(
    guard: SocketGuard,
    handle: DaemonHandle,
    started_at: OffsetDateTime,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<()> {
    tokio::pin!(shutdown);
    tracing::info!(socket = %guard.path().display(), "daemon à l'écoute");
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("arrêt demandé, fermeture du socket");
                return Ok(());
            }
            accepted = guard.listener.accept() => {
                match accepted {
                    Ok((stream, _addr)) => {
                        let handle = handle.clone();
                        tokio::spawn(async move {
                            if let Err(e) = connection(stream, handle, started_at).await {
                                tracing::debug!("connexion terminée : {e}");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!("accept a échoué : {e}");
                    }
                }
            }
        }
    }
}

/// Serve one client: read requests, write responses, and stream events once
/// the client subscribes.
async fn connection(
    stream: UnixStream,
    handle: DaemonHandle,
    started_at: OffsetDateTime,
) -> Result<()> {
    let mut framed = Framed::new(stream, LinesCodec::new_with_max_length(MAX_LINE));
    send(
        &mut framed,
        Frame::Hello {
            version: orchestra_core::VERSION.to_string(),
            protocol: PROTOCOL_VERSION,
            daemon_started: started_at,
        },
    )
    .await?;

    let mut sub: Option<Subscription> = None;

    loop {
        // Without a subscription there is nothing to forward, so just read.
        let next_event = async {
            match sub.as_mut() {
                Some(s) => s.rx.recv().await,
                None => std::future::pending().await,
            }
        };

        tokio::select! {
            incoming = framed.next() => {
                let Some(line) = incoming else { return Ok(()) };
                let line = match line {
                    Ok(l) => l,
                    Err(LinesCodecError::MaxLineLengthExceeded) => {
                        tracing::warn!("ligne trop longue, connexion fermée");
                        return Ok(());
                    }
                    Err(e) => return Err(e.into()),
                };
                if line.trim().is_empty() {
                    continue;
                }
                let req: Request = match serde_json::from_str(&line) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("requête illisible : {e}");
                        send(&mut framed, Frame::Response(Response {
                            id: 0,
                            result: Err(ApiError::invalid(format!("JSON invalide : {e}"))).into(),
                        })).await?;
                        continue;
                    }
                };

                match req.cmd {
                    Command::Subscribe { filter, since_seq, backlog } => {
                        let reply = start_subscription(
                            &handle, &mut framed, &mut sub, filter, since_seq, backlog,
                        ).await;
                        send(&mut framed, Frame::Response(Response { id: req.id, result: reply.into() })).await?;
                    }
                    Command::Unsubscribe => {
                        sub = None;
                        send(&mut framed, Frame::Response(Response {
                            id: req.id,
                            result: Ok(Reply::Ack).into(),
                        })).await?;
                    }
                    cmd => {
                        let result = handle.call(cmd).await;
                        send(&mut framed, Frame::Response(Response { id: req.id, result: result.into() })).await?;
                    }
                }
            }
            event = next_event => {
                match event {
                    Ok(ev) => {
                        let Some(s) = sub.as_mut() else { continue };
                        if ev.seq <= s.last_sent || !s.filter.matches(&ev) {
                            continue;
                        }
                        s.last_sent = ev.seq;
                        send(&mut framed, Frame::Event(Box::new((*ev).clone()))).await?;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        // The client fell behind: replay what it missed from the store.
                        tracing::warn!("client en retard de {n} événements, resynchronisation");
                        if let Some(s) = sub.as_mut() {
                            let missed = handle
                                .bus()
                                .store()
                                .events_since(s.last_sent, s.filter.clone(), 1000)
                                .await
                                .unwrap_or_default();
                            for ev in missed {
                                s.last_sent = ev.seq;
                                send(&mut framed, Frame::Event(Box::new(ev))).await?;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        sub = None;
                    }
                }
            }
        }
    }
}

struct Subscription {
    rx: broadcast::Receiver<Arc<Event>>,
    filter: EventFilter,
    last_sent: i64,
}

async fn start_subscription(
    handle: &DaemonHandle,
    framed: &mut Framed<UnixStream, LinesCodec>,
    slot: &mut Option<Subscription>,
    filter: EventFilter,
    since_seq: Option<i64>,
    backlog: u32,
) -> Result<Reply, ApiError> {
    // Subscribe before reading the backlog so nothing published in between is
    // lost; duplicates are filtered by `last_sent`.
    let rx = handle.bus().subscribe();
    let store = handle.bus().store();
    let current_seq = store
        .last_seq()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let history = match since_seq {
        Some(seq) => {
            store
                .events_since(seq, filter.clone(), backlog.clamp(1, 5000))
                .await
        }
        None if backlog > 0 => store.recent_events(filter.clone(), backlog.min(5000)).await,
        None => Ok(Vec::new()),
    }
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut last_sent = since_seq.unwrap_or(0);
    for ev in history {
        last_sent = last_sent.max(ev.seq);
        send(framed, Frame::Event(Box::new(ev)))
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }

    *slot = Some(Subscription {
        rx,
        filter,
        last_sent,
    });
    Ok(Reply::Subscribed { current_seq })
}

async fn send(framed: &mut Framed<UnixStream, LinesCodec>, frame: Frame) -> Result<()> {
    let line = serde_json::to_string(&frame)?;
    framed.send(line).await?;
    Ok(())
}
