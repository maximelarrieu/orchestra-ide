//! Screens. The board and cost views shipped in phases 0 and 1; the ticket,
//! new-ticket and team screens come with phase 2. The agent view arrives with
//! phase 3, when there is a live agent to watch.

pub mod agent;
pub mod board;
pub mod cost;
pub mod diff;
pub mod epic;
pub mod layout;
pub mod new_ticket;
pub mod proposal;
pub mod rules;
pub mod ticket;
pub mod todo;

pub use layout::{pane_block, render, truncate};

/// What a frame drawn at `width` × `height` reads as, one string per row.
///
/// The test terminal behind every screen test, kept here so that each test
/// says only what it draws. Public for the integration tests; not an API.
#[doc(hidden)]
pub fn text_of(width: u16, height: u16, draw: impl FnOnce(&mut ratatui::Frame<'_>)) -> String {
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
        .expect("un terminal de test se crée toujours");
    term.draw(draw).expect("le rendu de test n'échoue pas");
    let buf = term.backend().buffer();
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
