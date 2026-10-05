//! What waits on the user.
//!
//! A board of a dozen tickets says where each one is; it does not say which
//! ones are stuck until someone looks. The daemon derives, for each ticket, the
//! one thing it is waiting on the user for — if any — and the board gathers
//! them in a queue. One rule, here, so the daemon and every client agree on
//! what « à toi » means.

use serde::{Deserialize, Serialize};

use crate::model::TicketStatus;
use crate::review::Verdict;

/// The reason a ticket is waiting on the user, most urgent first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attention {
    /// A branch ready to merge, refused at the door.
    MergeBlocked,
    /// The repository's own checks are red and the repair rounds are spent.
    ChecksFailed,
    /// The relecture still blocks after its correction rounds, or ended
    /// without a readable verdict.
    ReviewBlocked,
    /// An agent asked a question and waits for the answer.
    AgentWaiting,
    /// The orchestrator's proposal waits to be read (rule 9).
    ProposalReady,
    /// The relecture found nothing blocking: the branch can be integrated.
    ReadyToIntegrate,
    /// A team was accepted and never launched.
    ReadyToLaunch,
    /// A pull request waits for its review on the forge.
    PullRequestOpen,
}

impl Attention {
    pub const ALL: [Attention; 8] = [
        Attention::MergeBlocked,
        Attention::ChecksFailed,
        Attention::ReviewBlocked,
        Attention::AgentWaiting,
        Attention::ProposalReady,
        Attention::ReadyToIntegrate,
        Attention::ReadyToLaunch,
        Attention::PullRequestOpen,
    ];

    pub fn label_fr(self) -> &'static str {
        match self {
            Attention::MergeBlocked => "fusion bloquée",
            Attention::ChecksFailed => "vérifications en échec",
            Attention::ReviewBlocked => "relecture bloquante",
            Attention::AgentWaiting => "un agent attend ta réponse",
            Attention::ProposalReady => "équipe à relire",
            Attention::ReadyToIntegrate => "prêt à intégrer",
            Attention::ReadyToLaunch => "équipe à lancer",
            Attention::PullRequestOpen => "PR à valider",
        }
    }

    /// Something went wrong, as opposed to a next step being ready.
    pub fn is_problem(self) -> bool {
        matches!(
            self,
            Attention::MergeBlocked
                | Attention::ChecksFailed
                | Attention::ReviewBlocked
                | Attention::AgentWaiting
        )
    }
}

/// What the daemon knows about a ticket that bears on the question.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub status: Option<TicketStatus>,
    pub has_proposal: bool,
    pub has_team: bool,
    pub agent_waiting: bool,
    /// The last relecture verdict, if there was one.
    pub verdict: Option<Verdict>,
    /// The last pass of checks failed.
    pub checks_failed: bool,
    pub merge_blocked: bool,
    pub pull_request_open: bool,
}

/// The one thing this ticket waits on the user for, if any.
pub fn of(f: &Facts) -> Option<Attention> {
    use TicketStatus::*;
    let status = f.status?;
    if f.agent_waiting && matches!(status, Running | Review) {
        return Some(Attention::AgentWaiting);
    }
    match status {
        // A draft with a proposal but no team: the proposal is what waits.
        // Without a proposal, planning is under way, or has not been asked.
        Draft if f.has_proposal && !f.has_team => Some(Attention::ProposalReady),
        Draft => None,
        Planned if f.has_team => Some(Attention::ReadyToLaunch),
        Planned => None,
        // The team works; nothing to do until it hands back.
        Running => None,
        Review => Some(if f.merge_blocked {
            Attention::MergeBlocked
        } else if f.checks_failed {
            Attention::ChecksFailed
        } else if f.pull_request_open {
            Attention::PullRequestOpen
        } else if f.verdict == Some(Verdict::Ready) {
            Attention::ReadyToIntegrate
        } else {
            // A blocking verdict after the last round, or none readable:
            // either way the loop handed the ticket back.
            Attention::ReviewBlocked
        }),
        Done | Failed | Cancelled => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(status: TicketStatus) -> Facts {
        Facts {
            status: Some(status),
            ..Default::default()
        }
    }

    #[test]
    fn a_draft_waits_only_once_its_proposal_is_there() {
        assert_eq!(of(&facts(TicketStatus::Draft)), None, "planification en cours");
        let mut f = facts(TicketStatus::Draft);
        f.has_proposal = true;
        assert_eq!(of(&f), Some(Attention::ProposalReady));
    }

    #[test]
    fn an_accepted_team_waits_to_be_launched() {
        let mut f = facts(TicketStatus::Planned);
        f.has_team = true;
        assert_eq!(of(&f), Some(Attention::ReadyToLaunch));
    }

    #[test]
    fn a_running_team_needs_nobody_unless_an_agent_asks() {
        let mut f = facts(TicketStatus::Running);
        assert_eq!(of(&f), None);
        f.agent_waiting = true;
        assert_eq!(of(&f), Some(Attention::AgentWaiting));
    }

    #[test]
    fn a_ticket_in_review_says_what_it_waits_for() {
        let mut f = facts(TicketStatus::Review);
        assert_eq!(of(&f), Some(Attention::ReviewBlocked), "sans verdict lisible");
        f.verdict = Some(Verdict::Changes);
        assert_eq!(of(&f), Some(Attention::ReviewBlocked));
        f.verdict = Some(Verdict::Ready);
        assert_eq!(of(&f), Some(Attention::ReadyToIntegrate));
        f.pull_request_open = true;
        assert_eq!(of(&f), Some(Attention::PullRequestOpen));
        f.checks_failed = true;
        assert_eq!(of(&f), Some(Attention::ChecksFailed), "un fait passe avant un avis");
        f.merge_blocked = true;
        assert_eq!(of(&f), Some(Attention::MergeBlocked));
    }

    #[test]
    fn closed_tickets_wait_for_nothing() {
        for s in [TicketStatus::Done, TicketStatus::Failed, TicketStatus::Cancelled] {
            let mut f = facts(s);
            f.has_proposal = true;
            f.merge_blocked = true;
            assert_eq!(of(&f), None, "{s:?}");
        }
    }

    #[test]
    fn problems_sort_ahead_of_next_steps() {
        let mut all = Attention::ALL.to_vec();
        all.sort();
        let first_step = all.iter().position(|a| !a.is_problem()).unwrap();
        assert!(all[first_step..].iter().all(|a| !a.is_problem()));
    }
}
