//! Reads already on their way to the daemon.
//!
//! The screen polls every second, and an event can ask for the same read
//! again in between: on the agent screen, every usage sample asked for the
//! whole ticket. When the daemon is slow — a large board, a busy store — those
//! requests piled up behind each other and all came back with the same answer.
//! A read is now sent once at a time; asked again meanwhile, it is sent once
//! more when the first answer lands, so what follows a change is never lost.

use std::sync::{Arc, Mutex};

use orchestra_core::protocol::Command;

/// Shared between the loop and the tasks that wait for replies.
#[derive(Clone, Default)]
pub struct Inflight {
    /// Each read in flight, and whether it was asked for again meanwhile.
    reads: Arc<Mutex<Vec<(Command, bool)>>>,
}

impl Inflight {
    /// Whether `cmd` should go now. Writes always do; a read that is already
    /// in flight is held back and remembered.
    pub fn begin(&self, cmd: &Command) -> bool {
        if !is_read(cmd) {
            return true;
        }
        let mut reads = self.reads.lock().unwrap_or_else(|e| e.into_inner());
        match reads.iter_mut().find(|(c, _)| c == cmd) {
            Some((_, again)) => {
                *again = true;
                false
            }
            None => {
                reads.push((cmd.clone(), false));
                true
            }
        }
    }

    /// The answer to `cmd` arrived. True when it was asked for again in the
    /// meantime and must go once more; it then stays in flight.
    pub fn finish(&self, cmd: &Command) -> bool {
        if !is_read(cmd) {
            return false;
        }
        let mut reads = self.reads.lock().unwrap_or_else(|e| e.into_inner());
        let Some(i) = reads.iter().position(|(c, _)| c == cmd) else {
            return false;
        };
        if reads[i].1 {
            reads[i].1 = false;
            true
        } else {
            reads.remove(i);
            false
        }
    }
}

/// Commands that only read, and so can be merged when they repeat.
pub fn is_read(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::ListTickets { .. }
            | Command::GetTicket { .. }
            | Command::GetUsage { .. }
            | Command::ListProjects
            | Command::ListAgents { .. }
            | Command::ListRoles { .. }
            | Command::ListRules { .. }
            | Command::ListTodos
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn tickets() -> Command {
        Command::ListTickets {
            project_id: None,
            status: None,
        }
    }

    #[test]
    fn a_read_goes_once_at_a_time() {
        let f = Inflight::default();
        assert!(f.begin(&tickets()));
        assert!(!f.begin(&tickets()), "déjà en route");
        assert!(!f.begin(&tickets()));
        // Asked again meanwhile: it goes once more, not twice.
        assert!(f.finish(&tickets()));
        assert!(!f.finish(&tickets()));
        // Nothing left in flight: the next one goes straight away.
        assert!(f.begin(&tickets()));
    }

    #[test]
    fn different_reads_do_not_hold_each_other_back() {
        let f = Inflight::default();
        let a = Command::GetTicket { ticket_id: Uuid::new_v4() };
        let b = Command::GetTicket { ticket_id: Uuid::new_v4() };
        assert!(f.begin(&a));
        assert!(f.begin(&b));
        assert!(f.begin(&tickets()));
    }

    #[test]
    fn writes_are_never_held_back() {
        let f = Inflight::default();
        let launch = Command::LaunchTicket {
            ticket_id: Uuid::new_v4(),
            open_panes: false,
        };
        assert!(f.begin(&launch));
        assert!(f.begin(&launch));
        assert!(!f.finish(&launch));
    }
}
