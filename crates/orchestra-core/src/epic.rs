//! Epics: one request too large for a single team, split by the
//! orchestrator into tickets that each merge on their own, in order.
//!
//! The orchestrator proposes the split, the user edits and accepts it, and
//! only then do the tickets exist (rule 9). Each ticket keeps its own team,
//! planned when its turn comes. Dependencies are between tickets of the same
//! epic: a ticket waits until the ones it depends on are merged, and starts
//! from the default branch that now holds them.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{CoreError, Result};
use crate::model::{ProjectId, TicketId};

pub type EpicId = Uuid;

/// More than this and the request is a roadmap, not an epic.
pub const MAX_EPIC_TICKETS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpicStatus {
    /// Created; the split is being asked for, or failed.
    Draft,
    /// A split waits to be read.
    Split,
    /// Its tickets exist and are being worked through.
    Active,
    Done,
    Cancelled,
}

impl EpicStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            EpicStatus::Draft => "draft",
            EpicStatus::Split => "split",
            EpicStatus::Active => "active",
            EpicStatus::Done => "done",
            EpicStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "draft" => EpicStatus::Draft,
            "split" => EpicStatus::Split,
            "active" => EpicStatus::Active,
            "done" => EpicStatus::Done,
            "cancelled" => EpicStatus::Cancelled,
            _ => return None,
        })
    }

    pub fn label_fr(self) -> &'static str {
        match self {
            EpicStatus::Draft => "à découper",
            EpicStatus::Split => "découpage à relire",
            EpicStatus::Active => "en cours",
            EpicStatus::Done => "terminée",
            EpicStatus::Cancelled => "annulée",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Epic {
    pub id: EpicId,
    pub project_id: ProjectId,
    pub title: String,
    pub brief: String,
    pub status: EpicStatus,
    /// The last split the orchestrator proposed, edited or not.
    #[serde(default)]
    pub proposal: Option<EpicProposal>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// One ticket of a proposed split.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpicTicket {
    pub title: String,
    pub brief: String,
    /// Positions, in this proposal, of the tickets that must be merged first.
    #[serde(default)]
    pub depends_on: Vec<usize>,
    #[serde(default)]
    pub acceptance: Vec<String>,
}

impl EpicTicket {
    /// The brief the ticket is created with: its own, then what done means.
    pub fn ticket_brief(&self) -> String {
        let mut out = self.brief.trim().to_string();
        if !self.acceptance.is_empty() {
            out.push_str("\n\nCritères d'acceptation :");
            for c in &self.acceptance {
                out.push_str(&format!("\n- {}", c.trim()));
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpicProposal {
    /// How the orchestrator understood the request, in two or three lines.
    pub summary: String,
    pub tickets: Vec<EpicTicket>,
}

impl EpicProposal {
    /// Check the split, and return it in waves: tickets of one wave depend
    /// only on earlier waves.
    pub fn waves(&self) -> Result<Vec<Vec<usize>>> {
        let n = self.tickets.len();
        if n == 0 {
            return Err(CoreError::Parse("le découpage ne contient aucun ticket".into()));
        }
        if n > MAX_EPIC_TICKETS {
            return Err(CoreError::Parse(format!(
                "{n} tickets : au-delà de {MAX_EPIC_TICKETS}, c'est une feuille de route, pas une épopée"
            )));
        }
        for (i, t) in self.tickets.iter().enumerate() {
            if t.title.trim().is_empty() {
                return Err(CoreError::Parse(format!("le ticket {} n'a pas de titre", i + 1)));
            }
            for &d in &t.depends_on {
                if d >= n {
                    return Err(CoreError::Parse(format!(
                        "« {} » dépend du ticket {}, qui n'existe pas",
                        t.title,
                        d + 1
                    )));
                }
                if d == i {
                    return Err(CoreError::CyclicDependencies(t.title.clone()));
                }
            }
        }
        let mut done = vec![false; n];
        let mut waves = Vec::new();
        while done.iter().any(|d| !d) {
            let ready: Vec<usize> = (0..n)
                .filter(|&i| !done[i] && self.tickets[i].depends_on.iter().all(|&d| done[d]))
                .collect();
            if ready.is_empty() {
                let stuck: Vec<&str> = (0..n)
                    .filter(|&i| !done[i])
                    .map(|i| self.tickets[i].title.as_str())
                    .collect();
                return Err(CoreError::CyclicDependencies(stuck.join(", ")));
            }
            for &i in &ready {
                done[i] = true;
            }
            waves.push(ready);
        }
        Ok(waves)
    }

    /// Drop the ticket at `index`, and every dependency on it; later
    /// positions shift down by one.
    pub fn remove(&mut self, index: usize) {
        if index >= self.tickets.len() {
            return;
        }
        self.tickets.remove(index);
        for t in &mut self.tickets {
            t.depends_on.retain(|&d| d != index);
            for d in &mut t.depends_on {
                if *d > index {
                    *d -= 1;
                }
            }
        }
    }
}

/// Where a ticket sits in its epic, as the board shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpicLink {
    pub epic_id: EpicId,
    pub epic_title: String,
    /// Numbers of the tickets it waits for, not merged yet. Empty: free to go.
    #[serde(default)]
    pub waiting_on: Vec<i64>,
}

/// The tickets of an epic and what each depends on, as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpicMember {
    pub ticket_id: TicketId,
    pub position: u32,
    #[serde(default)]
    pub depends_on: Vec<TicketId>,
}

/// The schema handed to the orchestrator with `--json-schema` for a split.
pub fn epic_proposal_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["summary", "tickets"],
        "properties": {
            "summary": {
                "type": "string",
                "description": "En deux ou trois phrases, ce que la demande couvre et comment tu l'as découpée."
            },
            "tickets": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_EPIC_TICKETS,
                "description": "Des tickets livrables et fusionnables un par un, dans l'ordre où ils peuvent se faire.",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["title", "brief", "acceptance"],
                    "properties": {
                        "title": { "type": "string", "description": "Court, à l'impératif." },
                        "brief": {
                            "type": "string",
                            "description": "Ce que ce ticket doit livrer, seul, avec les fichiers ou modules concernés."
                        },
                        "depends_on": {
                            "type": "array",
                            "items": { "type": "integer", "minimum": 0 },
                            "description": "Positions (à partir de 0) des tickets de cette liste qui doivent être fusionnés avant celui-ci."
                        },
                        "acceptance": {
                            "type": "array",
                            "minItems": 1,
                            "items": { "type": "string" },
                            "description": "Ce qui doit être vrai une fois ce ticket fusionné, vérifiable."
                        }
                    }
                }
            }
        }
    })
}

/// Read the split out of a `result` line: `structured_output` first, the
/// text as a fallback.
pub fn proposal_from_result(structured: Option<&Value>, text: Option<&str>) -> Result<EpicProposal> {
    if let Some(value) = structured {
        return serde_json::from_value(value.clone())
            .map_err(|e| CoreError::Parse(format!("découpage illisible : {e}")));
    }
    let text = text.ok_or_else(|| CoreError::Parse("l'orchestrateur n'a rien renvoyé".into()))?;
    let json = crate::schema::extract_json_object(text)
        .ok_or_else(|| CoreError::Parse("aucun objet JSON dans le découpage".into()))?;
    serde_json::from_str(json).map_err(|e| CoreError::Parse(format!("découpage illisible : {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(title: &str, deps: &[usize]) -> EpicTicket {
        EpicTicket {
            title: title.into(),
            brief: format!("faire {title}"),
            depends_on: deps.to_vec(),
            acceptance: vec![format!("{title} existe")],
        }
    }

    fn split(tickets: Vec<EpicTicket>) -> EpicProposal {
        EpicProposal { summary: "s".into(), tickets }
    }

    #[test]
    fn a_split_comes_back_in_waves() {
        let p = split(vec![t("schéma", &[]), t("api", &[0]), t("écran", &[1]), t("docs", &[0])]);
        assert_eq!(p.waves().unwrap(), vec![vec![0], vec![1, 3], vec![2]]);
    }

    #[test]
    fn a_split_that_cannot_be_ordered_is_refused() {
        assert!(split(vec![]).waves().is_err());
        assert!(split(vec![t("a", &[1]), t("b", &[0])]).waves().is_err(), "cycle");
        assert!(split(vec![t("a", &[0])]).waves().is_err(), "sur lui-même");
        assert!(split(vec![t("a", &[5])]).waves().is_err(), "hors bornes");
        assert!(split((0..9).map(|i| t(&format!("t{i}"), &[])).collect()).waves().is_err());
        assert!(split(vec![t("  ", &[])]).waves().is_err(), "sans titre");
    }

    #[test]
    fn removing_a_ticket_keeps_the_others_pointing_at_the_right_ones() {
        let mut p = split(vec![t("a", &[]), t("b", &[0]), t("c", &[0, 1])]);
        p.remove(1);
        assert_eq!(p.tickets.len(), 2);
        assert_eq!(p.tickets[1].title, "c");
        assert_eq!(p.tickets[1].depends_on, vec![0], "b retiré, a reste en position 0");
        assert!(p.waves().is_ok());
    }

    #[test]
    fn the_criteria_travel_in_the_ticket_brief() {
        let brief = t("api", &[]).ticket_brief();
        assert!(brief.starts_with("faire api"));
        assert!(brief.contains("Critères d'acceptation :\n- api existe"));
    }

    #[test]
    fn a_structured_split_is_read() {
        let v = serde_json::json!({
            "summary": "trois temps",
            "tickets": [{"title": "a", "brief": "b", "acceptance": ["c"]},
                        {"title": "d", "brief": "e", "depends_on": [0], "acceptance": ["f"]}]
        });
        let p = proposal_from_result(Some(&v), None).unwrap();
        assert_eq!(p.tickets[1].depends_on, vec![0]);
        let schema = epic_proposal_schema();
        assert_eq!(schema["properties"]["tickets"]["maxItems"], serde_json::json!(MAX_EPIC_TICKETS));
    }
}
