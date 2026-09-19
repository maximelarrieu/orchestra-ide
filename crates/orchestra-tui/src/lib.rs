//! Orchestra terminal dashboard.

pub mod app;
pub mod client;
pub mod keymap;
mod run;
pub mod screens;

pub use app::{App, Msg, Screen};
pub use client::{Client, ClientHandle};
pub use run::run;
