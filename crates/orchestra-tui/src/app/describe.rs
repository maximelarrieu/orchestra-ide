//! Events in words: the activity strip, the planning trace, agent names.

use super::*;

/// How an agent is named on screen.
///
/// A relecture can send a role back to work, so the same role appears several
/// times on one ticket. Two identical rows would be unreadable, hence the run
/// number on every one but the first.
pub fn agent_label(agents: &[orchestra_core::protocol::AgentSummary], index: usize) -> String {
    let Some(agent) = agents.get(index) else {
        return String::new();
    };
    let earlier = agents[..index]
        .iter()
        .filter(|a| a.agent.role == agent.agent.role)
        .count();
    if earlier == 0 {
        agent.agent.role.clone()
    } else {
        format!("{} · reprise {earlier}", agent.agent.role)
    }
}

/// How many lines of the orchestrator's work the ticket screen keeps.
pub(super) const PLAN_TRACE_MAX: usize = 8;

/// One line of the planning trace, or `None` for an event that says nothing
/// about what the orchestrator is doing.
///
/// Deliberately not [`describe`]: this reads as a running commentary of one
/// run, in the present tense, not as a log of the whole daemon.
pub(super) fn plan_line(e: &Event) -> Option<String> {
    match &e.kind {
        EventKind::ToolStarted { tool, summary, .. } => {
            let verb = match tool.as_str() {
                "Read" => "lit",
                "Grep" => "cherche",
                "Glob" => "liste",
                other => other,
            };
            let summary = summary.trim();
            Some(if summary.is_empty() {
                verb.to_string()
            } else {
                format!("{verb} {summary}")
            })
        }
        // The content of a thinking block is never stored; its size still says
        // that something is happening.
        EventKind::AgentThinking { chars } => Some(format!(
            "réfléchit ({} caractères)",
            orchestra_core::pricing::fmt_tokens(*chars as u64)
        )),
        EventKind::AgentText { text } => {
            let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
            Some(orchestra_core::claude::stream::truncate(line, 70))
        }
        _ => None,
    }
}

/// One activity line for an event, or `None` when it is not worth showing.
pub fn describe(e: &Event) -> Option<String> {
    let t = e.ts.time();
    let stamp = format!("{:02}:{:02}:{:02}", t.hour(), t.minute(), t.second());
    let body = match &e.kind {
        EventKind::DaemonStarted { version } => format!("daemon {version} démarré"),
        EventKind::ProjectAdded { name, .. } => format!("projet « {name} » ajouté"),
        EventKind::ProjectForgotten { name } => format!("projet « {name} » oublié"),
        EventKind::TicketCreated { number, title } => format!("ticket #{number} « {title} » créé"),
        EventKind::TicketStatusChanged { from, to } => {
            format!("ticket {} → {}", from.label_fr(), to.label_fr())
        }
        EventKind::ProposalReady { proposal } => {
            format!("équipe proposée : {} rôles", proposal.members.len())
        }
        EventKind::ProposalFailed { error } => format!("planification échouée : {error}"),
        EventKind::WorktreeCreated { branch, .. } => format!("worktree sur {branch}"),
        EventKind::AgentSpawned { role, pid, .. } => format!("agent {role} démarré (pid {pid})"),
        EventKind::AgentStatusChanged { status, reason } => match reason {
            Some(r) => format!("agent {} — {}", status.label_fr(), r.label_fr()),
            None => format!("agent {}", status.label_fr()),
        },
        EventKind::ToolStarted { tool, summary, .. } => format!("{tool} {summary}"),
        EventKind::ToolFinished { ok, summary, .. } => {
            format!("{} {summary}", if *ok { "ok" } else { "échec" })
        }
        EventKind::AgentSteered { text, hard, .. } => {
            format!(
                "{} : {text}",
                if *hard { "redirection" } else { "consigne" }
            )
        }
        EventKind::AgentResult {
            subtype, num_turns, ..
        } => {
            format!("agent terminé ({subtype}, {num_turns} tours)")
        }
        EventKind::AgentStalled { silent_secs } => {
            format!("⚑ agent silencieux depuis {} min", silent_secs / 60)
        }
        EventKind::PullRequestOpened { url, number } => match number {
            Some(n) => format!("pull request #{n} ouverte — {url}"),
            None => format!("pull request ouverte — {url}"),
        },
        EventKind::PullRequestClosed { merged, .. } => {
            if *merged {
                "pull request fusionnée".to_string()
            } else {
                "pull request fermée sans fusion".to_string()
            }
        }
        EventKind::TicketResumed { skipped } if skipped.is_empty() => "ticket repris".to_string(),
        EventKind::TicketResumed { skipped } => {
            format!("ticket repris — déjà fait : {}", skipped.join(", "))
        }
        EventKind::TicketMerged {
            branch,
            into,
            commits,
            pushed_to,
        } => match pushed_to {
            Some(remote) => {
                format!("{branch} fusionnée dans {into} ({commits} commits), poussée sur {remote}")
            }
            None => format!("{branch} fusionnée dans {into} ({commits} commits)"),
        },
        EventKind::MergeBlocked { branch, reason, .. } => {
            format!("fusion de {branch} en attente : {reason}")
        }
        EventKind::ReviewVerdict {
            round,
            verdict,
            roles,
            ..
        } => {
            let who = if roles.is_empty() {
                String::new()
            } else {
                format!(" — {} repasse(nt)", roles.join(", "))
            };
            format!("relecture {round} : {}{who}", verdict.label_fr())
        }
        EventKind::CheckStarted { command, .. } => format!("vérification : {command}…"),
        EventKind::CheckFinished { run, .. } => {
            let mark = if run.ok { "✓" } else { "✗" };
            format!("{mark} {}", run.label_fr())
        }
        EventKind::HookBlocked { tool, reason } => format!("{tool} bloqué : {reason}"),
        EventKind::UnmanagedSessionSeen { cwd, .. } => {
            format!("session libre détectée dans {}", cwd.display())
        }
        EventKind::Warning { message } => format!("attention : {message}"),
        EventKind::RuleProposed {
            rule_kind,
            title,
            by,
            ..
        } => format!("{} proposée par {by} : « {title} » — à valider (8)", rule_kind.label_fr()),
        EventKind::RuleCreated { rule_kind, title, .. } => {
            format!("{} « {title} » créée", rule_kind.label_fr())
        }
        EventKind::RuleStatusChanged {
            rule_kind,
            title,
            to,
            ..
        } => format!("{} « {title} » : {}", rule_kind.label_fr(), to.label_fr()),
        EventKind::RoleCreated { name, .. } => format!("rôle « {name} » créé"),
        EventKind::RoleUpdated { name, change } => format!("rôle « {name} » : {change}"),
        EventKind::RoleDeleted { name } => format!("rôle « {name} » supprimé"),
        EventKind::RuleDeleted { rule_kind, title, .. } => {
            format!("{} « {title} » supprimée", rule_kind.label_fr())
        }
        EventKind::RulesChecked { round, violations } => {
            if violations.is_empty() {
                format!("conventions respectées (passe {round})")
            } else {
                format!(
                    "conventions : {} écart(s), passe {round} — {}",
                    violations.len(),
                    violations[0].line()
                )
            }
        }
        EventKind::TodoAdded { title } => format!("todo « {title} » ajouté"),
        EventKind::TodoUpdated { title } => format!("todo « {title} » modifié"),
        EventKind::TodoStatusChanged { title, from, to } => {
            format!("todo « {title} » : {} → {}", from.label_fr(), to.label_fr())
        }
        EventKind::TodoDeleted { title } => format!("todo « {title} » supprimé"),
        EventKind::TodoPromoted {
            title,
            ticket_number,
            project_name,
        } => format!("todo « {title} » promu en ticket #{ticket_number} sur {project_name}"),
        // Too chatty or not user-facing.
        EventKind::AgentText { .. }
        | EventKind::AgentThinking { .. }
        | EventKind::Usage { .. }
        | EventKind::SubagentStarted { .. }
        | EventKind::SubagentFinished { .. }
        | EventKind::PaneOpened { .. }
        | EventKind::PaneClosed { .. } => return None,
    };
    Some(format!("{stamp}  {body}"))
}
