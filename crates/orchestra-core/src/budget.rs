//! Spending ceilings above the single agent. Pure: the daemon measures what
//! was spent, this says what is left and which ceiling speaks first.

use serde::{Deserialize, Serialize};

/// `[budget]`: indicative USD, priced from the models actually used (rule 8).
/// The ticket ceiling is `daemon.max_ticket_usd`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BudgetConfig {
    /// All the tickets of one epic together.
    pub epic_usd: Option<f64>,
    /// Everything one project has ever spent through Orchestra.
    pub project_usd: Option<f64>,
    /// Everything spent through Orchestra over the last 24 hours.
    pub day_usd: Option<f64>,
}

impl BudgetConfig {
    pub fn validate(&self) -> Result<(), String> {
        for (name, v) in [
            ("epic_usd", self.epic_usd),
            ("project_usd", self.project_usd),
            ("day_usd", self.day_usd),
        ] {
            if matches!(v, Some(b) if b <= 0.0) {
                return Err(format!("budget.{name} doit être positif"));
            }
        }
        Ok(())
    }
}

/// One ceiling and what has been spent under it.
#[derive(Debug, Clone, PartialEq)]
pub struct Ceiling {
    pub scope: &'static str,
    pub cap: f64,
    pub spent: f64,
}

/// A ceiling that is reached.
#[derive(Debug, Clone, PartialEq)]
pub struct Exceeded {
    pub scope: &'static str,
    pub cap: f64,
    pub spent: f64,
}

/// What an agent may still spend: the smallest margin among the ceilings, or
/// the first ceiling already reached. `None` when no ceiling applies.
pub fn left(ceilings: &[Ceiling]) -> Result<Option<f64>, Exceeded> {
    let mut least: Option<f64> = None;
    for c in ceilings {
        let margin = c.cap - c.spent;
        if margin <= 0.0 {
            return Err(Exceeded { scope: c.scope, cap: c.cap, spent: c.spent });
        }
        least = Some(least.map_or(margin, |l| l.min(margin)));
    }
    Ok(least)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(scope: &'static str, cap: f64, spent: f64) -> Ceiling {
        Ceiling { scope, cap, spent }
    }

    #[test]
    fn no_ceiling_means_no_limit() {
        assert_eq!(left(&[]), Ok(None));
    }

    #[test]
    fn the_tightest_margin_wins() {
        let got = left(&[c("ticket", 10.0, 2.0), c("jour", 5.0, 4.0)]);
        assert_eq!(got, Ok(Some(1.0)));
    }

    #[test]
    fn a_reached_ceiling_names_itself() {
        let got = left(&[c("ticket", 10.0, 2.0), c("projet", 3.0, 3.0)]);
        assert_eq!(got, Err(Exceeded { scope: "projet", cap: 3.0, spent: 3.0 }));
    }

    #[test]
    fn a_ceiling_must_be_positive() {
        let b = BudgetConfig { day_usd: Some(0.0), ..Default::default() };
        assert!(b.validate().is_err());
        assert!(BudgetConfig::default().validate().is_ok());
    }
}
