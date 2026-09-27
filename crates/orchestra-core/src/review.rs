//! The relecture verdict, as the reviewer writes it and the daemon reads it.
//!
//! The reviewer is an ordinary agent: it talks. What the supervisor needs from
//! it is one bit — does this branch pass — and, when it does not, who has to go
//! back to work. So the role's instructions end its message with a block the
//! machine can read:
//!
//! ```text
//! VERDICT: corrections
//! - backend: la pagination boucle quand `offset` dépasse le total (store/rows.rs:88)
//! - tests: rien ne couvre la liste vide
//! ```
//!
//! Parsing stays deliberately forgiving — accents, casing, `*` instead of `-`,
//! a stray blank line — because the text comes from a model. What it will not
//! do is guess: a message with no verdict at all yields `None`, and the
//! supervisor then stops and hands the ticket to the user rather than spending
//! another round on a reading it is not sure of.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::config::ReviewConfig;
use crate::model::{TeamMember, TeamProposal};

/// What the relecture concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Nothing blocks: the branch can go to the user.
    Ready,
    /// Something blocks, and the team has to fix it.
    Changes,
}

impl Verdict {
    pub fn is_ready(self) -> bool {
        matches!(self, Verdict::Ready)
    }

    pub fn label_fr(self) -> &'static str {
        match self {
            Verdict::Ready => "rien ne bloque",
            Verdict::Changes => "corrections demandées",
        }
    }
}

/// One blocking point, and the role the reviewer expects to fix it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeRequest {
    /// The role named at the head of the line, when one is.
    #[serde(default)]
    pub role: Option<String>,
    pub detail: String,
}

/// A parsed verdict block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub verdict: Verdict,
    #[serde(default)]
    pub changes: Vec<ChangeRequest>,
}

impl Review {
    /// The roles to send back to work, in the order the reviewer named them,
    /// keeping only those that are actually on the team. A blocking point
    /// naming nobody concerns everyone, hence the empty answer: the caller
    /// then falls back to the whole team.
    pub fn roles_to_fix<S: AsRef<str>>(&self, team: &[S]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for change in &self.changes {
            let Some(role) = &change.role else { continue };
            let Some(known) = team
                .iter()
                .map(|r| r.as_ref())
                .find(|r| normalize(r) == normalize(role))
            else {
                continue;
            };
            if !out.iter().any(|r| r == known) {
                out.push(known.to_string());
            }
        }
        out
    }

    /// Everything that blocks, as the correction prompt spells it out.
    pub fn blocking_lines(&self) -> Vec<String> {
        self.changes
            .iter()
            .map(|c| match &c.role {
                Some(role) => format!("{role} : {}", c.detail),
                None => c.detail.clone(),
            })
            .collect()
    }
}

/// The objective the relecture is given when the daemon adds it itself.
fn reviewer_objective(ticket_title: &str) -> String {
    format!(
        "Relire tout ce que l'équipe a livré sur cette branche pour « {} » : \
         conventions du dépôt, tests, et défauts réels. Rendre un verdict.",
        ticket_title.trim()
    )
}

/// Put the relecture at the end of a proposal, and make it depend on everyone.
///
/// Returns true when the proposal changed. The reviewer is added even when the
/// orchestrator did not ask for it — that is the point — but it lands in the
/// proposal the user relit before accepting, never behind his back.
pub fn append_reviewer(
    proposal: &mut TeamProposal,
    cfg: &ReviewConfig,
    known_roles: &BTreeSet<String>,
    ticket_title: &str,
) -> bool {
    if !cfg.enabled || !known_roles.contains(&cfg.role) || proposal.members.is_empty() {
        return false;
    }
    let others: Vec<String> = proposal
        .members
        .iter()
        .map(|m| m.role.clone())
        .filter(|r| r != &cfg.role)
        .collect();
    if others.is_empty() {
        // A ticket the orchestrator handed to the reviewer alone: nothing to
        // review, and nothing sensible to add.
        return false;
    }

    match proposal.members.iter().position(|m| m.role == cfg.role) {
        Some(i) => {
            // It is already there: make sure it really runs last.
            let member = &mut proposal.members[i];
            let mut changed = false;
            for role in &others {
                if !member.depends_on.contains(role) {
                    member.depends_on.push(role.clone());
                    changed = true;
                }
            }
            let member = proposal.members.remove(i);
            proposal.members.push(member);
            changed || i != proposal.members.len() - 1
        }
        None => {
            proposal.members.push(TeamMember {
                role: cfg.role.clone(),
                objective: reviewer_objective(ticket_title),
                depends_on: others,
                model: None,
                effort: None,
                max_budget_usd: None,
                parallel_ok: false,
            });
            true
        }
    }
}

/// Lowercase, without accents: `Prêt` and `pret` are the same word.
pub(crate) fn normalize(s: &str) -> String {
    s.trim()
        .chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

/// Strip one bullet marker (`-`, `*`, `•`, `1.`) from the head of a line.
pub(crate) fn strip_bullet(line: &str) -> Option<&str> {
    let t = line.trim();
    for marker in ["- ", "* ", "• ", "– ", "— "] {
        if let Some(rest) = t.strip_prefix(marker) {
            return Some(rest.trim());
        }
    }
    // `1.` / `2)` and friends.
    let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
    if !digits.is_empty() {
        let rest = &t[digits.len()..];
        for marker in [". ", ") ", "- "] {
            if let Some(rest) = rest.strip_prefix(marker) {
                return Some(rest.trim());
            }
        }
    }
    None
}

/// The `VERDICT:` value, if this line carries one.
fn verdict_on_line(line: &str) -> Option<Verdict> {
    // A bullet or a bold marker in front of it changes nothing.
    let cleaned = line.trim().replace(['*', '#', '_'], "");
    let cleaned = strip_bullet(&cleaned).unwrap_or(cleaned.trim()).to_string();
    let (head, tail) = cleaned.split_once([':', '：'])?;
    if normalize(head) != "verdict" {
        return None;
    }
    let value = normalize(tail);
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    // The first word decides; the reviewer often adds a comment after it.
    let word = value.split_whitespace().next().unwrap_or(value);
    match word.trim_matches(|c: char| !c.is_alphanumeric()) {
        "pret" | "prete" | "ready" | "ok" | "rien" | "aucune" => Some(Verdict::Ready),
        "corrections" | "correction" | "corriger" | "changes" | "bloque" | "bloquant"
        | "blocked" | "ko" => Some(Verdict::Changes),
        _ => None,
    }
}

/// Split `backend : ce qui casse` into its role and its detail.
fn split_role(item: &str) -> (Option<String>, String) {
    let Some((head, tail)) = item.split_once([':', '：']) else {
        return (None, item.trim().to_string());
    };
    let head = head.trim().trim_matches(|c: char| c == '`' || c == '*');
    let tail = tail.trim();
    // A role is one bare word. Anything longer is a sentence that happened to
    // contain a colon, and cutting it there would lose half the point.
    let is_role = !head.is_empty()
        && head.split_whitespace().count() == 1
        && head
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && !tail.is_empty();
    if is_role {
        (Some(head.to_string()), tail.to_string())
    } else {
        (None, item.trim().to_string())
    }
}

/// Read the verdict block out of a reviewer's closing message.
///
/// Returns `None` when there is none: the caller must not read silence as an
/// approval.
pub fn parse_review(text: &str) -> Option<Review> {
    let lines: Vec<&str> = text.lines().collect();
    // The last verdict wins: a reviewer that quotes the expected format before
    // filling it in would otherwise be read on its example.
    let (index, verdict) = lines
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, l)| verdict_on_line(l).map(|v| (i, v)))?;

    let mut changes = Vec::new();
    for line in lines.iter().skip(index + 1) {
        let Some(item) = strip_bullet(line) else {
            // Blank lines and prose in between are stepped over; the block ends
            // with the message.
            continue;
        };
        if item.is_empty() {
            continue;
        }
        let (role, detail) = split_role(item);
        changes.push(ChangeRequest { role, detail });
    }

    Some(Review { verdict, changes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_verdict_reads_ready() {
        let r = parse_review("Tout est cohérent.\n\nVERDICT: prêt\n").unwrap();
        assert_eq!(r.verdict, Verdict::Ready);
        assert!(r.changes.is_empty());
    }

    #[test]
    fn corrections_come_with_their_roles() {
        let text = "\
J'ai relu la branche.

VERDICT : corrections
- backend : `store/rows.rs:88` boucle quand offset dépasse le total
- tests: rien ne couvre la liste vide
";
        let r = parse_review(text).unwrap();
        assert_eq!(r.verdict, Verdict::Changes);
        assert_eq!(r.changes.len(), 2);
        assert_eq!(r.changes[0].role.as_deref(), Some("backend"));
        assert!(r.changes[0].detail.contains("offset"));
        assert_eq!(
            r.roles_to_fix(&["backend", "tests", "docs"]),
            vec!["backend".to_string(), "tests".to_string()]
        );
    }

    #[test]
    fn a_role_that_is_not_on_the_team_is_dropped() {
        let r = parse_review("VERDICT: corrections\n- devops: le pipeline manque").unwrap();
        assert!(r.roles_to_fix(&["backend"]).is_empty());
    }

    #[test]
    fn the_shape_of_the_line_is_forgiving() {
        for line in [
            "VERDICT: PRÊT",
            "**VERDICT:** ready",
            "- verdict : ok",
            "Verdict : prêt à fusionner",
            "VERDICT：pret",
        ] {
            assert_eq!(
                parse_review(line).map(|r| r.verdict),
                Some(Verdict::Ready),
                "{line}"
            );
        }
        for line in ["VERDICT: corrections", "verdict: KO", "VERDICT : bloquant"] {
            assert_eq!(
                parse_review(line).map(|r| r.verdict),
                Some(Verdict::Changes),
                "{line}"
            );
        }
    }

    #[test]
    fn silence_is_not_an_approval() {
        assert!(parse_review("La branche me paraît bonne, bon travail.").is_none());
        assert!(parse_review("").is_none());
        assert!(parse_review("VERDICT:").is_none());
        assert!(parse_review("VERDICT: mitigé").is_none());
    }

    #[test]
    fn the_last_verdict_wins_over_a_quoted_example() {
        let text = "\
Je dois terminer par « VERDICT: prêt » ou « VERDICT: corrections ».

VERDICT: corrections
- backend: il manque la migration
";
        let r = parse_review(text).unwrap();
        assert_eq!(r.verdict, Verdict::Changes);
        assert_eq!(r.changes.len(), 1);
    }

    fn proposal(roles: &[&str]) -> TeamProposal {
        TeamProposal {
            summary: "résumé".into(),
            members: roles
                .iter()
                .map(|r| TeamMember {
                    role: (*r).into(),
                    objective: format!("objectif de {r}"),
                    depends_on: vec![],
                    model: None,
                    effort: None,
                    max_budget_usd: None,
                    parallel_ok: false,
                })
                .collect(),
            risks: vec![],
            estimated_size: crate::model::Size::M,
        }
    }

    fn known(roles: &[&str]) -> BTreeSet<String> {
        roles.iter().map(|r| r.to_string()).collect()
    }

    #[test]
    fn the_relecture_is_added_last_and_depends_on_everyone() {
        let cfg = ReviewConfig::default();
        let mut p = proposal(&["backend", "tests"]);
        assert!(append_reviewer(
            &mut p,
            &cfg,
            &known(&["backend", "tests", "reviewer"]),
            "Ajouter un cache"
        ));
        let last = p.members.last().unwrap();
        assert_eq!(last.role, "reviewer");
        assert_eq!(last.depends_on, vec!["backend".to_string(), "tests".into()]);
        assert!(last.objective.contains("cache"));

        let team = crate::model::Team::from_proposal(&p, &known(&["backend", "tests", "reviewer"]))
            .unwrap();
        assert_eq!(team.stages.last().unwrap(), &vec!["reviewer".to_string()]);
    }

    #[test]
    fn a_relecture_the_orchestrator_already_asked_for_is_moved_to_the_end() {
        let cfg = ReviewConfig::default();
        let mut p = proposal(&["reviewer", "backend"]);
        assert!(append_reviewer(
            &mut p,
            &cfg,
            &known(&["backend", "reviewer"]),
            "t"
        ));
        assert_eq!(p.members.len(), 2, "pas de doublon");
        assert_eq!(p.members.last().unwrap().role, "reviewer");
        assert_eq!(
            p.members.last().unwrap().depends_on,
            vec!["backend".to_string()]
        );
    }

    #[test]
    fn nothing_is_added_when_it_is_switched_off_or_missing() {
        let off = ReviewConfig {
            enabled: false,
            ..ReviewConfig::default()
        };
        let mut p = proposal(&["backend"]);
        assert!(!append_reviewer(
            &mut p,
            &off,
            &known(&["backend", "reviewer"]),
            "t"
        ));
        assert_eq!(p.members.len(), 1);

        // The role is not in the catalog: the team stays as proposed.
        let cfg = ReviewConfig::default();
        let mut p = proposal(&["backend"]);
        assert!(!append_reviewer(&mut p, &cfg, &known(&["backend"]), "t"));
        assert_eq!(p.members.len(), 1);
    }

    #[test]
    fn a_sentence_with_a_colon_keeps_its_two_halves() {
        let r = parse_review("VERDICT: corrections\n- le cas limite suivant casse : offset nul")
            .unwrap();
        assert_eq!(r.changes[0].role, None);
        assert!(r.changes[0].detail.starts_with("le cas limite"));
        assert_eq!(r.blocking_lines(), vec![r.changes[0].detail.clone()]);
    }
}
