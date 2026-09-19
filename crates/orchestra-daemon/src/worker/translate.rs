//! Turning a stream line into events the dashboard can show.
//!
//! Pure: no I/O, no clock, no database. That is what lets it be tested against
//! recorded fixtures rather than a live agent.

use orchestra_core::claude::{ContentBlock, StreamLine};
use orchestra_core::events::EventKind;
use orchestra_core::model::{AgentStatus, ExitReason, Tokens, UsageSample, UsageSource};
use uuid::Uuid;

/// What the caller needs to attach an event to a ticket.
#[derive(Debug, Clone, Copy)]
pub struct Scope {
    pub agent_id: Option<Uuid>,
    pub ticket_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub session_id: Uuid,
}

/// Translate one line. A line may yield several events (text, then usage) or
/// none at all.
pub fn translate(scope: &Scope, line: &StreamLine, now: time::OffsetDateTime) -> Vec<EventKind> {
    let mut out = Vec::new();
    match line {
        StreamLine::System(s) => {
            // `init` says the process is really up; other subtypes are noise.
            if s.subtype == "init" {
                out.push(EventKind::AgentStatusChanged {
                    status: AgentStatus::Running,
                    reason: None,
                });
            }
        }
        StreamLine::Assistant { message, .. } => {
            for block in &message.content {
                match block {
                    ContentBlock::Text { text } if !text.trim().is_empty() => {
                        out.push(EventKind::AgentText { text: redact(text) });
                    }
                    ContentBlock::Thinking { thinking } => {
                        // Only the size: reasoning is never stored.
                        out.push(EventKind::AgentThinking {
                            chars: thinking.chars().count(),
                        });
                    }
                    ContentBlock::ToolUse { .. } => {
                        if let Some((tool_use_id, tool, summary)) = block.tool_summary() {
                            out.push(EventKind::ToolStarted {
                                tool_use_id,
                                tool,
                                summary: redact(&summary),
                            });
                        }
                    }
                    _ => {}
                }
            }
            if !message.is_synthetic() {
                if let Some(record) = message.usage_record(None) {
                    out.push(EventKind::Usage {
                        sample: Box::new(UsageSample {
                            message_id: record.message_id,
                            session_id: scope.session_id,
                            subagent_id: None,
                            agent_id: scope.agent_id,
                            ticket_id: scope.ticket_id,
                            project_id: scope.project_id,
                            model: record.model,
                            tokens: record.tokens,
                            ts: record.ts.unwrap_or(now),
                            source: UsageSource::Stream,
                        }),
                    });
                }
            }
        }
        StreamLine::User { message, .. } => {
            for (tool_use_id, ok, summary) in tool_results(message) {
                out.push(EventKind::ToolFinished {
                    tool_use_id,
                    ok,
                    summary: redact(&summary),
                });
            }
        }
        StreamLine::Result(r) => {
            out.push(EventKind::AgentResult {
                subtype: r.subtype.clone(),
                num_turns: r.num_turns.unwrap_or(0),
                duration_ms: r.duration_ms.unwrap_or(0),
                total_cost_usd: r.total_cost_usd,
                text: redact(&orchestra_core::claude::stream::truncate(
                    r.result.as_deref().unwrap_or(""),
                    2000,
                )),
            });
            let reason = exit_reason(&r.subtype, r.is_error);
            out.push(EventKind::AgentStatusChanged {
                status: if reason.is_success() {
                    AgentStatus::Done
                } else {
                    AgentStatus::Failed
                },
                reason: Some(reason),
            });
        }
        StreamLine::StreamEvent { .. } | StreamLine::Other => {}
    }
    out
}

/// Map the `result` subtype onto why the run ended.
///
/// The exact error subtypes are Claude Code's, so anything unrecognised is kept
/// verbatim rather than forced into a known bucket.
pub fn exit_reason(subtype: &str, is_error: bool) -> ExitReason {
    match subtype {
        "success" if !is_error => ExitReason::Success,
        "error_max_turns" => ExitReason::MaxTurns,
        s if s.contains("budget") => ExitReason::MaxBudget,
        "error_during_execution" => ExitReason::ErrorDuringExecution,
        other => ExitReason::Unknown(other.to_string()),
    }
}

/// Tool results carried by a user turn: `(tool_use_id, ok, summary)`.
fn tool_results(message: &serde_json::Value) -> Vec<(String, bool, String)> {
    let Some(content) = message.get("content").and_then(|c| c.as_array()) else {
        return Vec::new();
    };
    content
        .iter()
        .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
        .map(|b| {
            let id = b
                .get("tool_use_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let is_error = b.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
            let summary = first_line(b.get("content"));
            (id, !is_error, summary)
        })
        .collect()
}

/// A tool result is either a string or a list of blocks; either way we keep
/// only its first line, short.
fn first_line(content: Option<&serde_json::Value>) -> String {
    let text = match content {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(blocks)) => blocks
            .iter()
            .find_map(|b| b.get("text").and_then(|t| t.as_str()))
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    };
    orchestra_core::claude::stream::truncate(text.lines().next().unwrap_or(""), 200)
}

/// Mask anything that looks like a credential before it is stored.
///
/// The event log holds an agent's own words and tool summaries, and both can
/// quote a key the agent just read. Masking here is cheap and covers the log,
/// the terminal and any future export in one place.
pub fn redact(text: &str) -> String {
    const PREFIXES: [&str; 6] = ["sk-ant-", "sk-", "ghp_", "github_pat_", "AKIA", "xoxb-"];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'outer: while !rest.is_empty() {
        for prefix in PREFIXES {
            if let Some(pos) = rest.find(prefix) {
                let (before, tail) = rest.split_at(pos);
                out.push_str(before);
                let secret_len = tail
                    .char_indices()
                    .find(|(i, c)| {
                        *i >= prefix.len() && !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(tail.len());
                // A bare prefix with nothing after it is not a secret.
                if secret_len > prefix.len() {
                    out.push_str("[masqué]");
                    rest = &tail[secret_len..];
                } else {
                    out.push_str(&tail[..prefix.len()]);
                    rest = &tail[prefix.len()..];
                }
                continue 'outer;
            }
        }
        out.push_str(rest);
        break;
    }
    if let Some(pos) = out.find("-----BEGIN") {
        out.truncate(pos);
        out.push_str("[clé privée masquée]");
    }
    out
}

/// Tokens reported by a `result` line, when it carries them.
pub fn result_tokens(line: &StreamLine) -> Option<Tokens> {
    match line {
        StreamLine::Result(r) => r.usage.as_ref().map(|u| u.tokens()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::claude::StreamLine;

    fn scope() -> Scope {
        Scope {
            agent_id: Some(Uuid::new_v4()),
            ticket_id: Some(Uuid::new_v4()),
            project_id: Some(Uuid::new_v4()),
            session_id: Uuid::new_v4(),
        }
    }

    fn run(line: &str) -> Vec<EventKind> {
        let parsed = StreamLine::parse(line).expect("ligne illisible");
        translate(&scope(), &parsed, orchestra_core::now())
    }

    #[test]
    fn the_init_line_marks_the_agent_running() {
        let events = run(
            r#"{"type":"system","subtype":"init","session_id":"1e28a3c1-ed24-4070-934d-452b203ef99e","model":"claude-opus-5"}"#,
        );
        assert!(matches!(
            events.as_slice(),
            [EventKind::AgentStatusChanged {
                status: AgentStatus::Running,
                ..
            }]
        ));
        // Other system subtypes are noise, including ones we have not seen.
        assert!(run(r#"{"type":"system","subtype":"thinking_tokens"}"#).is_empty());
    }

    #[test]
    fn an_assistant_turn_yields_text_tools_and_usage() {
        let events = run(
            r#"{"type":"assistant","session_id":"1e28a3c1-ed24-4070-934d-452b203ef99e","message":{"id":"msg_1","model":"claude-opus-5","content":[{"type":"thinking","thinking":"abc"},{"type":"text","text":"Je lance les tests."},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}],"usage":{"input_tokens":2,"output_tokens":90,"cache_read_input_tokens":10,"cache_creation_input_tokens":0}}}"#,
        );
        assert_eq!(events.len(), 4);
        assert!(matches!(&events[0], EventKind::AgentThinking { chars } if *chars == 3));
        assert!(
            matches!(&events[1], EventKind::AgentText { text } if text == "Je lance les tests.")
        );
        assert!(
            matches!(&events[2], EventKind::ToolStarted { tool, summary, .. } if tool == "Bash" && summary == "cargo test")
        );
        assert!(matches!(&events[3], EventKind::Usage { sample } if sample.tokens.output == 90));
    }

    #[test]
    fn thinking_content_is_never_stored() {
        let events = run(
            r#"{"type":"assistant","message":{"id":"m","model":"x","content":[{"type":"thinking","thinking":"un secret de raisonnement"}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
        );
        let serialised = serde_json::to_string(&events).unwrap();
        assert!(!serialised.contains("secret de raisonnement"));
        assert!(serialised.contains("agent_thinking"));
    }

    #[test]
    fn a_tool_result_closes_its_call() {
        let events = run(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"test result: ok. 12 passed\nplein d'autres lignes"}]}}"#,
        );
        assert!(
            matches!(&events[0], EventKind::ToolFinished { tool_use_id, ok, summary } if tool_use_id == "t1" && *ok && summary == "test result: ok. 12 passed")
        );

        let failed = run(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t2","is_error":true,"content":[{"type":"text","text":"erreur de compilation"}]}]}}"#,
        );
        assert!(matches!(&failed[0], EventKind::ToolFinished { ok, .. } if !ok));
    }

    #[test]
    fn a_synthetic_message_costs_nothing() {
        let events = run(
            r#"{"type":"assistant","message":{"id":"m","model":"<synthetic>","content":[{"type":"text","text":"Not logged in"}],"usage":{"input_tokens":0,"output_tokens":0}}}"#,
        );
        assert!(!events.iter().any(|e| matches!(e, EventKind::Usage { .. })));
        assert_eq!(events.len(), 1, "seul le texte reste");
    }

    #[test]
    fn the_result_line_ends_the_run() {
        let events = run(
            r#"{"type":"result","subtype":"success","is_error":false,"result":"terminé","num_turns":4,"duration_ms":1200,"total_cost_usd":0.5,"session_id":"1e28a3c1-ed24-4070-934d-452b203ef99e"}"#,
        );
        assert!(
            matches!(&events[0], EventKind::AgentResult { subtype, num_turns, .. } if subtype == "success" && *num_turns == 4)
        );
        assert!(matches!(
            &events[1],
            EventKind::AgentStatusChanged {
                status: AgentStatus::Done,
                reason: Some(ExitReason::Success)
            }
        ));
    }

    #[test]
    fn known_failures_are_named_and_unknown_ones_kept_verbatim() {
        assert_eq!(exit_reason("success", false), ExitReason::Success);
        assert_eq!(exit_reason("error_max_turns", true), ExitReason::MaxTurns);
        assert_eq!(
            exit_reason("error_during_execution", true),
            ExitReason::ErrorDuringExecution
        );
        assert_eq!(
            exit_reason("error_max_budget_usd", true),
            ExitReason::MaxBudget
        );
        // An unfamiliar subtype keeps its name rather than being mislabelled.
        match exit_reason("error_tout_neuf", true) {
            ExitReason::Unknown(s) => assert_eq!(s, "error_tout_neuf"),
            other => panic!("attendu Unknown, obtenu {other:?}"),
        }
        // `success` with the error flag set is not a success.
        assert!(!exit_reason("success", true).is_success());
    }

    #[test]
    fn credentials_are_masked_before_being_stored() {
        let cases = [
            ("clé sk-ant-api03-ABCdef123 dedans", "[masqué]"),
            ("token ghp_abcdefghij1234567890", "[masqué]"),
            ("aws AKIAIOSFODNN7EXAMPLE", "[masqué]"),
            ("slack xoxb-123-456-abc", "[masqué]"),
        ];
        for (input, expected) in cases {
            let out = redact(input);
            assert!(out.contains(expected), "« {input} » a donné « {out} »");
            assert!(!out.contains("ABCdef123"));
            assert!(!out.contains("abcdefghij1234567890"));
        }
        let key = redact("voici\n-----BEGIN RSA PRIVATE KEY-----\nMIIE...\n");
        assert!(key.contains("[clé privée masquée]"));
        assert!(!key.contains("MIIE"));

        // Ordinary text is untouched, including words that merely start alike.
        assert_eq!(redact("rien à cacher ici"), "rien à cacher ici");
        assert_eq!(redact("sk- seul"), "sk- seul");
        assert_eq!(redact(""), "");
    }

    #[test]
    fn redaction_reaches_text_tools_and_results() {
        let events = run(
            r#"{"type":"assistant","message":{"id":"m","model":"x","content":[{"type":"text","text":"la clé est sk-ant-api03-SECRET1234"},{"type":"tool_use","id":"t","name":"Bash","input":{"command":"export TOKEN=ghp_SECRET1234567890"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
        );
        let serialised = serde_json::to_string(&events).unwrap();
        assert!(!serialised.contains("SECRET1234"));
        assert!(serialised.contains("masqué"));
    }

    #[test]
    fn an_empty_text_block_produces_nothing() {
        let events = run(
            r#"{"type":"assistant","message":{"id":"m","model":"x","content":[{"type":"text","text":"   "}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
        );
        assert!(!events
            .iter()
            .any(|e| matches!(e, EventKind::AgentText { .. })));
    }
}
