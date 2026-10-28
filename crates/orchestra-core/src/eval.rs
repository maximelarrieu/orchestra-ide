//! Reference tickets, to compare roles, prompts and models on the same work.
//!
//! A case is a file in `<projet>/.orchestra/evals/`: a brief and the team that
//! does it. The team is written by the user, so running it respects rule 9 —
//! nothing here plans or decides. The farm does the work and the daemon's own
//! measures (status, verdict, tokens, cost) are what get compared: no model is
//! asked how it did.

use serde::Deserialize;

use crate::error::{CoreError, Result};
use crate::model::{TeamMember, TicketStatus};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EvalCase {
    /// Set from the file name, not read from the file.
    #[serde(skip)]
    pub name: String,
    pub title: String,
    pub brief: String,
    pub team: Vec<EvalMember>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EvalMember {
    pub role: String,
    pub objective: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub acceptance: Vec<String>,
}

impl EvalCase {
    pub fn parse(name: &str, src: &str) -> Result<Self> {
        let mut case: EvalCase = toml::from_str(src)
            .map_err(|e| CoreError::Parse(format!("cas d'évaluation « {name} » : {e}")))?;
        if case.team.is_empty() {
            return Err(CoreError::Parse(format!(
                "cas d'évaluation « {name} » : l'équipe est vide"
            )));
        }
        case.name = name.to_string();
        Ok(case)
    }

    pub fn members(&self) -> Vec<TeamMember> {
        self.team
            .iter()
            .map(|m| TeamMember {
                role: m.role.clone(),
                objective: m.objective.clone(),
                depends_on: m.depends_on.clone(),
                model: m.model.clone(),
                effort: None,
                max_budget_usd: None,
                acceptance: m.acceptance.clone(),
                parallel_ok: false,
            })
            .collect()
    }
}

/// What one case came to.
#[derive(Debug, Clone, PartialEq)]
pub struct EvalResult {
    pub name: String,
    pub status: TicketStatus,
    /// What the last relecture said; `None` when none ran.
    pub ready: Option<bool>,
    pub cost_usd: Option<f64>,
    pub tokens: u64,
    pub secs: u64,
    pub timed_out: bool,
}

impl EvalResult {
    /// Done its job: the relecture cleared the branch.
    pub fn passed(&self) -> bool {
        !self.timed_out
            && matches!(self.status, TicketStatus::Review | TicketStatus::Done)
            && self.ready == Some(true)
    }
}

/// One line per case, then a total.
pub fn report(results: &[EvalResult]) -> String {
    let mut out = String::new();
    for r in results {
        let verdict = match r.ready {
            Some(true) => "prêt",
            Some(false) => "corrections",
            None => "sans verdict",
        };
        let status = if r.timed_out { "délai dépassé" } else { r.status.label_fr() };
        out.push_str(&format!(
            "{} {:<24} {:<14} {:<13} {:>9} {:>9} tokens {:>5}s\n",
            if r.passed() { "✔" } else { "✘" },
            r.name,
            status,
            verdict,
            r.cost_usd.map(crate::pricing::fmt_usd).unwrap_or_else(|| "coût ?".into()),
            crate::pricing::fmt_tokens(r.tokens),
            r.secs
        ));
    }
    let passed = results.iter().filter(|r| r.passed()).count();
    let cost: f64 = results.iter().filter_map(|r| r.cost_usd).sum();
    out.push_str(&format!(
        "\n{passed}/{} réussis · {} au total (indicatif)\n",
        results.len(),
        crate::pricing::fmt_usd(cost)
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASE: &str = r#"
title = "Ajouter un cache"
brief = "Mets un cache LRU devant la lecture."

[[team]]
role = "backend"
objective = "Écrire le cache"

[[team]]
role = "reviewer"
objective = "Relire"
depends_on = ["backend"]
"#;

    #[test]
    fn a_case_is_read_with_its_name_and_team() {
        let c = EvalCase::parse("cache", CASE).unwrap();
        assert_eq!(c.name, "cache");
        let members = c.members();
        assert_eq!(members.len(), 2);
        assert_eq!(members[1].depends_on, vec!["backend".to_string()]);
    }

    #[test]
    fn an_empty_team_or_a_typo_is_refused() {
        assert!(EvalCase::parse("x", "title = \"t\"\nbrief = \"b\"\nteam = []").is_err());
        assert!(EvalCase::parse("x", "title = \"t\"").is_err());
    }

    fn result(name: &str, ready: Option<bool>, status: TicketStatus) -> EvalResult {
        EvalResult {
            name: name.into(),
            status,
            ready,
            cost_usd: Some(1.5),
            tokens: 2000,
            secs: 90,
            timed_out: false,
        }
    }

    #[test]
    fn only_a_cleared_branch_passes() {
        assert!(result("a", Some(true), TicketStatus::Review).passed());
        assert!(!result("b", Some(false), TicketStatus::Review).passed());
        assert!(!result("c", None, TicketStatus::Review).passed());
        assert!(!result("d", Some(true), TicketStatus::Failed).passed());
        let mut late = result("e", Some(true), TicketStatus::Review);
        late.timed_out = true;
        assert!(!late.passed());
    }

    #[test]
    fn the_report_counts_passes_and_sums_cost() {
        let text = report(&[
            result("a", Some(true), TicketStatus::Review),
            result("b", Some(false), TicketStatus::Review),
        ]);
        assert!(text.contains("1/2 réussis"), "{text}");
        assert!(text.contains("✔") && text.contains("✘"), "{text}");
    }
}
