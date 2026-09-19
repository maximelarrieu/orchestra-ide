//! Running Claude Code as the worker.
//!
//! Phase 2 uses this for the orchestrator's planning call; phase 3 adds the
//! supervisor that drives one process per role.

pub mod claude;
pub mod translate;

pub use claude::{ClaudeCommand, ClaudeProcess, ProcessEvent, PromptVia};
pub use translate::translate;
