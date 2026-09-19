//! Socket client used by the TUI and the CLI.
//!
//! Commands and the event stream share one connection, so a client sees its
//! own writes reflected in the stream in order.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use futures::{SinkExt, StreamExt};
use orchestra_core::events::{Event, EventFilter};
use orchestra_core::protocol::{
    ApiError, Command, Frame, Reply, Request, RequestIds, PROTOCOL_VERSION,
};
use tokio::net::UnixStream;
use tokio_util::codec::{Framed, LinesCodec};

const MAX_LINE: usize = 4 * 1024 * 1024;

pub struct Client {
    framed: Framed<UnixStream, LinesCodec>,
    ids: RequestIds,
    pub daemon_version: String,
    socket: PathBuf,
}

/// What a client gets back while waiting for a response: either the response
/// itself, or an event that arrived first.
enum Incoming {
    Response(u64, Result<Reply, ApiError>),
    Event(Box<Event>),
}

impl Client {
    /// Connect, or start the daemon and retry for a few seconds.
    pub async fn connect_or_spawn(socket: &Path) -> Result<Self> {
        match Self::connect(socket).await {
            Ok(c) => Ok(c),
            Err(_) => {
                spawn_daemon()?;
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                loop {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    match Self::connect(socket).await {
                        Ok(c) => return Ok(c),
                        Err(e) if std::time::Instant::now() >= deadline => {
                            return Err(e).context("le daemon n'a pas démarré à temps")
                        }
                        Err(_) => continue,
                    }
                }
            }
        }
    }

    pub async fn connect(socket: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket)
            .await
            .with_context(|| format!("connexion à {}", socket.display()))?;
        let mut framed = Framed::new(stream, LinesCodec::new_with_max_length(MAX_LINE));
        let hello = framed
            .next()
            .await
            .ok_or_else(|| anyhow!("le daemon a fermé la connexion immédiatement"))??;
        let daemon_version = match serde_json::from_str::<Frame>(&hello)? {
            Frame::Hello {
                version, protocol, ..
            } => {
                if protocol != PROTOCOL_VERSION {
                    bail!(
                        "version de protocole incompatible : daemon {protocol}, client {PROTOCOL_VERSION}. \
                         Redémarre le daemon (orchestra daemon)."
                    );
                }
                version
            }
            other => bail!("première trame inattendue : {other:?}"),
        };
        Ok(Client {
            framed,
            ids: RequestIds::default(),
            daemon_version,
            socket: socket.to_path_buf(),
        })
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Send a command and wait for its response, buffering nothing: events that
    /// arrive meanwhile are dropped. Use [`Client::split`] for the TUI.
    pub async fn call(&mut self, cmd: Command) -> Result<Reply> {
        let id = self.ids.next_id();
        self.send(Request { id, cmd }).await?;
        loop {
            match self.recv().await? {
                Incoming::Response(rid, result) if rid == id => {
                    return result.map_err(|e| anyhow!("{e}"))
                }
                Incoming::Response(..) | Incoming::Event(_) => continue,
            }
        }
    }

    /// Subscribe and hand the connection over to a task that forwards events.
    pub async fn subscribe(
        mut self,
        filter: EventFilter,
        backlog: u32,
    ) -> Result<(tokio::sync::mpsc::Receiver<Event>, ClientHandle)> {
        let id = self.ids.next_id();
        self.send(Request {
            id,
            cmd: Command::Subscribe {
                filter,
                since_seq: None,
                backlog,
            },
        })
        .await?;

        let (event_tx, event_rx) = tokio::sync::mpsc::channel(1024);
        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<(
            Command,
            tokio::sync::oneshot::Sender<Result<Reply>>,
        )>(32);

        tokio::spawn(async move {
            let mut pending: Vec<(u64, tokio::sync::oneshot::Sender<Result<Reply>>)> = Vec::new();
            loop {
                tokio::select! {
                    outgoing = cmd_rx.recv() => {
                        let Some((cmd, reply)) = outgoing else { return };
                        let id = self.ids.next_id();
                        if let Err(e) = self.send(Request { id, cmd }).await {
                            let _ = reply.send(Err(e));
                            return;
                        }
                        pending.push((id, reply));
                    }
                    incoming = self.recv() => {
                        match incoming {
                            Ok(Incoming::Event(ev)) => {
                                if event_tx.send(*ev).await.is_err() {
                                    return;
                                }
                            }
                            Ok(Incoming::Response(rid, result)) => {
                                if let Some(pos) = pending.iter().position(|(id, _)| *id == rid) {
                                    let (_, tx) = pending.remove(pos);
                                    let _ = tx.send(result.map_err(|e| anyhow!("{e}")));
                                }
                            }
                            Err(e) => {
                                tracing::debug!("flux daemon interrompu : {e}");
                                return;
                            }
                        }
                    }
                }
            }
        });

        Ok((event_rx, ClientHandle { tx: cmd_tx }))
    }

    async fn send(&mut self, req: Request) -> Result<()> {
        let line = serde_json::to_string(&req)?;
        self.framed.send(line).await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<Incoming> {
        loop {
            let line = self
                .framed
                .next()
                .await
                .ok_or_else(|| anyhow!("le daemon a fermé la connexion"))??;
            if line.trim().is_empty() {
                continue;
            }
            return match serde_json::from_str::<Frame>(&line)? {
                Frame::Response(r) => Ok(Incoming::Response(r.id, r.result.into_result())),
                Frame::Event(e) => Ok(Incoming::Event(e)),
                Frame::Hello { .. } => continue,
            };
        }
    }
}

/// Command side of a subscribed connection.
#[derive(Clone)]
pub struct ClientHandle {
    tx: tokio::sync::mpsc::Sender<(Command, tokio::sync::oneshot::Sender<Result<Reply>>)>,
}

impl ClientHandle {
    pub async fn call(&self, cmd: Command) -> Result<Reply> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send((cmd, tx))
            .await
            .map_err(|_| anyhow!("connexion au daemon perdue"))?;
        rx.await.map_err(|_| anyhow!("réponse perdue"))?
    }

    pub fn is_connected(&self) -> bool {
        !self.tx.is_closed()
    }
}

/// Start `orchestra daemon` detached from this process.
fn spawn_daemon() -> Result<()> {
    let exe = std::env::current_exe().context("chemin de l'exécutable introuvable")?;
    std::process::Command::new(exe)
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("démarrage du daemon")?;
    Ok(())
}
