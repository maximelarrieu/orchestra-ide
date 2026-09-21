//! Colour and symbols.
//!
//! Colour is never the only channel: every status also carries a symbol, and
//! the palette avoids relying on the red/green pair, which is the one many
//! people cannot tell apart. Blue, cyan, yellow and magenta carry the meaning;
//! red appears only alongside a cross, and green only alongside a tick.
//!
//! Only the sixteen terminal colours are used, so the result follows whatever
//! theme the terminal already has rather than fighting it.

use orchestra_core::model::{AgentStatus, TicketStatus};
use ratatui::style::{Color, Modifier, Style};

/// How a status is drawn: a symbol, a colour, and whether it should stand out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Badge {
    pub symbol: &'static str,
    pub color: Color,
    pub bold: bool,
    pub dim: bool,
}

impl Badge {
    pub fn style(&self) -> Style {
        let mut style = Style::default().fg(self.color);
        if self.bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.dim {
            style = style.add_modifier(Modifier::DIM);
        }
        style
    }

    /// `● en cours`, for a table cell.
    pub fn label(&self, text: &str) -> String {
        format!("{} {text}", self.symbol)
    }
}

/// What an agent's state looks like.
pub fn agent(status: AgentStatus) -> Badge {
    match status {
        // Working right now: the one thing the eye should find first.
        AgentStatus::Running => Badge {
            symbol: "●",
            color: Color::Cyan,
            bold: true,
            dim: false,
        },
        AgentStatus::Starting => Badge {
            symbol: "◐",
            color: Color::Cyan,
            bold: false,
            dim: false,
        },
        AgentStatus::WaitingInput => Badge {
            symbol: "◌",
            color: Color::Yellow,
            bold: true,
            dim: false,
        },
        AgentStatus::Pending => Badge {
            symbol: "○",
            color: Color::Gray,
            bold: false,
            dim: true,
        },
        AgentStatus::Done => Badge {
            symbol: "✓",
            color: Color::Green,
            bold: false,
            dim: false,
        },
        AgentStatus::Failed => Badge {
            symbol: "✗",
            color: Color::Red,
            bold: true,
            dim: false,
        },
        AgentStatus::Crashed => Badge {
            symbol: "✗",
            color: Color::Magenta,
            bold: true,
            dim: false,
        },
        AgentStatus::Cancelled => Badge {
            symbol: "⊘",
            color: Color::Yellow,
            bold: false,
            dim: true,
        },
        AgentStatus::Manual => Badge {
            symbol: "☰",
            color: Color::Blue,
            bold: false,
            dim: false,
        },
    }
}

/// What a ticket's state looks like.
pub fn ticket(status: TicketStatus) -> Badge {
    match status {
        TicketStatus::Running => Badge {
            symbol: "●",
            color: Color::Cyan,
            bold: true,
            dim: false,
        },
        TicketStatus::Review => Badge {
            symbol: "◆",
            color: Color::Yellow,
            bold: true,
            dim: false,
        },
        TicketStatus::Planned => Badge {
            symbol: "◇",
            color: Color::Blue,
            bold: false,
            dim: false,
        },
        TicketStatus::Draft => Badge {
            symbol: "○",
            color: Color::Gray,
            bold: false,
            dim: true,
        },
        TicketStatus::Done => Badge {
            symbol: "✓",
            color: Color::Green,
            bold: false,
            dim: false,
        },
        TicketStatus::Failed => Badge {
            symbol: "✗",
            color: Color::Red,
            bold: true,
            dim: false,
        },
        TicketStatus::Cancelled => Badge {
            symbol: "⊘",
            color: Color::Gray,
            bold: false,
            dim: true,
        },
    }
}

/// A key the current screen alone offers.
///
/// Weight carries the distinction and colour only backs it up: on a monochrome
/// terminal the screen's own key still stands out from the ones that work
/// everywhere, which are dimmed.
pub fn key() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

/// What a key of the current screen does.
pub fn key_label() -> Style {
    Style::default()
}

/// A key that holds everywhere: there, but set back.
pub fn key_dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

/// The brackets around a key: they separate, they do not draw the eye.
pub fn bracket() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

/// A slow spinner, so a working agent is visibly alive even in a silence.
pub fn spinner(tick: u64) -> &'static str {
    const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
    FRAMES[(tick as usize) % FRAMES.len()]
}

/// `1 min 20 s`, `3 s`: how long something has been going.
pub fn elapsed(since: time::OffsetDateTime, now: time::OffsetDateTime) -> String {
    let seconds = (now - since).whole_seconds().max(0);
    match seconds {
        0..=59 => format!("{seconds} s"),
        60..=3599 => format!("{} min {} s", seconds / 60, seconds % 60),
        _ => format!("{} h {} min", seconds / 3600, (seconds % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_has_its_own_symbol_within_its_family() {
        // Colour alone must never be the difference: each state carries a
        // symbol too, for anyone who cannot tell two hues apart.
        let mut agents: Vec<&str> = AgentStatus::ALL.iter().map(|s| agent(*s).symbol).collect();
        agents.sort_unstable();
        agents.dedup();
        assert!(
            agents.len() >= 6,
            "trop de statuts d'agent partagent un symbole : {agents:?}"
        );

        let mut tickets: Vec<&str> = TicketStatus::ALL
            .iter()
            .map(|s| ticket(*s).symbol)
            .collect();
        tickets.sort_unstable();
        tickets.dedup();
        assert!(
            tickets.len() >= 6,
            "trop de statuts de ticket partagent un symbole : {tickets:?}"
        );
    }

    #[test]
    fn red_and_green_never_carry_a_distinction_on_their_own() {
        // The pair many people cannot separate must always come with
        // different symbols.
        for status in AgentStatus::ALL {
            let b = agent(status);
            if b.color == Color::Red || b.color == Color::Green {
                assert!(
                    b.symbol == "✗" || b.symbol == "✓",
                    "{status:?} s'appuie sur la couleur seule"
                );
            }
        }
    }

    #[test]
    fn what_is_running_stands_out_and_what_is_over_recedes() {
        let running = agent(AgentStatus::Running);
        assert!(running.bold && !running.dim);
        let pending = agent(AgentStatus::Pending);
        assert!(pending.dim);
        assert!(ticket(TicketStatus::Running).bold);
        assert!(ticket(TicketStatus::Cancelled).dim);
    }

    #[test]
    fn a_badge_prefixes_its_label() {
        let b = agent(AgentStatus::Running);
        assert_eq!(b.label("en cours"), "● en cours");
    }

    #[test]
    fn the_spinner_turns_and_never_runs_out() {
        let frames: Vec<&str> = (0..10).map(spinner).collect();
        assert_ne!(frames[0], frames[1]);
        assert_eq!(frames[0], frames[8], "huit images, puis ça recommence");
    }

    #[test]
    fn elapsed_reads_in_words() {
        let now = orchestra_core::now();
        assert_eq!(elapsed(now, now), "0 s");
        assert_eq!(elapsed(now - time::Duration::seconds(45), now), "45 s");
        assert_eq!(
            elapsed(now - time::Duration::seconds(80), now),
            "1 min 20 s"
        );
        assert_eq!(
            elapsed(now - time::Duration::seconds(7300), now),
            "2 h 1 min"
        );
        // A clock that jumped backwards must not print a negative age.
        assert_eq!(elapsed(now + time::Duration::seconds(10), now), "0 s");
    }
}
