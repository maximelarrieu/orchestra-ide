//! Did a convention help? Compares the tickets that ended before it was
//! accepted with those that ended after. Pure: the daemon reads the events and
//! the ledger and hands the outcomes in.
//!
//! This is a comparison, not a proof — other things change between two
//! periods — so each side states how many tickets it rests on, and the answer
//! says when either side is too thin to read anything into.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Fewer tickets than this on a side and the average says nothing.
pub const MIN_TICKETS: usize = 3;

/// What one ended ticket cost in friction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TicketOutcome {
    #[serde(with = "time::serde::rfc3339")]
    pub ended_at: OffsetDateTime,
    /// Blocking verdicts of the relecture.
    pub correction_rounds: u32,
    /// Red runs of the repository checks.
    pub red_checks: u32,
    pub failed: bool,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Averages {
    pub tickets: usize,
    pub correction_rounds: f64,
    pub red_checks: f64,
    /// Share of tickets that failed, from 0 to 1.
    pub failure_rate: f64,
    /// Over the tickets whose cost is known.
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RuleEffect {
    pub before: Averages,
    pub after: Averages,
    /// Both sides rest on at least [`MIN_TICKETS`] tickets.
    pub readable: bool,
}

pub fn effect_of(accepted_at: OffsetDateTime, outcomes: &[TicketOutcome]) -> RuleEffect {
    let (before, after): (Vec<_>, Vec<_>) = outcomes.iter().partition(|o| o.ended_at < accepted_at);
    let before = averages(&before);
    let after = averages(&after);
    let readable = before.tickets >= MIN_TICKETS && after.tickets >= MIN_TICKETS;
    RuleEffect { before, after, readable }
}

fn averages(group: &[&TicketOutcome]) -> Averages {
    let n = group.len();
    if n == 0 {
        return Averages::default();
    }
    let mean = |f: &dyn Fn(&TicketOutcome) -> f64| group.iter().map(|o| f(o)).sum::<f64>() / n as f64;
    let costs: Vec<f64> = group.iter().filter_map(|o| o.cost_usd).collect();
    Averages {
        tickets: n,
        correction_rounds: mean(&|o| f64::from(o.correction_rounds)),
        red_checks: mean(&|o| f64::from(o.red_checks)),
        failure_rate: mean(&|o| if o.failed { 1.0 } else { 0.0 }),
        cost_usd: (!costs.is_empty()).then(|| costs.iter().sum::<f64>() / costs.len() as f64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: i64) -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH + time::Duration::days(day)
    }

    fn outcome(day: i64, rounds: u32, failed: bool, cost: f64) -> TicketOutcome {
        TicketOutcome {
            ended_at: at(day),
            correction_rounds: rounds,
            red_checks: 0,
            failed,
            cost_usd: Some(cost),
        }
    }

    #[test]
    fn tickets_are_split_by_the_acceptance_date() {
        let outcomes = vec![
            outcome(1, 2, true, 4.0),
            outcome(2, 2, false, 6.0),
            outcome(3, 2, false, 5.0),
            outcome(11, 0, false, 2.0),
            outcome(12, 1, false, 3.0),
            outcome(13, 0, false, 1.0),
        ];
        let e = effect_of(at(10), &outcomes);
        assert_eq!((e.before.tickets, e.after.tickets), (3, 3));
        assert_eq!(e.before.correction_rounds, 2.0);
        assert!((e.after.correction_rounds - 1.0 / 3.0).abs() < 1e-9);
        assert!((e.before.failure_rate - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(e.before.cost_usd, Some(5.0));
        assert_eq!(e.after.cost_usd, Some(2.0));
        assert!(e.readable);
    }

    #[test]
    fn a_thin_side_is_not_readable() {
        let outcomes = vec![outcome(1, 2, false, 1.0), outcome(11, 0, false, 1.0)];
        assert!(!effect_of(at(10), &outcomes).readable);
    }

    #[test]
    fn no_ticket_gives_empty_averages() {
        let e = effect_of(at(10), &[]);
        assert_eq!(e.before, Averages::default());
        assert!(!e.readable);
    }
}
