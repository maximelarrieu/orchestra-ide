//! Domain types shared by every component. Pure data + the rules that guard it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{CoreError, Result};

pub type ProjectId = Uuid;
pub type TicketId = Uuid;
pub type AgentId = Uuid;

// ---------------------------------------------------------------------------
// Project
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectKind {
    /// Added on purpose by the user; can host tickets.
    Managed,
    /// Auto-created from an unmanaged Claude Code session seen in the transcripts.
    Discovered,
}

impl ProjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectKind::Managed => "managed",
            ProjectKind::Discovered => "discovered",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "managed" => Ok(ProjectKind::Managed),
            "discovered" => Ok(ProjectKind::Discovered),
            other => Err(CoreError::Parse(format!(
                "type de projet inconnu : {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub path: PathBuf,
    pub default_branch: String,
    #[serde(default)]
    pub zellij_tab: Option<String>,
    pub kind: ProjectKind,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

// ---------------------------------------------------------------------------
// Ticket
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketStatus {
    Draft,
    Planned,
    Running,
    Review,
    Done,
    Failed,
    Cancelled,
}

impl TicketStatus {
    pub const ALL: [TicketStatus; 7] = [
        TicketStatus::Draft,
        TicketStatus::Planned,
        TicketStatus::Running,
        TicketStatus::Review,
        TicketStatus::Done,
        TicketStatus::Failed,
        TicketStatus::Cancelled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TicketStatus::Draft => "draft",
            TicketStatus::Planned => "planned",
            TicketStatus::Running => "running",
            TicketStatus::Review => "review",
            TicketStatus::Done => "done",
            TicketStatus::Failed => "failed",
            TicketStatus::Cancelled => "cancelled",
        }
    }

    /// Label shown in the TUI.
    pub fn label_fr(self) -> &'static str {
        match self {
            TicketStatus::Draft => "brouillon",
            TicketStatus::Planned => "planifié",
            TicketStatus::Running => "en cours",
            TicketStatus::Review => "à relire",
            TicketStatus::Done => "terminé",
            TicketStatus::Failed => "échoué",
            TicketStatus::Cancelled => "annulé",
        }
    }

    /// True once the ticket no longer moves on its own.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TicketStatus::Done | TicketStatus::Failed | TicketStatus::Cancelled
        )
    }

    /// Display order on the board (kanban columns).
    pub fn board_order(self) -> u8 {
        match self {
            TicketStatus::Running => 0,
            TicketStatus::Review => 1,
            TicketStatus::Planned => 2,
            TicketStatus::Draft => 3,
            TicketStatus::Failed => 4,
            TicketStatus::Done => 5,
            TicketStatus::Cancelled => 6,
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| CoreError::Parse(format!("statut de ticket inconnu : {s}")))
    }
}

impl fmt::Display for TicketStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The only place that knows the ticket lifecycle.
pub fn can_transition(from: TicketStatus, to: TicketStatus) -> bool {
    use TicketStatus::*;
    match (from, to) {
        (Draft, Planned) => true,
        (Planned, Running) => true,
        // A re-plan sends an already planned ticket back to draft.
        (Planned, Draft) => true,
        (Running, Review) | (Running, Failed) => true,
        (Review, Done) | (Review, Running) => true,
        // A failed ticket can be relaunched from a stage.
        (Failed, Running) => true,
        (Draft | Planned | Running | Review, Cancelled) => true,
        _ => false,
    }
}

pub fn check_transition(from: TicketStatus, to: TicketStatus) -> Result<()> {
    if can_transition(from, to) {
        Ok(())
    } else {
        Err(CoreError::IllegalTransition {
            from: from.as_str().to_string(),
            to: to.as_str().to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ticket {
    pub id: TicketId,
    pub project_id: ProjectId,
    /// Human-facing number, unique per project (`#12`).
    pub number: i64,
    pub title: String,
    pub brief: String,
    pub status: TicketStatus,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub worktree_path: Option<PathBuf>,
    /// Last proposal returned by the orchestrator, accepted or not.
    #[serde(default)]
    pub proposal: Option<TeamProposal>,
    /// Team accepted by the user; set when the ticket leaves `Draft`.
    #[serde(default)]
    pub team: Option<Team>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// `12-ajoute-le-cache` from `#12` and "Ajoute le cache": used for the branch
/// name and the worktree directory.
pub fn ticket_slug(number: i64, title: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = true;
    for ch in title.chars() {
        let c = match ch {
            'à' | 'â' | 'ä' | 'á' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'î' | 'ï' | 'í' => 'i',
            'ô' | 'ö' | 'ó' => 'o',
            'ù' | 'û' | 'ü' | 'ú' => 'u',
            'ÿ' => 'y',
            'ñ' => 'n',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && slug.len() < 40 {
            slug.push('-');
            last_dash = true;
        }
        if slug.len() >= 40 {
            break;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        format!("{number}")
    } else {
        format!("{number}-{slug}")
    }
}

// ---------------------------------------------------------------------------
// Team
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Size {
    Xs,
    S,
    M,
    L,
    Xl,
}

impl Size {
    pub fn as_str(self) -> &'static str {
        match self {
            Size::Xs => "XS",
            Size::S => "S",
            Size::M => "M",
            Size::L => "L",
            Size::Xl => "XL",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub const ALL: [Effort; 5] = [
        Effort::Low,
        Effort::Medium,
        Effort::High,
        Effort::Xhigh,
        Effort::Max,
    ];

    /// Value passed to `claude --effort`.
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Xhigh => "xhigh",
            Effort::Max => "max",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| CoreError::Parse(format!("niveau d'effort inconnu : {s}")))
    }

    /// Next value, wrapping. Used by the `e` key in the proposal editor.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|v| *v == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

impl fmt::Display for Effort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One role the orchestrator wants on the team, contextualised for this ticket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamMember {
    /// Must match a `RoleDefinition::name` from the catalog.
    pub role: String,
    pub objective: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<Effort>,
    #[serde(default)]
    pub max_budget_usd: Option<f64>,
    /// May run alongside its siblings in the same stage. Ignored in the MVP
    /// (everything is sequential), kept so the scheduler can use it later.
    #[serde(default)]
    pub parallel_ok: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamProposal {
    /// How the orchestrator understood the brief, in 2-3 lines.
    pub summary: String,
    pub members: Vec<TeamMember>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default = "default_size")]
    pub estimated_size: Size,
}

fn default_size() -> Size {
    Size::M
}

/// A proposal the user accepted, with its execution order resolved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Team {
    pub members: Vec<TeamMember>,
    /// Role names grouped by stage; every stage depends on the previous ones.
    pub stages: Vec<Vec<String>>,
}

impl Team {
    /// Validate a proposal against the catalog and resolve its stages.
    pub fn from_proposal(proposal: &TeamProposal, known_roles: &BTreeSet<String>) -> Result<Self> {
        if proposal.members.is_empty() {
            return Err(CoreError::Parse(
                "la proposition ne contient aucun rôle".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        for m in &proposal.members {
            if !known_roles.contains(&m.role) {
                return Err(CoreError::UnknownRole(m.role.clone()));
            }
            if !seen.insert(m.role.clone()) {
                return Err(CoreError::Parse(format!(
                    "le rôle « {} » apparaît deux fois dans la proposition",
                    m.role
                )));
            }
        }
        for m in &proposal.members {
            for dep in &m.depends_on {
                if !seen.contains(dep) {
                    return Err(CoreError::Parse(format!(
                        "le rôle « {} » dépend de « {dep} », absent de l'équipe",
                        m.role
                    )));
                }
                if dep == &m.role {
                    return Err(CoreError::CyclicDependencies(m.role.clone()));
                }
            }
        }
        let stages = topo_stages(&proposal.members)?;
        Ok(Team {
            members: proposal.members.clone(),
            stages,
        })
    }

    pub fn member(&self, role: &str) -> Option<&TeamMember> {
        self.members.iter().find(|m| m.role == role)
    }

    /// Flattened execution order: (stage index, member).
    pub fn ordered(&self) -> Vec<(u32, &TeamMember)> {
        let mut out = Vec::new();
        for (i, stage) in self.stages.iter().enumerate() {
            for role in stage {
                if let Some(m) = self.member(role) {
                    out.push((i as u32, m));
                }
            }
        }
        out
    }
}

/// Kahn's algorithm, grouping ties into stages. Deterministic: ties keep the
/// order the roles appear in the proposal.
fn topo_stages(members: &[TeamMember]) -> Result<Vec<Vec<String>>> {
    let mut pending: BTreeMap<usize, BTreeSet<String>> = members
        .iter()
        .enumerate()
        .map(|(i, m)| (i, m.depends_on.iter().cloned().collect()))
        .collect();

    let mut stages: Vec<Vec<String>> = Vec::new();
    let mut done: BTreeSet<String> = BTreeSet::new();

    while !pending.is_empty() {
        let mut ready: Vec<usize> = pending
            .iter()
            .filter(|(_, deps)| deps.iter().all(|d| done.contains(d)))
            .map(|(i, _)| *i)
            .collect();
        if ready.is_empty() {
            let stuck: Vec<&str> = pending.keys().map(|i| members[*i].role.as_str()).collect();
            return Err(CoreError::CyclicDependencies(stuck.join(", ")));
        }
        ready.sort_unstable();
        let stage: Vec<String> = ready.iter().map(|i| members[*i].role.clone()).collect();
        for i in &ready {
            pending.remove(i);
            done.insert(members[*i].role.clone());
        }
        stages.push(stage);
    }
    Ok(stages)
}

// ---------------------------------------------------------------------------
// Agent
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// Queued behind an earlier stage.
    Pending,
    /// Process spawned, no `system/init` seen yet.
    Starting,
    Running,
    /// Backing off (rate limit) or waiting for the user.
    WaitingInput,
    Done,
    Failed,
    Cancelled,
    /// Process died without a `result` line.
    Crashed,
    /// The user took the session over with `claude --resume`.
    Manual,
}

impl AgentStatus {
    pub const ALL: [AgentStatus; 9] = [
        AgentStatus::Pending,
        AgentStatus::Starting,
        AgentStatus::Running,
        AgentStatus::WaitingInput,
        AgentStatus::Done,
        AgentStatus::Failed,
        AgentStatus::Cancelled,
        AgentStatus::Crashed,
        AgentStatus::Manual,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AgentStatus::Pending => "pending",
            AgentStatus::Starting => "starting",
            AgentStatus::Running => "running",
            AgentStatus::WaitingInput => "waiting_input",
            AgentStatus::Done => "done",
            AgentStatus::Failed => "failed",
            AgentStatus::Cancelled => "cancelled",
            AgentStatus::Crashed => "crashed",
            AgentStatus::Manual => "manual",
        }
    }

    pub fn label_fr(self) -> &'static str {
        match self {
            AgentStatus::Pending => "en attente",
            AgentStatus::Starting => "démarrage",
            AgentStatus::Running => "en cours",
            AgentStatus::WaitingInput => "en pause",
            AgentStatus::Done => "terminé",
            AgentStatus::Failed => "échoué",
            AgentStatus::Cancelled => "annulé",
            AgentStatus::Crashed => "planté",
            AgentStatus::Manual => "manuel",
        }
    }

    /// True while a process is (or should be) alive.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            AgentStatus::Starting | AgentStatus::Running | AgentStatus::WaitingInput
        )
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            AgentStatus::Done | AgentStatus::Failed | AgentStatus::Cancelled | AgentStatus::Crashed
        )
    }

    pub fn parse(s: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| CoreError::Parse(format!("statut d'agent inconnu : {s}")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitReason {
    Success,
    MaxTurns,
    MaxBudget,
    ErrorDuringExecution,
    Killed,
    Interrupted,
    Crashed(i32),
    Unknown(String),
}

impl ExitReason {
    pub fn label_fr(&self) -> String {
        match self {
            ExitReason::Success => "succès".into(),
            ExitReason::MaxTurns => "limite de tours atteinte".into(),
            ExitReason::MaxBudget => "budget atteint".into(),
            ExitReason::ErrorDuringExecution => "erreur pendant l'exécution".into(),
            ExitReason::Killed => "arrêté".into(),
            ExitReason::Interrupted => "interrompu".into(),
            ExitReason::Crashed(code) => format!("planté (code {code})"),
            ExitReason::Unknown(s) => format!("inconnu ({s})"),
        }
    }

    /// Did the agent finish its job?
    pub fn is_success(&self) -> bool {
        matches!(self, ExitReason::Success)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub ticket_id: TicketId,
    pub project_id: ProjectId,
    pub role: String,
    pub objective: String,
    pub stage: u32,
    /// The uuid we pass to `claude --session-id`; also the transcript file name.
    pub session_id: Uuid,
    /// Empty string means "let Claude Code pick" (no `--model` flag).
    pub model: String,
    pub effort: Effort,
    #[serde(default)]
    pub max_budget_usd: Option<f64>,
    pub status: AgentStatus,
    #[serde(default)]
    pub exit_reason: Option<ExitReason>,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub pane_id: Option<String>,
    #[serde(default = "one")]
    pub attempt: u32,
    /// Final `result` text, handed to the next stage.
    #[serde(default)]
    pub handoff: Option<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub ended_at: Option<OffsetDateTime>,
}

fn one() -> u32 {
    1
}

/// Role name of the pseudo-agent that records the planning call's tokens.
pub const ORCHESTRATOR_ROLE: &str = "orchestrator";

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub cache_read: u64,
    #[serde(default)]
    pub cache_creation: u64,
    /// Thinking tokens, already counted inside `output`. Reported separately,
    /// never billed twice.
    #[serde(default)]
    pub thinking: u64,
}

impl Tokens {
    /// Everything that entered the model, cache included.
    pub fn total_input(&self) -> u64 {
        self.input + self.cache_read + self.cache_creation
    }

    pub fn total(&self) -> u64 {
        self.total_input() + self.output
    }

    /// Share of input tokens served from cache, 0.0 to 1.0.
    pub fn cache_hit_ratio(&self) -> f64 {
        let total = self.total_input();
        if total == 0 {
            0.0
        } else {
            self.cache_read as f64 / total as f64
        }
    }
}

impl std::ops::AddAssign for Tokens {
    fn add_assign(&mut self, o: Self) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_creation += o.cache_creation;
        self.thinking += o.thinking;
    }
}

impl std::iter::Sum for Tokens {
    fn sum<I: Iterator<Item = Tokens>>(iter: I) -> Self {
        let mut acc = Tokens::default();
        for t in iter {
            acc += t;
        }
        acc
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    /// Parsed from a managed agent's stdout.
    Stream,
    /// Parsed from a `~/.claude/projects/**/*.jsonl` transcript.
    Transcript,
}

impl UsageSource {
    pub fn as_str(self) -> &'static str {
        match self {
            UsageSource::Stream => "stream",
            UsageSource::Transcript => "transcript",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "stream" => Ok(UsageSource::Stream),
            "transcript" => Ok(UsageSource::Transcript),
            other => Err(CoreError::Parse(format!(
                "source d'usage inconnue : {other}"
            ))),
        }
    }
}

/// One API response's token cost. `message_id` is the dedupe key: the same
/// response reaches us once per content block in the transcript and once more
/// on a managed agent's stdout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageSample {
    pub message_id: String,
    pub session_id: Uuid,
    #[serde(default)]
    pub subagent_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<AgentId>,
    #[serde(default)]
    pub ticket_id: Option<TicketId>,
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    pub model: String,
    pub tokens: Tokens,
    #[serde(with = "time::serde::rfc3339")]
    pub ts: OffsetDateTime,
    pub source: UsageSource,
}

// ---------------------------------------------------------------------------
// Roles
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleScope {
    Global,
    Project,
}

/// A reusable agent role, parsed from Markdown with YAML frontmatter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleDefinition {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<Effort>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    #[serde(default)]
    pub max_budget_usd: Option<f64>,
    /// Inline subagent definitions passed to `claude --agents`.
    #[serde(default)]
    pub subagents: Option<serde_json::Value>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Markdown body, appended to the system prompt.
    pub system_prompt: String,
    pub source: PathBuf,
    pub scope: RoleScope,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(role: &str, deps: &[&str]) -> TeamMember {
        TeamMember {
            role: role.into(),
            objective: format!("objectif de {role}"),
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            model: None,
            effort: None,
            max_budget_usd: None,
            parallel_ok: false,
        }
    }

    fn catalog(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn proposal(members: Vec<TeamMember>) -> TeamProposal {
        TeamProposal {
            summary: "résumé".into(),
            members,
            risks: vec![],
            estimated_size: Size::M,
        }
    }

    #[test]
    fn transitions_follow_the_lifecycle() {
        use TicketStatus::*;
        assert!(can_transition(Draft, Planned));
        assert!(can_transition(Planned, Running));
        assert!(can_transition(Running, Review));
        assert!(can_transition(Review, Done));
        assert!(can_transition(Failed, Running));
        assert!(can_transition(Running, Cancelled));
        // Nothing comes back from a terminal state except Failed -> Running.
        assert!(!can_transition(Done, Running));
        assert!(!can_transition(Cancelled, Running));
        assert!(!can_transition(Draft, Running));
        assert!(!can_transition(Draft, Done));
        assert!(check_transition(Draft, Done).is_err());
    }

    #[test]
    fn stages_follow_dependencies() {
        let p = proposal(vec![
            member("architect", &[]),
            member("backend", &["architect"]),
            member("frontend", &["architect"]),
            member("reviewer", &["backend", "frontend"]),
        ]);
        let team = Team::from_proposal(
            &p,
            &catalog(&["architect", "backend", "frontend", "reviewer"]),
        )
        .unwrap();
        assert_eq!(
            team.stages,
            vec![
                vec!["architect".to_string()],
                vec!["backend".to_string(), "frontend".to_string()],
                vec!["reviewer".to_string()],
            ]
        );
        let order: Vec<(u32, &str)> = team
            .ordered()
            .into_iter()
            .map(|(s, m)| (s, m.role.as_str()))
            .collect();
        assert_eq!(
            order,
            vec![
                (0, "architect"),
                (1, "backend"),
                (1, "frontend"),
                (2, "reviewer")
            ]
        );
    }

    #[test]
    fn unknown_role_is_rejected() {
        let p = proposal(vec![member("wizard", &[])]);
        let err = Team::from_proposal(&p, &catalog(&["backend"])).unwrap_err();
        assert!(matches!(err, CoreError::UnknownRole(r) if r == "wizard"));
    }

    #[test]
    fn cycles_are_rejected() {
        let p = proposal(vec![
            member("backend", &["frontend"]),
            member("frontend", &["backend"]),
        ]);
        let err = Team::from_proposal(&p, &catalog(&["backend", "frontend"])).unwrap_err();
        assert!(matches!(err, CoreError::CyclicDependencies(_)));
    }

    #[test]
    fn self_dependency_is_rejected() {
        let p = proposal(vec![member("backend", &["backend"])]);
        let err = Team::from_proposal(&p, &catalog(&["backend"])).unwrap_err();
        assert!(matches!(err, CoreError::CyclicDependencies(_)));
    }

    #[test]
    fn dependency_outside_the_team_is_rejected() {
        let p = proposal(vec![member("backend", &["architect"])]);
        let err = Team::from_proposal(&p, &catalog(&["backend", "architect"])).unwrap_err();
        assert!(matches!(err, CoreError::Parse(_)));
    }

    #[test]
    fn duplicate_role_is_rejected() {
        let p = proposal(vec![member("backend", &[]), member("backend", &[])]);
        let err = Team::from_proposal(&p, &catalog(&["backend"])).unwrap_err();
        assert!(matches!(err, CoreError::Parse(_)));
    }

    #[test]
    fn empty_proposal_is_rejected() {
        let err = Team::from_proposal(&proposal(vec![]), &catalog(&["backend"])).unwrap_err();
        assert!(matches!(err, CoreError::Parse(_)));
    }

    #[test]
    fn tokens_add_and_report_cache() {
        let mut a = Tokens {
            input: 10,
            output: 5,
            cache_read: 80,
            cache_creation: 10,
            thinking: 2,
        };
        a += Tokens {
            input: 10,
            output: 5,
            cache_read: 0,
            cache_creation: 0,
            thinking: 1,
        };
        assert_eq!(a.input, 20);
        assert_eq!(a.output, 10);
        assert_eq!(a.thinking, 3);
        assert_eq!(a.total_input(), 110);
        assert_eq!(a.total(), 120);
        assert!((a.cache_hit_ratio() - 80.0 / 110.0).abs() < 1e-9);
        assert_eq!(Tokens::default().cache_hit_ratio(), 0.0);
    }

    #[test]
    fn effort_round_trips_and_cycles() {
        for e in Effort::ALL {
            assert_eq!(Effort::parse(e.as_str()).unwrap(), e);
        }
        assert_eq!(Effort::Low.next(), Effort::Medium);
        assert_eq!(Effort::Max.next(), Effort::Low);
        assert!(Effort::parse("turbo").is_err());
    }

    #[test]
    fn statuses_round_trip() {
        for s in TicketStatus::ALL {
            assert_eq!(TicketStatus::parse(s.as_str()).unwrap(), s);
        }
        for s in AgentStatus::ALL {
            assert_eq!(AgentStatus::parse(s.as_str()).unwrap(), s);
        }
        assert_eq!(ProjectKind::parse("managed").unwrap(), ProjectKind::Managed);
        assert!(ProjectKind::parse("nope").is_err());
        assert_eq!(UsageSource::parse("stream").unwrap(), UsageSource::Stream);
    }

    #[test]
    fn slugs_are_branch_safe() {
        assert_eq!(ticket_slug(12, "Ajoute le cache"), "12-ajoute-le-cache");
        assert_eq!(
            ticket_slug(3, "Créer l'écran Coût !"),
            "3-creer-l-ecran-cout"
        );
        assert_eq!(ticket_slug(7, "   "), "7");
        assert_eq!(ticket_slug(1, "///"), "1");
        let long = ticket_slug(99, &"mot ".repeat(40));
        assert!(long.len() <= 45, "slug trop long : {long}");
        assert!(!long.ends_with('-'));
    }
}
