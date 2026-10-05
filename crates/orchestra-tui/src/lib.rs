//! Orchestra terminal dashboard.

pub mod app;
pub mod client;
pub mod forms;
mod inflight;
pub mod keymap;
pub mod keys;
mod run;
pub mod screens;
mod tail;
pub mod theme;
pub mod widgets;

pub use app::{App, Msg, Screen};
pub use client::{Client, ClientHandle};
pub use run::run;
pub use tail::tail;
