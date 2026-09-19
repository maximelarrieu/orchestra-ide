//! Screens. Phase 0 shipped the board, phase 1 adds the cost view; the rest
//! are placeholders drawn by `board::render` so the tab strip stays honest
//! about what exists.

pub mod board;
pub mod cost;

pub use board::{pane_block, render};
