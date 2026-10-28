//! Model escalation: a role run again on a ticket — a correction round, a
//! second relecture — has failed once on the model it started with, so each
//! rerun climbs one rung of the configured ladder. Pure: the daemon counts the
//! reruns and applies the answer.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutingConfig {
    /// Models from the cheapest to the most capable, by alias or full name
    /// (`["haiku", "sonnet", "opus"]`). Empty: no escalation.
    pub escalate: Vec<String>,
}

impl RoutingConfig {
    /// The model a role should run on after `reruns` earlier runs on the same
    /// ticket. `None` when nothing changes: no ladder, no rerun, a model that
    /// is not on the ladder, or already at the top.
    pub fn escalate(&self, current: &str, reruns: usize) -> Option<String> {
        if reruns == 0 {
            return None;
        }
        let at = self.rung(current)?;
        let to = (at + reruns).min(self.escalate.len() - 1);
        (to > at).then(|| self.escalate[to].clone())
    }

    /// The rung a model sits on: the ladder entry that names it, as an alias
    /// (`sonnet`) or inside a full id (`claude-sonnet-4-5-…`).
    fn rung(&self, model: &str) -> Option<usize> {
        let model = model.to_ascii_lowercase();
        self.escalate
            .iter()
            .position(|m| !m.is_empty() && model.contains(&m.to_ascii_lowercase()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ladder() -> RoutingConfig {
        RoutingConfig { escalate: vec!["haiku".into(), "sonnet".into(), "opus".into()] }
    }

    #[test]
    fn a_first_run_never_changes_model() {
        assert_eq!(ladder().escalate("haiku", 0), None);
    }

    #[test]
    fn each_rerun_climbs_one_rung_up_to_the_top() {
        assert_eq!(ladder().escalate("haiku", 1).as_deref(), Some("sonnet"));
        assert_eq!(ladder().escalate("haiku", 2).as_deref(), Some("opus"));
        assert_eq!(ladder().escalate("haiku", 9).as_deref(), Some("opus"));
        assert_eq!(ladder().escalate("opus", 3), None);
    }

    #[test]
    fn a_full_model_id_finds_its_rung() {
        assert_eq!(
            ladder().escalate("claude-sonnet-4-5-20250929", 1).as_deref(),
            Some("opus")
        );
    }

    #[test]
    fn no_ladder_or_unknown_model_means_no_change() {
        assert_eq!(RoutingConfig::default().escalate("haiku", 2), None);
        assert_eq!(ladder().escalate("gpt-x", 1), None);
    }
}
