//! The team's habits: conventions and architecture decisions.
//!
//! A role says *what* an agent is for; a rule says *how things are done here*.
//! Two kinds, one format — Markdown with a YAML header, like the roles:
//!
//! - a **convention** is a habit attached to roles: the pull request template
//!   the integrator always follows, the shape of a commit message, "a fixed bug
//!   comes with the test that would have caught it". Global ones live in
//!   `~/.config/orchestra/conventions`, and a project overrides any of them by
//!   name in `<projet>/.orchestra/conventions`;
//! - an **ADR** is an architecture decision a project has taken, which every
//!   agent of that project reads and respects. They only exist per project, in
//!   `<projet>/.orchestra/adr/NNNN-slug.md`.
//!
//! ```text
//! ---
//! title: Format des messages de commit
//! status: accepted
//! applies_to: []
//! checks:
//!   - commit_message: '^\[[a-z-]+\] \S'
//! ---
//! Chaque commit commence par ton rôle entre crochets…
//! ```
//!
//! Two things keep this honest. A rule an agent *proposed* is written with
//! `status: proposed` and is never handed to anyone until the user accepts it
//! — the orchestra proposes, the user decides. And what can be measured is not
//! asked: a convention may carry [`RuleCheck`]s, which the daemon evaluates on
//! the branch itself before any fusion. The daemon reads the facts (commit
//! messages, `PR.md`); this module only judges them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::model::RoleScope;
use crate::review::{normalize, strip_bullet};
use crate::roles::split_frontmatter;

/// How much of one rule's body an agent is given. Rules are meant to be short;
/// one that is not should not push the ticket itself out of the prompt.
const BODY_CAP: usize = 4000;

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    Convention,
    Adr,
}

impl RuleKind {
    pub fn label_fr(self) -> &'static str {
        match self {
            RuleKind::Convention => "convention",
            RuleKind::Adr => "ADR",
        }
    }

    /// The directory this kind lives in, under a config or `.orchestra` dir.
    pub fn dir_name(self) -> &'static str {
        match self {
            RuleKind::Convention => "conventions",
            RuleKind::Adr => "adr",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match normalize(s).as_str() {
            "convention" | "conventions" | "habitude" => Ok(RuleKind::Convention),
            "adr" | "decision" | "décision" => Ok(RuleKind::Adr),
            other => Err(CoreError::Parse(format!(
                "sorte de règle inconnue : {other} (convention ou adr)"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuleStatus {
    /// Written by an agent, waiting for the user. Never applied.
    Proposed,
    #[default]
    Accepted,
    /// Replaced by a later decision; kept for the history, not applied.
    Superseded,
    Rejected,
}

impl RuleStatus {
    pub const ALL: [RuleStatus; 4] = [
        RuleStatus::Proposed,
        RuleStatus::Accepted,
        RuleStatus::Superseded,
        RuleStatus::Rejected,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RuleStatus::Proposed => "proposed",
            RuleStatus::Accepted => "accepted",
            RuleStatus::Superseded => "superseded",
            RuleStatus::Rejected => "rejected",
        }
    }

    pub fn label_fr(self) -> &'static str {
        match self {
            RuleStatus::Proposed => "proposée",
            RuleStatus::Accepted => "acceptée",
            RuleStatus::Superseded => "remplacée",
            RuleStatus::Rejected => "rejetée",
        }
    }

    /// Only an accepted rule reaches an agent or gates a fusion.
    pub fn is_active(self) -> bool {
        matches!(self, RuleStatus::Accepted)
    }

    pub fn parse(s: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|v| v.as_str() == s.trim())
            .ok_or_else(|| CoreError::Parse(format!("statut de règle inconnu : {s}")))
    }
}

/// Which integrations a convention concerns. The pull request template means
/// nothing when the daemon fuses locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuleMode {
    #[default]
    Any,
    Pr,
    Merge,
}

impl RuleMode {
    pub fn matches(self, is_pr: bool) -> bool {
        match self {
            RuleMode::Any => true,
            RuleMode::Pr => is_pr,
            RuleMode::Merge => !is_pr,
        }
    }
}

/// Something the daemon can verify on the branch, without asking anyone.
///
/// Written `- commit_message: '…'` in a header: YAML has no convenient
/// spelling for a tagged enum, so it goes through [`RawCheck`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawCheck", into = "RawCheck")]
pub enum RuleCheck {
    /// Every commit's first line matches this regular expression.
    CommitMessage(String),
    /// `PR.md` has a heading for each of these sections. Only evaluated when a
    /// pull request is what comes out.
    PrSections(Vec<String>),
}

/// One check as it is written: exactly one of its keys.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCheck {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    commit_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pr_sections: Option<Vec<String>>,
}

impl TryFrom<RawCheck> for RuleCheck {
    type Error = String;

    fn try_from(raw: RawCheck) -> std::result::Result<Self, Self::Error> {
        match (raw.commit_message, raw.pr_sections) {
            (Some(p), None) => Ok(RuleCheck::CommitMessage(p)),
            (None, Some(s)) => Ok(RuleCheck::PrSections(s)),
            _ => Err("un check porte exactement une clé : commit_message ou pr_sections".into()),
        }
    }
}

impl From<RuleCheck> for RawCheck {
    fn from(c: RuleCheck) -> Self {
        match c {
            RuleCheck::CommitMessage(p) => RawCheck {
                commit_message: Some(p),
                ..Default::default()
            },
            RuleCheck::PrSections(s) => RawCheck {
                pr_sections: Some(s),
                ..Default::default()
            },
        }
    }
}

impl RuleCheck {
    pub fn label_fr(&self) -> String {
        match self {
            RuleCheck::CommitMessage(p) => format!("messages de commit conformes à « {p} »"),
            RuleCheck::PrSections(s) => format!("PR.md contient : {}", s.join(", ")),
        }
    }
}

/// One convention or decision, as loaded from its file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    /// File stem: `commits`, `0003-paginer-les-listes`. Unique per kind.
    pub name: String,
    pub kind: RuleKind,
    pub title: String,
    pub status: RuleStatus,
    /// Roles this convention is given to. Empty means everyone. Always empty
    /// for an ADR: a decision binds the whole team.
    #[serde(default)]
    pub applies_to: Vec<String>,
    #[serde(default)]
    pub mode: RuleMode,
    #[serde(default)]
    pub checks: Vec<RuleCheck>,
    /// The ADR this one replaces, by name.
    #[serde(default)]
    pub supersedes: Option<String>,
    /// `reviewer, ticket #12` when an agent proposed it.
    #[serde(default)]
    pub proposed_by: Option<String>,
    pub body: String,
    pub source: PathBuf,
    pub scope: RoleScope,
}

impl Rule {
    pub fn applies_to_role(&self, role: &str) -> bool {
        self.applies_to.is_empty() || self.applies_to.iter().any(|r| r == role)
    }

    /// `backend, tests` or `tous les rôles`.
    pub fn audience_fr(&self) -> String {
        if self.applies_to.is_empty() {
            "tous les rôles".into()
        } else {
            self.applies_to.join(", ")
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Fields accepted in the header. Strict, like a role's: these files are
/// written by people, and a typo must show rather than silently do nothing.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Frontmatter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    title: String,
    #[serde(default)]
    status: RuleStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    applies_to: Vec<String>,
    #[serde(default, skip_serializing_if = "is_any")]
    mode: RuleMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    checks: Vec<RuleCheck>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    supersedes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    proposed_by: Option<String>,
}

fn is_any(m: &RuleMode) -> bool {
    matches!(m, RuleMode::Any)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn parse_rule(path: &Path, src: &str, kind: RuleKind, scope: RoleScope) -> Result<Rule> {
    let (header, body) = split_frontmatter(src)?;
    let fm: Frontmatter = serde_yaml_ng::from_str(header)?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let name = fm
        .name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or(stem);
    if !valid_name(&name) {
        return Err(CoreError::Parse(format!(
            "nom de règle invalide « {name} » : lettres, chiffres, tiret et souligné seulement"
        )));
    }
    let title = fm.title.trim().to_string();
    if title.is_empty() {
        return Err(CoreError::Parse(format!("la règle « {name} » n'a pas de titre")));
    }
    let body = body.trim().to_string();
    if body.is_empty() {
        return Err(CoreError::Parse(format!(
            "la règle « {name} » n'a pas de texte sous son entête"
        )));
    }
    if kind == RuleKind::Adr && (!fm.applies_to.is_empty() || !fm.checks.is_empty()) {
        return Err(CoreError::Parse(format!(
            "l'ADR « {name} » vaut pour toute l'équipe : ni applies_to ni checks"
        )));
    }
    for check in &fm.checks {
        match check {
            RuleCheck::CommitMessage(pattern) => {
                regex::Regex::new(pattern).map_err(|e| {
                    CoreError::Parse(format!("motif invalide dans « {name} » : {e}"))
                })?;
            }
            RuleCheck::PrSections(sections) if sections.is_empty() => {
                return Err(CoreError::Parse(format!(
                    "« {name} » : pr_sections sans aucune section"
                )));
            }
            RuleCheck::PrSections(_) => {}
        }
    }
    Ok(Rule {
        name,
        kind,
        title,
        status: fm.status,
        applies_to: fm.applies_to,
        mode: fm.mode,
        checks: fm.checks,
        supersedes: fm.supersedes.filter(|s| !s.trim().is_empty()),
        proposed_by: fm.proposed_by.filter(|s| !s.trim().is_empty()),
        body,
        source: path.to_path_buf(),
        scope,
    })
}

/// Rules found in one directory, sorted by name. A file that fails to parse is
/// reported, never dropped: a broken rule the user believes is enforced is
/// worse than an error message.
pub fn load_dir(dir: &Path, kind: RuleKind, scope: RoleScope) -> (Vec<Rule>, Vec<(PathBuf, CoreError)>) {
    let mut rules = Vec::new();
    let mut errors = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (rules, errors);
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .filter(|p| {
            !p.file_name()
                .map(|n| n.to_string_lossy().starts_with('_'))
                .unwrap_or(true)
        })
        .collect();
    paths.sort();
    for path in paths {
        match std::fs::read_to_string(&path) {
            Ok(src) => match parse_rule(&path, &src, kind, scope) {
                Ok(rule) => rules.push(rule),
                Err(e) => errors.push((path, e)),
            },
            Err(e) => errors.push((
                path.clone(),
                CoreError::Parse(format!("lecture impossible : {e}")),
            )),
        }
    }
    rules.sort_by(|a, b| a.name.cmp(&b.name));
    (rules, errors)
}

// ---------------------------------------------------------------------------
// The rule book a ticket sees
// ---------------------------------------------------------------------------

/// Global conventions, the project's own overriding them by name, and the
/// project's decisions.
#[derive(Debug, Default)]
pub struct RuleBook {
    pub rules: Vec<Rule>,
    pub errors: Vec<(PathBuf, CoreError)>,
}

impl RuleBook {
    /// `global_dir` is the global conventions directory; `project_dir` the
    /// project's `.orchestra` directory, holding `conventions/` and `adr/`.
    pub fn load(global_dir: &Path, project_dir: Option<&Path>) -> Self {
        let (rules, errors) = load_dir(global_dir, RuleKind::Convention, RoleScope::Global);
        Self::with_project(rules, errors, project_dir)
    }

    /// Like [`RuleBook::load`], except that a global directory that does not
    /// exist yet — an install that predates conventions, `orchestra init` not
    /// rerun — falls back on the conventions shipped with the binary. An
    /// existing directory is the user's word, even when it is empty.
    pub fn load_or_bundled(
        global_dir: &Path,
        bundled: &[(&str, &str)],
        project_dir: Option<&Path>,
    ) -> Self {
        if global_dir.is_dir() {
            return Self::load(global_dir, project_dir);
        }
        let mut rules = Vec::new();
        let mut errors = Vec::new();
        for (file, src) in bundled {
            let path = global_dir.join(file);
            match parse_rule(&path, src, RuleKind::Convention, RoleScope::Global) {
                Ok(rule) => rules.push(rule),
                Err(e) => errors.push((path, e)),
            }
        }
        Self::with_project(rules, errors, project_dir)
    }

    fn with_project(
        mut rules: Vec<Rule>,
        mut errors: Vec<(PathBuf, CoreError)>,
        project_dir: Option<&Path>,
    ) -> Self {
        if let Some(dir) = project_dir {
            let (own, own_errors) = load_dir(
                &dir.join(RuleKind::Convention.dir_name()),
                RuleKind::Convention,
                RoleScope::Project,
            );
            errors.extend(own_errors);
            for rule in own {
                match rules.iter().position(|r| r.name == rule.name) {
                    Some(i) => rules[i] = rule,
                    None => rules.push(rule),
                }
            }
            let (adrs, adr_errors) = load_dir(
                &dir.join(RuleKind::Adr.dir_name()),
                RuleKind::Adr,
                RoleScope::Project,
            );
            errors.extend(adr_errors);
            rules.extend(adrs);
        }
        rules.sort_by(|a, b| (a.kind as u8, &a.name).cmp(&(b.kind as u8, &b.name)));
        RuleBook { rules, errors }
    }

    pub fn get(&self, kind: RuleKind, name: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.kind == kind && r.name == name)
    }

    pub fn pending(&self) -> usize {
        self.rules
            .iter()
            .filter(|r| r.status == RuleStatus::Proposed)
            .count()
    }

    /// What one role is handed: the accepted conventions meant for it in this
    /// mode, then every accepted decision.
    pub fn for_role(&self, role: &str, is_pr: bool) -> Vec<&Rule> {
        let conventions = self.rules.iter().filter(|r| {
            r.kind == RuleKind::Convention
                && r.status.is_active()
                && r.applies_to_role(role)
                && r.mode.matches(is_pr)
        });
        let decisions = self.adrs();
        conventions.chain(decisions).collect()
    }

    /// Accepted decisions, oldest first.
    pub fn adrs(&self) -> impl Iterator<Item = &Rule> {
        self.rules
            .iter()
            .filter(|r| r.kind == RuleKind::Adr && r.status.is_active())
    }

    /// Every measurable check that gates a fusion in this mode, with the rule
    /// it comes from. The branch is one thing: a check binds it whoever wrote
    /// the commits, so `applies_to` does not filter here.
    pub fn checks(&self, is_pr: bool) -> Vec<(&Rule, &RuleCheck)> {
        self.rules
            .iter()
            .filter(|r| r.kind == RuleKind::Convention && r.status.is_active() && r.mode.matches(is_pr))
            .flat_map(|r| r.checks.iter().map(move |c| (r, c)))
            .filter(|(_, c)| is_pr || !matches!(c, RuleCheck::PrSections(_)))
            .collect()
    }

    /// The Markdown appended to one role's instructions. Empty when nothing
    /// applies, so a project without rules costs no token.
    pub fn prompt_section(&self, role: &str, is_pr: bool) -> String {
        let rules = self.for_role(role, is_pr);
        let mut out = String::new();
        let conventions: Vec<&&Rule> = rules.iter().filter(|r| r.kind == RuleKind::Convention).collect();
        if !conventions.is_empty() {
            out.push_str(
                "\n\n## Conventions de l'équipe\n\n\
                 Ces habitudes valent pour toi, à chaque ticket. Celles marquées ⚙ sont \
                 vérifiées par le daemon sur la branche avant toute fusion : un écart \
                 renvoie l'intégration au travail.\n",
            );
            for r in conventions {
                let gear = if r.checks.is_empty() { "" } else { " ⚙" };
                out.push_str(&format!("\n### {}{gear}\n\n{}\n", r.title, cap(&r.body)));
            }
        }
        let decisions: Vec<&&Rule> = rules.iter().filter(|r| r.kind == RuleKind::Adr).collect();
        if !decisions.is_empty() {
            out.push_str(
                "\n\n## Décisions d'architecture du projet\n\n\
                 Ces décisions sont prises. Ton travail les respecte ; si l'une t'empêche \
                 d'avancer, dis-le dans ton résumé ou propose un nouvel ADR — ne la \
                 contourne pas en silence.\n",
            );
            for r in decisions {
                out.push_str(&format!("\n### {} — {}\n\n{}\n", r.name, r.title, cap(&r.body)));
            }
        }
        out
    }

    /// One line per accepted decision, for the orchestrator's planning prompt.
    pub fn adr_summary_lines(&self) -> String {
        self.adrs()
            .map(|r| format!("- {} — {}", r.name, r.title))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn cap(body: &str) -> String {
    if body.chars().count() <= BODY_CAP {
        return body.to_string();
    }
    let head: String = body.chars().take(BODY_CAP).collect();
    format!("{head}\n\n[… tronqué — le fichier complet est plus long]")
}

// ---------------------------------------------------------------------------
// Measured checks
// ---------------------------------------------------------------------------

/// What the daemon read on the branch, for the checks to judge.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchFacts {
    /// Full messages of the branch's own commits, merges excluded.
    pub commits: Vec<String>,
    /// `PR.md`, when the integrator wrote one.
    pub pr_body: Option<String>,
}

/// One way the branch breaks a rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    pub rule: String,
    pub detail: String,
}

impl Violation {
    pub fn line(&self) -> String {
        format!("{} : {}", self.rule, self.detail)
    }
}

/// How many offending commits are named before the rest is summed up: the
/// integrator needs examples, not the whole log.
const COMMITS_NAMED: usize = 5;

/// Judge one check against the facts.
pub fn evaluate(rule: &Rule, check: &RuleCheck, facts: &BranchFacts) -> Vec<Violation> {
    let violation = |detail: String| Violation {
        rule: rule.name.clone(),
        detail,
    };
    match check {
        RuleCheck::CommitMessage(pattern) => {
            // Validated at load time; a pattern that fails here is one that
            // could not be read, and must not pass silently either.
            let Ok(re) = regex::Regex::new(pattern) else {
                return vec![violation(format!("motif illisible « {pattern} »"))];
            };
            let bad: Vec<&str> = facts
                .commits
                .iter()
                .map(|m| m.lines().next().unwrap_or("").trim())
                .filter(|subject| !re.is_match(subject))
                .collect();
            if bad.is_empty() {
                return Vec::new();
            }
            let mut out: Vec<Violation> = bad
                .iter()
                .take(COMMITS_NAMED)
                .map(|s| violation(format!("le commit « {s} » ne suit pas « {pattern} »")))
                .collect();
            if bad.len() > COMMITS_NAMED {
                out.push(violation(format!(
                    "et {} autre(s) commit(s) dans le même cas",
                    bad.len() - COMMITS_NAMED
                )));
            }
            out
        }
        RuleCheck::PrSections(sections) => {
            let Some(body) = &facts.pr_body else {
                return vec![violation("PR.md est absent ou vide".into())];
            };
            let headings: Vec<String> = body
                .lines()
                .filter(|l| l.trim_start().starts_with('#'))
                .map(|l| normalize(l.trim_start().trim_start_matches('#')))
                .collect();
            sections
                .iter()
                .filter(|s| !headings.iter().any(|h| h == &normalize(s)))
                .map(|s| violation(format!("PR.md n'a pas de section « {s} »")))
                .collect()
        }
    }
}

/// Every violation of every active check, in the order the rules are listed.
pub fn evaluate_all(book: &RuleBook, is_pr: bool, facts: &BranchFacts) -> Vec<Violation> {
    book.checks(is_pr)
        .into_iter()
        .flat_map(|(rule, check)| evaluate(rule, check, facts))
        .collect()
}

// ---------------------------------------------------------------------------
// Proposals written by agents
// ---------------------------------------------------------------------------

/// A rule an agent suggested at the end of its message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedRule {
    pub kind: RuleKind,
    pub title: String,
    #[serde(default)]
    pub applies_to: Vec<String>,
    pub body: String,
}

/// `PROPOSITION: convention`, if this line opens a proposal.
fn proposal_on_line(line: &str) -> Option<RuleKind> {
    let cleaned = line.trim().replace(['*', '#', '_', '`'], "");
    let cleaned = strip_bullet(&cleaned).unwrap_or(cleaned.trim()).to_string();
    let (head, tail) = cleaned.split_once([':', '：'])?;
    if normalize(head) != "proposition" {
        return None;
    }
    let word = tail.split_whitespace().next()?;
    RuleKind::parse(word.trim_matches(|c: char| !c.is_alphanumeric())).ok()
}

/// A `key: value` line whose key is one of `keys`.
fn field<'a>(line: &'a str, keys: &[&str]) -> Option<&'a str> {
    let cleaned = line.trim().trim_start_matches(['*', '-', ' ']);
    let (head, tail) = cleaned.split_once([':', '：'])?;
    let head = normalize(head.trim_matches(|c: char| c == '*' || c == '`'));
    keys.contains(&head.as_str()).then(|| tail.trim())
}

/// Ends a proposal's body: the next proposal, the reviewer's verdict, or an
/// explicit end marker.
fn ends_block(line: &str) -> bool {
    let n = normalize(&line.trim().replace(['*', '#', '_', '`'], ""));
    n.starts_with("verdict") && n.contains(':')
        || n == "fin proposition"
        || n == "fin de la proposition"
        || proposal_on_line(line).is_some()
}

/// Read the proposals out of an agent's closing message.
///
/// Forgiving about the form — accents, bold markers, a code fence around the
/// block — and strict about the substance: a proposal without a title or
/// without a body is not one, and nothing is guessed to fill the gap.
pub fn parse_proposals(text: &str) -> Vec<ProposedRule> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(kind) = proposal_on_line(lines[i]) else {
            i += 1;
            continue;
        };
        i += 1;
        let mut title = None;
        let mut applies_to = Vec::new();
        let mut body: Vec<&str> = Vec::new();
        while i < lines.len() && !ends_block(lines[i]) {
            let line = lines[i];
            i += 1;
            if line.trim_start().starts_with("```") {
                continue;
            }
            if body.is_empty() {
                if title.is_none() {
                    if let Some(t) = field(line, &["titre", "title"]) {
                        title = Some(t.to_string());
                        continue;
                    }
                }
                if let Some(r) = field(line, &["roles", "role", "applies_to"]) {
                    applies_to = r
                        .split([',', ' '])
                        .map(|s| s.trim().trim_matches('`'))
                        .filter(|s| !s.is_empty() && *s != "tous")
                        .map(str::to_string)
                        .collect();
                    continue;
                }
                if line.trim().is_empty() {
                    continue;
                }
            }
            body.push(line);
        }
        // Consume an explicit end marker so it does not start anything.
        if i < lines.len() && proposal_on_line(lines[i]).is_none() && !normalize(lines[i]).starts_with("verdict") {
            i += 1;
        }
        let body = body.join("\n").trim().to_string();
        let Some(title) = title.filter(|t| !t.is_empty()) else {
            continue;
        };
        if body.is_empty() {
            continue;
        }
        if kind == RuleKind::Adr {
            applies_to.clear();
        }
        out.push(ProposedRule {
            kind,
            title,
            applies_to,
            body,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Writing rule files
// ---------------------------------------------------------------------------

/// `paginer-les-listes-d-api`, from a title.
pub fn slug(title: &str) -> String {
    // `ticket_slug` already knows French accents and branch-safe characters;
    // its number prefix is the only part not wanted here.
    let with_number = crate::model::ticket_slug(0, &title.to_lowercase());
    let s = with_number.strip_prefix("0-").unwrap_or(&with_number);
    if s.is_empty() || s == "0" {
        "regle".into()
    } else {
        s.to_string()
    }
}

/// The next free `NNNN-slug` among existing ADR names.
pub fn next_adr_name<'a>(existing: impl IntoIterator<Item = &'a str>, title: &str) -> String {
    let last = existing
        .into_iter()
        .filter_map(|n| n.split('-').next()?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    format!("{:04}-{}", last + 1, slug(title))
}

/// A new rule file, ready to write.
pub struct RuleDraft<'a> {
    pub kind: RuleKind,
    pub title: &'a str,
    pub status: RuleStatus,
    pub applies_to: &'a [String],
    pub proposed_by: Option<&'a str>,
    pub body: &'a str,
}

pub fn render(draft: &RuleDraft<'_>) -> Result<String> {
    let fm = Frontmatter {
        name: None,
        title: draft.title.trim().to_string(),
        status: draft.status,
        applies_to: draft.applies_to.to_vec(),
        mode: RuleMode::Any,
        checks: Vec::new(),
        supersedes: None,
        proposed_by: draft.proposed_by.map(str::to_string),
    };
    let header = serde_yaml_ng::to_string(&fm)?;
    Ok(format!("---\n{}---\n{}\n", header, draft.body.trim()))
}

/// The body a hand-created rule starts with.
pub fn skeleton_body(kind: RuleKind) -> &'static str {
    match kind {
        RuleKind::Convention => {
            "Ce que l'agent doit faire, à l'impératif, en quelques lignes.\n\n\
             Pour la faire vérifier par le daemon, ajoute dans l'entête par exemple :\n\
             `checks: [{commit_message: '^\\[[a-z-]+\\] '}]`\n\
             et, pour la limiter à des rôles : `applies_to: [integrator]`."
        }
        RuleKind::Adr => {
            "## Contexte\n\nCe qui oblige à choisir.\n\n\
             ## Décision\n\nCe qui est retenu, en une phrase.\n\n\
             ## Conséquences\n\nCe que ça rend facile, ce que ça rend difficile."
        }
    }
}

/// Change the `status:` line of a rule file, keeping everything else — the
/// user's comments and layout included — exactly as it was.
pub fn with_status(src: &str, status: RuleStatus) -> Result<String> {
    crate::roles::set_header_field(src, "status", status.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMITS: &str = r#"---
title: Format des messages de commit
checks:
  - commit_message: '^\[[a-z-]+\] \S'
---
Chaque commit commence par ton rôle entre crochets.
"#;

    const PR: &str = r#"---
title: Squelette de pull request
applies_to: [integrator]
mode: pr
checks:
  - pr_sections: ["Ce que ça change", "Pourquoi"]
---
Écris PR.md.
"#;

    fn rule(name: &str, src: &str, kind: RuleKind) -> Rule {
        parse_rule(Path::new(&format!("/x/{name}.md")), src, kind, RoleScope::Global).unwrap()
    }

    fn book(rules: Vec<Rule>) -> RuleBook {
        RuleBook {
            rules,
            errors: Vec::new(),
        }
    }

    #[test]
    fn a_rule_takes_its_name_from_its_file_and_defaults_to_accepted() {
        let r = rule("commits", COMMITS, RuleKind::Convention);
        assert_eq!(r.name, "commits");
        assert_eq!(r.status, RuleStatus::Accepted);
        assert!(r.applies_to_role("backend"));
        assert_eq!(r.checks.len(), 1);
        let pr = rule("pr", PR, RuleKind::Convention);
        assert!(!pr.applies_to_role("backend"));
        assert!(pr.applies_to_role("integrator"));
        assert_eq!(pr.mode, RuleMode::Pr);
    }

    #[test]
    fn broken_rules_are_refused_with_a_reason() {
        let p = Path::new("/x/r.md");
        let bad_regex = "---\ntitle: t\nchecks:\n  - commit_message: '(['\n---\ncorps\n";
        assert!(parse_rule(p, bad_regex, RuleKind::Convention, RoleScope::Global).is_err());
        let no_title = "---\nstatus: accepted\n---\ncorps\n";
        assert!(parse_rule(p, no_title, RuleKind::Convention, RoleScope::Global).is_err());
        let no_body = "---\ntitle: t\n---\n\n";
        assert!(parse_rule(p, no_body, RuleKind::Convention, RoleScope::Global).is_err());
        let typo = "---\ntitle: t\nstauts: accepted\n---\ncorps\n";
        assert!(parse_rule(p, typo, RuleKind::Convention, RoleScope::Global).is_err());
        let adr_with_checks = "---\ntitle: t\nchecks:\n  - commit_message: 'x'\n---\ncorps\n";
        assert!(parse_rule(p, adr_with_checks, RuleKind::Adr, RoleScope::Project).is_err());
    }

    #[test]
    fn a_project_overrides_a_global_convention_by_name_and_adds_its_decisions() {
        let global = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::write(global.path().join("commits.md"), COMMITS).unwrap();
        std::fs::write(global.path().join("pr.md"), PR).unwrap();
        std::fs::create_dir_all(project.path().join("conventions")).unwrap();
        std::fs::create_dir_all(project.path().join("adr")).unwrap();
        std::fs::write(
            project.path().join("conventions/commits.md"),
            "---\ntitle: Commits du projet\nstatus: rejected\n---\nautre chose\n",
        )
        .unwrap();
        std::fs::write(
            project.path().join("adr/0001-sqlite.md"),
            "---\ntitle: SQLite, pas Postgres\n---\n## Décision\n\nSQLite.\n",
        )
        .unwrap();
        std::fs::write(project.path().join("adr/0002-casse.md"), "pas d'entête").unwrap();

        let b = RuleBook::load(global.path(), Some(project.path()));
        assert_eq!(b.errors.len(), 1, "{:?}", b.errors);
        let commits = b.get(RuleKind::Convention, "commits").unwrap();
        assert_eq!(commits.scope, RoleScope::Project);
        assert_eq!(commits.status, RuleStatus::Rejected);
        assert!(b.get(RuleKind::Adr, "0001-sqlite").is_some());

        // The rejected override switches the global one off for this project.
        let backend = b.for_role("backend", true);
        assert!(backend.iter().all(|r| r.name != "commits"));
        assert!(backend.iter().any(|r| r.name == "0001-sqlite"));
        assert!(b.checks(true).iter().all(|(r, _)| r.name != "commits"));
    }

    #[test]
    fn a_missing_global_directory_falls_back_on_the_shipped_conventions() {
        let root = tempfile::tempdir().unwrap();
        let absent = root.path().join("conventions");
        let bundled = [("commits.md", COMMITS)];
        let b = RuleBook::load_or_bundled(&absent, &bundled, None);
        assert!(b.get(RuleKind::Convention, "commits").is_some());
        // Once it exists, even empty, it is what the user wants.
        std::fs::create_dir_all(&absent).unwrap();
        let b = RuleBook::load_or_bundled(&absent, &bundled, None);
        assert!(b.rules.is_empty());
    }

    #[test]
    fn only_accepted_rules_reach_an_agent() {
        let mut proposed = rule("commits", COMMITS, RuleKind::Convention);
        proposed.status = RuleStatus::Proposed;
        let b = book(vec![proposed]);
        assert!(b.for_role("backend", false).is_empty());
        assert!(b.prompt_section("backend", false).is_empty());
        assert!(b.checks(false).is_empty());
        assert_eq!(b.pending(), 1);
    }

    #[test]
    fn each_role_gets_its_own_conventions_and_the_mode_filters_the_template() {
        let b = book(vec![
            rule("commits", COMMITS, RuleKind::Convention),
            rule("pr", PR, RuleKind::Convention),
        ]);
        let integrator_pr = b.prompt_section("integrator", true);
        assert!(integrator_pr.contains("Squelette de pull request ⚙"), "{integrator_pr}");
        assert!(integrator_pr.contains("Format des messages de commit"));
        assert!(!b.prompt_section("integrator", false).contains("Squelette"));
        assert!(!b.prompt_section("backend", true).contains("Squelette"));
        assert!(b.checks(false).iter().all(|(_, c)| !matches!(c, RuleCheck::PrSections(_))));
        assert_eq!(b.checks(true).len(), 2);
    }

    #[test]
    fn commit_messages_are_checked_on_their_first_line() {
        let r = rule("commits", COMMITS, RuleKind::Convention);
        let facts = BranchFacts {
            commits: vec![
                "[backend] ajoute le cache\n\ncorps libre".into(),
                "wip".into(),
                "Ajoute un test".into(),
            ],
            pr_body: None,
        };
        let v = evaluate(&r, &r.checks[0], &facts);
        assert_eq!(v.len(), 2, "{v:?}");
        assert!(v[0].detail.contains("« wip »"));
        assert_eq!(v[0].rule, "commits");
        let clean = BranchFacts {
            commits: vec!["[tests] couvre la liste vide".into()],
            pr_body: None,
        };
        assert!(evaluate(&r, &r.checks[0], &clean).is_empty());
    }

    #[test]
    fn many_bad_commits_are_summed_up_not_listed() {
        let r = rule("commits", COMMITS, RuleKind::Convention);
        let facts = BranchFacts {
            commits: (0..9).map(|i| format!("wip {i}")).collect(),
            pr_body: None,
        };
        let v = evaluate(&r, &r.checks[0], &facts);
        assert_eq!(v.len(), COMMITS_NAMED + 1);
        assert!(v.last().unwrap().detail.contains("4 autre(s)"));
    }

    #[test]
    fn pr_sections_ignore_case_accents_and_heading_level() {
        let r = rule("pr", PR, RuleKind::Convention);
        let ok = BranchFacts {
            commits: vec![],
            pr_body: Some("## ce que ca change\n\nx\n\n### POURQUOI\n\ny".into()),
        };
        assert!(evaluate(&r, &r.checks[0], &ok).is_empty());
        let partial = BranchFacts {
            commits: vec![],
            pr_body: Some("## Pourquoi\n\ny".into()),
        };
        let v = evaluate(&r, &r.checks[0], &partial);
        assert_eq!(v.len(), 1);
        assert!(v[0].detail.contains("Ce que ça change"));
        let missing = BranchFacts::default();
        assert!(evaluate(&r, &r.checks[0], &missing)[0].detail.contains("absent"));
    }

    #[test]
    fn proposals_are_read_before_the_verdict_and_never_guessed() {
        let text = include_str!("../tests/fixtures/proposals.md");
        let p = parse_proposals(text);
        assert_eq!(p.len(), 2, "{p:?}");
        assert_eq!(p[0].kind, RuleKind::Convention);
        assert_eq!(p[0].title, "Toujours paginer les listes d'API");
        assert_eq!(p[0].applies_to, vec!["backend".to_string()]);
        assert!(p[0].body.contains("limit"));
        assert!(!p[0].body.contains("VERDICT"));
        assert_eq!(p[1].kind, RuleKind::Adr);
        assert!(p[1].applies_to.is_empty());
        assert!(p[1].body.contains("## Décision"));
        // The verdict itself is untouched by the proposals around it.
        assert!(crate::review::parse_review(text).is_some());
    }

    #[test]
    fn a_proposal_without_title_or_body_is_dropped() {
        assert!(parse_proposals("PROPOSITION: convention\nsans titre ici\n").is_empty());
        assert!(parse_proposals("PROPOSITION: adr\ntitre: T\n").is_empty());
        assert!(parse_proposals("PROPOSITION: recette\ntitre: T\ncorps\n").is_empty());
        let p = parse_proposals("**Proposition : Convention**\nTitre : T\ncorps\n");
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn rendered_files_parse_back() {
        let roles = vec!["backend".to_string()];
        let src = render(&RuleDraft {
            kind: RuleKind::Convention,
            title: "Toujours paginer",
            status: RuleStatus::Proposed,
            applies_to: &roles,
            proposed_by: Some("reviewer, ticket #3"),
            body: "Utilise limit et offset.",
        })
        .unwrap();
        let r = parse_rule(Path::new("/x/toujours-paginer.md"), &src, RuleKind::Convention, RoleScope::Project).unwrap();
        assert_eq!(r.status, RuleStatus::Proposed);
        assert_eq!(r.applies_to, roles);
        assert_eq!(r.proposed_by.as_deref(), Some("reviewer, ticket #3"));
        let skeleton = render(&RuleDraft {
            kind: RuleKind::Adr,
            title: "Titre",
            status: RuleStatus::Accepted,
            applies_to: &[],
            proposed_by: None,
            body: skeleton_body(RuleKind::Adr),
        })
        .unwrap();
        assert!(parse_rule(Path::new("/x/0001-titre.md"), &skeleton, RuleKind::Adr, RoleScope::Project).is_ok());
    }

    #[test]
    fn changing_a_status_keeps_the_rest_of_the_file() {
        let src = "---\n# à moi\ntitle: T\nstatus: proposed\nproposed_by: x\n---\ncorps\n";
        let out = with_status(src, RuleStatus::Accepted).unwrap();
        assert_eq!(out, "---\n# à moi\ntitle: T\nstatus: accepted\nproposed_by: x\n---\ncorps\n");
        let without = "---\ntitle: T\n---\ncorps\n";
        let out = with_status(without, RuleStatus::Rejected).unwrap();
        let r = parse_rule(Path::new("/x/t.md"), &out, RuleKind::Convention, RoleScope::Global).unwrap();
        assert_eq!(r.status, RuleStatus::Rejected);
        assert_eq!(r.body, "corps");
    }

    #[test]
    fn adr_names_count_up_and_slugs_are_file_safe() {
        assert_eq!(slug("Toujours paginer les listes d'API"), "toujours-paginer-les-listes-d-api");
        assert_eq!(slug("!!!"), "regle");
        assert_eq!(next_adr_name(["0001-a", "0007-b", "notes"], "Écrire en SQL"), "0008-ecrire-en-sql");
        assert_eq!(next_adr_name([], "X"), "0001-x");
    }

    #[test]
    fn statuses_and_kinds_round_trip() {
        for s in RuleStatus::ALL {
            assert_eq!(RuleStatus::parse(s.as_str()).unwrap(), s);
        }
        assert_eq!(RuleKind::parse("ADR").unwrap(), RuleKind::Adr);
        assert_eq!(RuleKind::parse("décision").unwrap(), RuleKind::Adr);
        assert!(RuleKind::parse("recette").is_err());
    }
}
