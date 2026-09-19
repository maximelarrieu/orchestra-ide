//! Screens. The board and cost views shipped in phases 0 and 1; the ticket,
//! new-ticket and team screens come with phase 2. The agent view arrives with
//! phase 3, when there is a live agent to watch.

pub mod board;
pub mod cost;
pub mod new_ticket;
pub mod proposal;
pub mod ticket;

pub use board::{pane_block, render};
