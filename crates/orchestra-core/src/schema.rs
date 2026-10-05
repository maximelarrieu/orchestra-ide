//! The JSON schema handed to `claude --json-schema` for a team proposal.
//!
//! The schema is built from the catalog rather than written once: the `role`
//! field carries an `enum` of the roles that actually exist. Without it the
//! model invents plausible job titles instead of picking from the catalog,
//! which was observed on the first try ("Backend Engineer" rather than
//! `backend`). The reply is validated again on arrival, since a schema is a
//! strong hint and not a guarantee.

use serde_json::{json, Value};

use crate::error::{CoreError, Result};
use crate::model::TeamProposal;

/// Upper bound on the team size, to keep the orchestrator from designing a
/// committee. Also the value quoted in the prompt.
pub const MAX_TEAM: usize = 6;

/// The model aliases the orchestrator may pick for a member. Aliases, not
/// ids: the CLI resolves them, and the cost is priced from the model the
/// samples report (rule 8), never from this name.
pub const MODEL_TIERS: [&str; 3] = ["haiku", "sonnet", "opus"];

/// Build the schema for a proposal limited to `roles`.
pub fn team_proposal_schema(roles: &[String]) -> Value {
    let role_field = if roles.is_empty() {
        json!({ "type": "string" })
    } else {
        json!({ "type": "string", "enum": roles })
    };

    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["summary", "members", "estimated_size"],
        "properties": {
            "summary": {
                "type": "string",
                "description": "En deux ou trois phrases, ce que la feature demande et comment tu l'as comprise."
            },
            "estimated_size": {
                "type": "string",
                "enum": ["xs", "s", "m", "l", "xl"],
                "description": "Ampleur du travail."
            },
            "risks": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Points d'attention, pièges, zones du dépôt à manier avec précaution."
            },
            "members": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_TEAM,
                "description": "La plus petite équipe capable de livrer la feature.",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["role", "objective", "acceptance"],
                    "properties": {
                        "role": role_field,
                        "objective": {
                            "type": "string",
                            "description": "Ce que ce rôle doit accomplir pour CE ticket, concrètement."
                        },
                        "acceptance": {
                            "type": "array",
                            "minItems": 1,
                            "items": { "type": "string" },
                            "description": "Ce qui doit être vrai quand ce rôle a fini : des faits qu'un relecteur peut vérifier (un test qui passe, un fichier qui existe, un comportement observable), pas des intentions."
                        },
                        "model": {
                            "type": "string",
                            "enum": MODEL_TIERS,
                            "description": "À omettre pour garder le modèle du rôle. haiku pour une tâche mécanique et bien bornée, opus pour une tâche délicate ou risquée."
                        },
                        "depends_on": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Rôles de cette équipe dont le travail doit être terminé avant celui-ci."
                        },
                        "parallel_ok": {
                            "type": "boolean",
                            "description": "Ce rôle peut travailler en même temps que ses pairs sans conflit."
                        }
                    }
                }
            }
        }
    })
}

/// Read the proposal out of a `result` line.
///
/// `structured_output` holds the parsed object and `result` the same JSON as a
/// string. The first is used when present, the second is the fallback, which
/// also covers a model that answered in prose around its JSON.
pub fn proposal_from_result(
    structured_output: Option<&Value>,
    result_text: Option<&str>,
) -> Result<TeamProposal> {
    if let Some(value) = structured_output {
        return serde_json::from_value(value.clone())
            .map_err(|e| CoreError::Parse(format!("proposition d'équipe illisible : {e}")));
    }
    let text = result_text
        .ok_or_else(|| CoreError::Parse("l'orchestrateur n'a rien renvoyé d'exploitable".into()))?;
    let json = extract_json_object(text).ok_or_else(|| {
        CoreError::Parse(format!(
            "aucun objet JSON dans la réponse de l'orchestrateur : {}",
            crate::claude::stream::truncate(text, 200)
        ))
    })?;
    serde_json::from_str(json)
        .map_err(|e| CoreError::Parse(format!("proposition d'équipe illisible : {e}")))
}

/// First balanced `{…}` block, ignoring braces inside strings.
pub(crate) fn extract_json_object(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Size;

    fn roles() -> Vec<String> {
        vec!["architect".into(), "backend".into(), "tests".into()]
    }

    #[test]
    fn the_schema_restricts_roles_to_the_catalog() {
        let schema = team_proposal_schema(&roles());
        let role = &schema["properties"]["members"]["items"]["properties"]["role"];
        assert_eq!(
            role["enum"],
            json!(["architect", "backend", "tests"]),
            "sans énumération le modèle invente des intitulés"
        );
        assert_eq!(schema["properties"]["members"]["maxItems"], json!(MAX_TEAM));
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[test]
    fn every_member_comes_with_checkable_acceptance_criteria() {
        let schema = team_proposal_schema(&roles());
        let member = &schema["properties"]["members"]["items"];
        assert!(member["required"]
            .as_array()
            .unwrap()
            .contains(&json!("acceptance")));
        assert_eq!(member["properties"]["acceptance"]["minItems"], json!(1));
        // The model is optional and limited to aliases the CLI knows.
        assert!(!member["required"].as_array().unwrap().contains(&json!("model")));
        assert_eq!(member["properties"]["model"]["enum"], json!(MODEL_TIERS));
    }

    #[test]
    fn criteria_and_model_reach_the_proposal() {
        let value = json!({
            "summary": "s",
            "members": [{
                "role": "backend",
                "objective": "o",
                "acceptance": ["`cargo test cache` passe"],
                "model": "haiku"
            }]
        });
        let p = proposal_from_result(Some(&value), None).unwrap();
        assert_eq!(p.members[0].acceptance, vec!["`cargo test cache` passe".to_string()]);
        assert_eq!(p.members[0].model.as_deref(), Some("haiku"));
    }

    #[test]
    fn an_empty_catalog_leaves_the_field_free() {
        let schema = team_proposal_schema(&[]);
        let role = &schema["properties"]["members"]["items"]["properties"]["role"];
        assert!(role.get("enum").is_none());
        assert_eq!(role["type"], json!("string"));
    }

    #[test]
    fn a_structured_reply_is_read_directly() {
        let value = json!({
            "summary": "Ajouter un cache.",
            "estimated_size": "m",
            "risks": ["invalidation"],
            "members": [
                {"role": "backend", "objective": "écrire le cache"},
                {"role": "tests", "objective": "couvrir", "depends_on": ["backend"]}
            ]
        });
        let p = proposal_from_result(Some(&value), None).unwrap();
        assert_eq!(p.members.len(), 2);
        assert_eq!(p.estimated_size, Size::M);
        assert_eq!(p.members[1].depends_on, vec!["backend".to_string()]);
        assert_eq!(p.risks, vec!["invalidation".to_string()]);
    }

    #[test]
    fn the_text_reply_is_the_fallback() {
        let text = r#"{"summary":"s","estimated_size":"s","members":[{"role":"backend","objective":"o"}]}"#;
        let p = proposal_from_result(None, Some(text)).unwrap();
        assert_eq!(p.members[0].role, "backend");
        assert_eq!(p.estimated_size, Size::S);
    }

    #[test]
    fn json_wrapped_in_prose_is_still_read() {
        // A model that ignores the schema and explains itself first.
        let text = "Voici l'équipe :\n```json\n{\"summary\":\"s\",\"members\":[{\"role\":\"backend\",\"objective\":\"o\"}]}\n```\nVoilà.";
        let p = proposal_from_result(None, Some(text)).unwrap();
        assert_eq!(p.members[0].role, "backend");
        // estimated_size is optional on the way in; it defaults.
        assert_eq!(p.estimated_size, Size::M);
    }

    #[test]
    fn braces_inside_strings_do_not_confuse_the_extractor() {
        let text = r#"note {"summary":"un { accolade } dedans","members":[{"role":"backend","objective":"o"}]} fin"#;
        let p = proposal_from_result(None, Some(text)).unwrap();
        assert!(p.summary.contains('{'));
        assert_eq!(p.members.len(), 1);
    }

    #[test]
    fn an_unusable_reply_says_so_rather_than_panicking() {
        let err = proposal_from_result(None, Some("désolé, je ne peux pas")).unwrap_err();
        assert!(err.to_string().contains("aucun objet JSON"), "{err}");

        assert!(proposal_from_result(None, None).is_err());

        // Valid JSON, wrong shape.
        let err = proposal_from_result(Some(&json!({"a": 1})), None).unwrap_err();
        assert!(err.to_string().contains("illisible"), "{err}");

        // Truncated JSON.
        assert!(proposal_from_result(None, Some(r#"{"summary":"s","memb"#)).is_err());
    }
}
