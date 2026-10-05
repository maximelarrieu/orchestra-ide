//! Orchestra core: pure types and rules. No tokio, no SQLite, no terminal.

pub mod attention;
pub mod checks;
pub mod claude;
pub mod config;
pub mod conventions;
pub mod epic;
pub mod error;
pub mod events;
pub mod guard;
pub mod model;
pub mod pricing;
pub mod protocol;
pub mod review;
pub mod roles;
pub mod schema;

pub use error::{CoreError, Result};
pub use events::{Event, EventKind, NewEvent};
pub use model::*;

/// Crate version, also reported by the daemon in `Frame::Hello`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Current UTC time. Single place to swap for a fake clock in tests.
pub fn now() -> time::OffsetDateTime {
    time::OffsetDateTime::now_utc()
}
