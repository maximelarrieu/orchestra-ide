//! Terminal setup and the event loop that drives [`crate::App`].

use std::io::{self, Stdout};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{Event as TermEvent, EventStream, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use orchestra_core::events::EventFilter;
use orchestra_core::protocol::{Command, Reply};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::app::{App, Msg};
use crate::client::{Client, ClientHandle};
use crate::keymap;

/// Minimum time between redraws, so a burst of events cannot spin the loop.
const FRAME: Duration = Duration::from_millis(60);

/// Restores the terminal even on panic or error.
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("passage en mode brut")?;
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen).context("écran alternatif")?;
        let terminal = Terminal::new(CrosstermBackend::new(out)).context("init du terminal")?;
        Ok(TerminalGuard { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

/// Connect (starting the daemon if needed), then run until the user quits.
pub async fn run(socket: &Path) -> Result<()> {
    let client = Client::connect_or_spawn(socket).await?;
    let version = client.daemon_version.clone();
    // A small backlog so the activity strip is not empty on open.
    let (mut events, mut handle) = client.subscribe(EventFilter::board(), 50).await?;

    // Replies arrive out of band and are folded back into the app as messages.
    let (reply_tx, mut replies) = mpsc::channel::<Result<Reply>>(64);

    let mut app = App::new();
    app.update(Msg::Reconnected(version));
    app.refresh();
    dispatch(&handle, &reply_tx, app.take_outbox());

    let mut guard = TerminalGuard::enter()?;
    // An `Option` so it can be dropped while an editor owns the terminal:
    // a live stream would steal the keystrokes typed into it.
    let mut keys = Some(EventStream::new());
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    let mut dirty = true;
    let mut last_draw = std::time::Instant::now() - FRAME;

    loop {
        if dirty && last_draw.elapsed() >= FRAME {
            guard.terminal.draw(|f| crate::screens::render(&app, f))?;
            dirty = false;
            last_draw = std::time::Instant::now();
        }

        tokio::select! {
            key = async { keys.as_mut().expect("rétabli après l'éditeur").next().await } => {
                match key {
                    Some(Ok(TermEvent::Key(k))) if k.kind == KeyEventKind::Press => {
                        let action = if app.is_typing() {
                            keymap::map_input(k)
                        } else {
                            keymap::map(k)
                        };
                        if let Some(a) = action {
                            let cmds = app.update(Msg::Key(a));
                            dispatch(&handle, &reply_tx, cmds);
                            dirty = true;
                        }
                    }
                    Some(Ok(TermEvent::Resize(..))) => dirty = true,
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(e).context("lecture du clavier"),
                    None => break,
                }
            }
            event = events.recv() => {
                match event {
                    Some(ev) => {
                        let cmds = app.update(Msg::Event(Box::new(ev)));
                        dispatch(&handle, &reply_tx, cmds);
                        dirty = true;
                    }
                    None => {
                        app.update(Msg::Disconnected);
                        dirty = true;
                    }
                }
            }
            reply = replies.recv() => {
                match reply {
                    Some(Ok(r)) => {
                        let cmds = app.update(Msg::Reply(Box::new(r)));
                        dispatch(&handle, &reply_tx, cmds);
                        dirty = true;
                    }
                    Some(Err(e)) => {
                        app.status = format!("erreur : {e}");
                        dirty = true;
                    }
                    None => {}
                }
            }
            _ = ticker.tick() => {
                // The tick is what polls: the board's list, the open ticket,
                // the cost view, and every counter that has to advance on its
                // own. Redrawing without it only repaints stale state — which
                // is exactly what it did, so nothing moved until an event
                // happened to arrive.
                let cmds = app.update(Msg::Tick);
                // The tick still turns the spinners when the socket is down;
                // its queries would only fill the status line with errors.
                if handle.is_connected() {
                    dispatch(&handle, &reply_tx, cmds);
                } else {
                    if app.connected {
                        app.update(Msg::Disconnected);
                    }
                    // A daemon that came back gets picked up on its own: the
                    // alternative is a screen that stays frozen until the user
                    // quits and starts again.
                    if let Some((new_events, new_handle, version)) = reconnect(socket).await {
                        events = new_events;
                        handle = new_handle;
                        app.update(Msg::Reconnected(version));
                        app.refresh();
                        dispatch(&handle, &reply_tx, app.take_outbox());
                    }
                }
                dirty = true;
            }
        }

        if let Some(path) = app.take_edit_request() {
            drop(keys.take());
            let outcome = edit(&mut guard, &path).await;
            keys = Some(EventStream::new());
            let cmds = app.edited(outcome);
            dispatch(&handle, &reply_tx, cmds);
            dirty = true;
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

/// Hand the terminal to `$VISUAL` / `$EDITOR` (then `vi`) on one file, and
/// take it back whatever happens. The daemon keeps running meanwhile; its
/// events are simply read once the screen is back.
async fn edit(guard: &mut TerminalGuard, path: &Path) -> std::result::Result<(), String> {
    let editor = std::env::var("VISUAL")
        .ok()
        .or_else(|| std::env::var("EDITOR").ok())
        .filter(|e| !e.trim().is_empty())
        .unwrap_or_else(|| "vi".into());
    // `code -w`, `emacsclient -t`: a program and its flags, no shell.
    let mut words = editor.split_whitespace();
    let program = words.next().unwrap_or("vi").to_string();
    let args: Vec<String> = words.map(str::to_string).collect();

    let _ = disable_raw_mode();
    let _ = execute!(guard.terminal.backend_mut(), LeaveAlternateScreen);
    let _ = guard.terminal.show_cursor();
    let status = tokio::process::Command::new(&program)
        .args(&args)
        .arg(path)
        .status()
        .await;
    let _ = enable_raw_mode();
    let _ = execute!(guard.terminal.backend_mut(), EnterAlternateScreen);
    let _ = guard.terminal.clear();

    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("{program} a rendu {s}")),
        Err(e) => Err(format!("{program} introuvable : {e}")),
    }
}

/// Reconnect to a daemon that came back, without starting one.
///
/// Deliberately not `connect_or_spawn`: a daemon the user stopped must stay
/// stopped, and a restarted one is found at the next tick anyway.
async fn reconnect(
    socket: &Path,
) -> Option<(
    mpsc::Receiver<orchestra_core::events::Event>,
    ClientHandle,
    String,
)> {
    let client = Client::connect(socket).await.ok()?;
    let version = client.daemon_version.clone();
    let (events, handle) = client.subscribe(EventFilter::board(), 50).await.ok()?;
    Some((events, handle, version))
}

/// Fire off commands without blocking the loop; each reply comes back on
/// `reply_tx`.
fn dispatch(handle: &ClientHandle, reply_tx: &mpsc::Sender<Result<Reply>>, cmds: Vec<Command>) {
    for cmd in cmds {
        let handle = handle.clone();
        let tx = reply_tx.clone();
        tokio::spawn(async move {
            let result = handle.call(cmd).await;
            let _ = tx.send(result).await;
        });
    }
}
