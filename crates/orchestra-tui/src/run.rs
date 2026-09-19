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
    let (mut events, handle) = client.subscribe(EventFilter::board(), 50).await?;

    // Replies arrive out of band and are folded back into the app as messages.
    let (reply_tx, mut replies) = mpsc::channel::<Result<Reply>>(64);

    let mut app = App::new();
    app.update(Msg::Reconnected(version));
    app.refresh();
    dispatch(&handle, &reply_tx, app.take_outbox());

    let mut guard = TerminalGuard::enter()?;
    let mut keys = EventStream::new();
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
            key = keys.next() => {
                match key {
                    Some(Ok(TermEvent::Key(k))) if k.kind == KeyEventKind::Press => {
                        let action = if app.palette.is_some() {
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
                if !handle.is_connected() && app.connected {
                    app.update(Msg::Disconnected);
                }
                dirty = true;
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
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
