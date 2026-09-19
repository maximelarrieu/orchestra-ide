//! The NDJSON an agent writes on stdout with `--output-format stream-json`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::model::Tokens;

/// One line of the stream.
///
/// Every variant keeps a `rest` map so a new field never breaks parsing, and an
/// unknown `type` lands in [`StreamLine::Other`] instead of failing the line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamLine {
    System(SystemLine),
    Assistant {
        message: ApiMessage,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(default)]
        parent_tool_use_id: Option<String>,
        #[serde(default)]
        timestamp: Option<String>,
        #[serde(flatten)]
        rest: Map<String, Value>,
    },
    /// Tool results, echoed back as a user turn.
    User {
        #[serde(default)]
        message: Value,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(default)]
        parent_tool_use_id: Option<String>,
        #[serde(flatten)]
        rest: Map<String, Value>,
    },
    Result(ResultLine),
    /// Only present with `--include-partial-messages`.
    StreamEvent {
        #[serde(default)]
        event: Value,
        #[serde(default)]
        session_id: Option<Uuid>,
    },
    #[serde(other)]
    Other,
}

impl StreamLine {
    /// Parse one line, never panicking on unknown shapes.
    pub fn parse(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line)
    }

    pub fn session_id(&self) -> Option<Uuid> {
        match self {
            StreamLine::System(s) => s.session_id,
            StreamLine::Assistant { session_id, .. }
            | StreamLine::User { session_id, .. }
            | StreamLine::StreamEvent { session_id, .. } => *session_id,
            StreamLine::Result(r) => r.session_id,
            StreamLine::Other => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemLine {
    #[serde(default)]
    pub subtype: String,
    #[serde(default)]
    pub session_id: Option<Uuid>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    /// Reported as `claude_code_version` on the init line.
    #[serde(default, rename = "claude_code_version")]
    pub version: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

/// The final line of a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResultLine {
    #[serde(default)]
    pub subtype: String,
    #[serde(default)]
    pub is_error: bool,
    /// The agent's closing message, used as the hand-off to the next role.
    #[serde(default)]
    pub result: Option<String>,
    /// Payload of `--json-schema`, when the run asked for structured output.
    #[serde(default)]
    pub structured_output: Option<Value>,
    #[serde(default)]
    pub total_cost_usd: Option<f64>,
    #[serde(default)]
    pub usage: Option<ApiUsage>,
    #[serde(default, rename = "modelUsage")]
    pub model_usage: Option<Value>,
    #[serde(default)]
    pub num_turns: Option<u32>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub session_id: Option<Uuid>,
    #[serde(default)]
    pub permission_denials: Vec<Value>,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl ResultLine {
    /// Did the run finish its job?
    pub fn is_success(&self) -> bool {
        !self.is_error && self.subtype == "success"
    }
}

/// An API message as both the stream and the transcript embed it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiMessage {
    /// `msg_…`. Absent on locally synthesised messages.
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub usage: Option<ApiUsage>,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl ApiMessage {
    /// Messages Claude Code makes up locally (errors, notices) carry this model
    /// and no real cost.
    pub fn is_synthetic(&self) -> bool {
        self.model.as_deref() == Some("<synthetic>")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub output_tokens_details: Option<OutputDetails>,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl ApiUsage {
    pub fn tokens(&self) -> Tokens {
        Tokens {
            input: self.input_tokens,
            output: self.output_tokens,
            cache_read: self.cache_read_input_tokens,
            cache_creation: self.cache_creation_input_tokens,
            thinking: self
                .output_tokens_details
                .as_ref()
                .map(|d| d.thinking_tokens)
                .unwrap_or(0),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OutputDetails {
    #[serde(default)]
    pub thinking_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        #[serde(default)]
        text: String,
    },
    Thinking {
        #[serde(default)]
        thinking: String,
    },
    ToolUse {
        #[serde(default)]
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        input: Value,
    },
    ToolResult {
        #[serde(default)]
        tool_use_id: String,
        #[serde(default)]
        is_error: bool,
        #[serde(default)]
        content: Value,
    },
    #[serde(other)]
    Other,
}

impl ContentBlock {
    /// Short label for the activity log, e.g. `cargo test --workspace` for a
    /// Bash call or the path for an edit. Never the full tool input.
    pub fn tool_summary(&self) -> Option<(String, String, String)> {
        let ContentBlock::ToolUse { id, name, input } = self else {
            return None;
        };
        let pick = |key: &str| input.get(key).and_then(|v| v.as_str()).unwrap_or("");
        let summary = match name.as_str() {
            "Bash" => pick("command").to_string(),
            "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
                pick("file_path").to_string()
            }
            "Grep" => pick("pattern").to_string(),
            "Glob" => pick("pattern").to_string(),
            "WebFetch" => pick("url").to_string(),
            "Task" | "Agent" => pick("description").to_string(),
            _ => input
                .as_object()
                .and_then(|o| o.keys().next().cloned())
                .unwrap_or_default(),
        };
        Some((id.clone(), name.clone(), truncate(&summary, 120)))
    }
}

/// Cut a string to `max` characters on a character boundary.
pub fn truncate(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= max {
        s
    } else {
        let head: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

/// Fields of a `result` line, flattened for the store.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelUsageBreakdown(pub BTreeMap<String, Value>);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_line_parses() {
        let line = r#"{"type":"system","subtype":"init","cwd":"/tmp/x","session_id":"1e28a3c1-ed24-4070-934d-452b203ef99e","tools":["Bash","Read"],"model":"claude-opus-5","permissionMode":"default","claude_code_version":"2.1.276","apiKeySource":"none"}"#;
        let parsed = StreamLine::parse(line).unwrap();
        let StreamLine::System(s) = &parsed else {
            panic!("attendu system, obtenu {parsed:?}")
        };
        assert_eq!(s.subtype, "init");
        assert_eq!(s.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(s.version.as_deref(), Some("2.1.276"));
        assert_eq!(s.tools.len(), 2);
        // Unknown fields survive rather than break the line.
        assert!(s.rest.contains_key("apiKeySource"));
    }

    #[test]
    fn assistant_line_yields_a_usage_record() {
        let line = r#"{"type":"assistant","message":{"id":"msg_01","model":"claude-opus-5","role":"assistant","content":[{"type":"text","text":"bonjour"}],"usage":{"input_tokens":2,"output_tokens":787,"cache_creation_input_tokens":8722,"cache_read_input_tokens":3061,"output_tokens_details":{"thinking_tokens":120},"service_tier":"standard"}},"session_id":"1e28a3c1-ed24-4070-934d-452b203ef99e","parent_tool_use_id":null}"#;
        let StreamLine::Assistant { message, .. } = StreamLine::parse(line).unwrap() else {
            panic!("attendu assistant")
        };
        let rec = message.usage_record(None).unwrap();
        assert_eq!(rec.message_id, "msg_01");
        assert_eq!(rec.model, "claude-opus-5");
        assert_eq!(rec.tokens.output, 787);
        assert_eq!(rec.tokens.cache_read, 3061);
        assert_eq!(rec.tokens.cache_creation, 8722);
        assert_eq!(rec.tokens.thinking, 120);
        assert_eq!(rec.tokens.total_input(), 2 + 3061 + 8722);
    }

    #[test]
    fn a_message_without_usage_has_no_record() {
        let m = ApiMessage {
            id: Some("msg_x".into()),
            model: Some("m".into()),
            usage: None,
            content: vec![],
            stop_reason: None,
            rest: Map::new(),
        };
        assert!(m.usage_record(None).is_none());
    }

    #[test]
    fn synthetic_messages_are_recognised() {
        let line = r#"{"type":"assistant","message":{"id":"x","model":"<synthetic>","role":"assistant","content":[{"type":"text","text":"Not logged in"}],"usage":{"input_tokens":0,"output_tokens":0}},"session_id":"1e28a3c1-ed24-4070-934d-452b203ef99e","error":"authentication_failed"}"#;
        let StreamLine::Assistant { message, .. } = StreamLine::parse(line).unwrap() else {
            panic!("attendu assistant")
        };
        assert!(message.is_synthetic());
    }

    #[test]
    fn result_line_parses_with_its_cost() {
        let line = r#"{"type":"result","subtype":"success","is_error":false,"result":"fini","total_cost_usd":0.42,"num_turns":3,"duration_ms":1200,"session_id":"1e28a3c1-ed24-4070-934d-452b203ef99e","usage":{"input_tokens":10,"output_tokens":20},"modelUsage":{"claude-opus-5":{"inputTokens":10}},"permission_denials":[]}"#;
        let StreamLine::Result(r) = StreamLine::parse(line).unwrap() else {
            panic!("attendu result")
        };
        assert!(r.is_success());
        assert_eq!(r.total_cost_usd, Some(0.42));
        assert_eq!(r.num_turns, Some(3));
        assert_eq!(r.result.as_deref(), Some("fini"));
    }

    #[test]
    fn an_unknown_line_type_is_not_an_error() {
        let parsed = StreamLine::parse(r#"{"type":"quelque_chose_de_neuf","a":1}"#).unwrap();
        assert_eq!(parsed, StreamLine::Other);
        assert_eq!(parsed.session_id(), None);
    }

    #[test]
    fn an_unknown_content_block_is_not_an_error() {
        let line = r#"{"type":"assistant","message":{"id":"m","model":"x","content":[{"type":"image","source":{}},{"type":"text","text":"ok"}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        let StreamLine::Assistant { message, .. } = StreamLine::parse(line).unwrap() else {
            panic!("attendu assistant")
        };
        assert_eq!(message.content.len(), 2);
        assert_eq!(message.content[0], ContentBlock::Other);
    }

    #[test]
    fn tool_calls_are_summarised_without_leaking_their_input() {
        let cases = vec![
            (
                r#"{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test --workspace","description":"tests"}}"#,
                "cargo test --workspace",
            ),
            (
                r#"{"type":"tool_use","id":"t2","name":"Edit","input":{"file_path":"src/x.rs","old_string":"SECRET","new_string":"y"}}"#,
                "src/x.rs",
            ),
            (
                r#"{"type":"tool_use","id":"t3","name":"Grep","input":{"pattern":"TODO","path":"."}}"#,
                "TODO",
            ),
        ];
        for (json, expected) in cases {
            let block: ContentBlock = serde_json::from_str(json).unwrap();
            let (_, _, summary) = block.tool_summary().unwrap();
            assert_eq!(summary, expected);
        }
        // The replaced text of an edit never reaches the summary.
        let block: ContentBlock = serde_json::from_str(
            r#"{"type":"tool_use","id":"t","name":"Edit","input":{"file_path":"a","old_string":"SECRET"}}"#,
        )
        .unwrap();
        let (_, _, summary) = block.tool_summary().unwrap();
        assert!(!summary.contains("SECRET"));
    }

    #[test]
    fn long_summaries_are_cut_on_character_boundaries() {
        let long = "é".repeat(400);
        let cut = truncate(&long, 120);
        assert_eq!(cut.chars().count(), 120);
        assert!(cut.ends_with('…'));
        assert_eq!(truncate("a\nb", 10), "a b");
    }

    #[test]
    fn text_blocks_have_no_tool_summary() {
        let block = ContentBlock::Text { text: "x".into() };
        assert!(block.tool_summary().is_none());
    }
}
