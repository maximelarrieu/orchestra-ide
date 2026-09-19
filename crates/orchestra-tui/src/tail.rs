//! `orchestra tail <agent>`: one agent, full screen.
//!
//! This is what runs in a zellij pane next to the dashboard. It shows the same
//! live log as the Agent screen and can send the agent a message, but nothing
//! else: the pane is a window onto one agent, not a second dashboard.

use std::io::{self, Stdout};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{Event as TermEvent, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use orchestra_core::events::EventFilter;
use orchestra_core::model::AgentId;
use orchestra_core::pricing::fmt_tokens;
use orchestra_core::protocol::{Command, Reply};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Frame, Terminal};

use crate::client::Client;
use crate::widgets::LiveLog;

const FRAME: Duration = Duration::from_millis(60);

struct Guard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Guard {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("passage en mode brut")?;
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen).context("écran alternatif")?;
        Ok(Guard {
            terminal: Terminal::new(CrosstermBackend::new(out)).context("init du terminal")?,
        })
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

/// State of the pane: a log, and possibly a message being typed.
#[derive(Default)]
struct Tail {
    log: LiveLog,
    steer: Option<String>,
    steer_hard: bool,
    title: String,
    status: String,
    quit: bool,
}

/// Follow one agent until the user leaves.
pub async fn tail(socket: &Path, agent_id: AgentId) -> Result<()> {
    let client = Client::connect_or_spawn(socket).await?;
    let (mut events, handle) = client
        .subscribe(EventFilter::for_agent(agent_id), 1000)
        .await?;

    let mut state = Tail {
        title: format!("agent {}", &agent_id.to_string()[..8]),
        ..Default::default()
    };

    let mut guard = Guard::enter()?;
    let mut keys = EventStream::new();
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    let mut dirty = true;
    let mut last_draw = std::time::Instant::now() - FRAME;

    loop {
        if dirty && last_draw.elapsed() >= FRAME {
            guard.terminal.draw(|f| render(&state, f))?;
            dirty = false;
            last_draw = std::time::Instant::now();
        }

        tokio::select! {
            key = keys.next() => {
                match key {
                    Some(Ok(TermEvent::Key(k))) if k.kind == KeyEventKind::Press => {
                        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                        if let Some(buffer) = state.steer.as_mut() {
                            match k.code {
                                KeyCode::Char('s') if ctrl => {
                                    let text = state.steer.take().unwrap_or_default();
                                    send(&handle, agent_id, text, state.steer_hard, &mut state).await;
                                }
                                KeyCode::Enter => {
                                    let text = state.steer.take().unwrap_or_default();
                                    send(&handle, agent_id, text, state.steer_hard, &mut state).await;
                                }
                                KeyCode::Esc => state.steer = None,
                                KeyCode::Backspace => { buffer.pop(); }
                                KeyCode::Char(c) if !ctrl => buffer.push(c),
                                _ => {}
                            }
                        } else {
                            match k.code {
                                KeyCode::Char('q') | KeyCode::Esc => state.quit = true,
                                KeyCode::Char('c') if ctrl => state.quit = true,
                                KeyCode::Char('s') => {
                                    state.steer = Some(String::new());
                                    state.steer_hard = false;
                                }
                                KeyCode::Char('S') => {
                                    state.steer = Some(String::new());
                                    state.steer_hard = true;
                                }
                                KeyCode::Char('x') => {
                                    if handle.call(Command::CancelAgent { agent_id }).await.is_ok() {
                                        state.status = "arrêt demandé".into();
                                    }
                                }
                                KeyCode::Char('j') | KeyCode::Down => state.log.scroll_down(1),
                                KeyCode::Char('k') | KeyCode::Up => state.log.scroll_up(1, 20),
                                KeyCode::Char('G') => state.log.follow(),
                                _ => {}
                            }
                        }
                        dirty = true;
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
                        state.log.push_event(&ev);
                        dirty = true;
                    }
                    None => {
                        state.status = "daemon déconnecté".into();
                        dirty = true;
                    }
                }
            }
            _ = ticker.tick() => dirty = true,
        }

        if state.quit {
            break;
        }
    }
    Ok(())
}

async fn send(
    handle: &crate::client::ClientHandle,
    agent_id: AgentId,
    text: String,
    hard: bool,
    state: &mut Tail,
) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    match handle
        .call(Command::SteerAgent {
            agent_id,
            text,
            hard,
        })
        .await
    {
        Ok(Reply::Ack) | Ok(_) => {
            state.status = if hard {
                "redirection envoyée".into()
            } else {
                "consigne envoyée".into()
            }
        }
        Err(e) => state.status = format!("refusé : {e}"),
    }
}

fn render(state: &Tail, frame: &mut Frame<'_>) {
    let area = frame.area();
    let input = if state.steer.is_some() { 3 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(input),
            Constraint::Length(1),
        ])
        .split(area);

    let height = chunks[0].height.saturating_sub(2) as usize;
    let lines: Vec<Line> = if state.log.is_empty() {
        vec![Line::from(Span::styled(
            "en attente de l'agent…",
            Style::default().add_modifier(Modifier::DIM),
        ))]
    } else {
        state
            .log
            .window(height)
            .into_iter()
            .map(|line| {
                let (marker, marker_style, text_style) =
                    crate::screens::agent::line_styles(line.kind);
                Line::from(vec![
                    Span::styled(
                        format!("{:>8} ", line.stamp),
                        Style::default().add_modifier(Modifier::DIM),
                    ),
                    Span::styled(marker.to_string(), marker_style),
                    Span::styled(line.text.clone(), text_style),
                ])
            })
            .collect()
    };

    let title = format!(
        " {} · {} tokens{} ",
        state.title,
        fmt_tokens(state.log.tokens.total()),
        if state.log.finished {
            " · terminé"
        } else {
            ""
        }
    );
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title)),
        chunks[0],
    );

    if let Some(buffer) = &state.steer {
        let label = if state.steer_hard {
            " Rediriger (Entrée envoyer, Échap annuler) "
        } else {
            " Consigne (Entrée envoyer, Échap annuler) "
        };
        frame.render_widget(
            Paragraph::new(format!("{buffer}▏"))
                .block(Block::default().borders(Borders::ALL).title(label)),
            chunks[1],
        );
    }

    let keys = if state.log.finished {
        "j/k défiler   G suivre   q quitter"
    } else {
        "s consigne   S rediriger   x arrêter   j/k défiler   G suivre   q quitter"
    };
    let footer = if state.status.is_empty() {
        keys.to_string()
    } else {
        format!("{}   ·   {keys}", state.status)
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            footer,
            Style::default().add_modifier(Modifier::DIM),
        ))),
        chunks[2],
    );
}
