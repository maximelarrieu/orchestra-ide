//! The JSONL Claude Code writes under `~/.claude/projects`.
//!
//! One directory per working directory, one file per session, plus
//! `<session>/subagents/agent-<id>.jsonl` for sub-agents. Interactive sessions
//! write the same usage as headless ones, which is what lets Orchestra account
//! for every Claude Code run on the machine, not only the ones it started.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use uuid::Uuid;

use super::stream::ApiMessage;
use super::UsageRecord;

/// Around thirty line types exist and only `assistant` carries usage, so the
/// scanner tests for this needle before parsing anything.
const ASSISTANT_NEEDLE: &str = "\"type\":\"assistant\"";

/// Cheap pre-filter: does this line deserve to be parsed at all?
pub fn is_assistant_line(line: &[u8]) -> bool {
    // Transcripts are written compactly, but tolerate one space after the colon.
    memfind(line, ASSISTANT_NEEDLE.as_bytes()) || memfind(line, b"\"type\": \"assistant\"")
}

fn memfind(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// A transcript line we care about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TranscriptLine {
    Assistant(Box<TranscriptAssistant>),
    /// Everything else: `user`, `attachment`, `ai-title`, `cost-state`, …
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptAssistant {
    pub message: ApiMessage,
    /// Session that owns the file. For a sub-agent this is the **parent**
    /// session, which is how sub-agent costs roll up naturally.
    #[serde(rename = "sessionId")]
    pub session_id: Uuid,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub uuid: Option<Uuid>,
    /// Groups the lines of one API response.
    #[serde(default, rename = "requestId")]
    pub request_id: Option<String>,
    /// Position of this content block inside the response.
    #[serde(default, rename = "apiBlockIndex")]
    pub block_index: Option<u32>,
    #[serde(default, rename = "agentId")]
    pub agent_id: Option<String>,
    #[serde(default, rename = "isSidechain")]
    pub is_sidechain: bool,
    #[serde(default, rename = "gitBranch")]
    pub git_branch: Option<String>,
    /// Claude Code version that wrote the line.
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default, rename = "attributionAgent")]
    pub attribution_agent: Option<String>,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl TranscriptAssistant {
    pub fn timestamp(&self) -> Option<OffsetDateTime> {
        self.timestamp
            .as_deref()
            .and_then(|s| OffsetDateTime::parse(s, &Rfc3339).ok())
    }

    /// Cost of this line, or `None` when there is nothing to bill.
    pub fn usage_record(&self) -> Option<UsageRecord> {
        let mut rec = self.message.usage_record(self.block_index)?;
        rec.ts = self.timestamp();
        Some(rec)
    }
}

impl TranscriptLine {
    pub fn parse(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line)
    }

    pub fn as_assistant(&self) -> Option<&TranscriptAssistant> {
        match self {
            TranscriptLine::Assistant(a) => Some(a),
            TranscriptLine::Other => None,
        }
    }
}

/// Where a transcript file sits in the layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptPath {
    /// Session the file belongs to, read from the file name or its parent.
    pub session_id: Option<Uuid>,
    /// Set for `…/<session>/subagents/agent-<id>.jsonl`.
    pub subagent_id: Option<String>,
}

impl TranscriptPath {
    /// Read what the path itself says, without opening the file.
    pub fn parse(path: &Path) -> Self {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        if let Some(id) = stem.strip_prefix("agent-") {
            // …/<session_id>/subagents/agent-<id>.jsonl
            let session_id = path
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .and_then(|n| Uuid::parse_str(&n.to_string_lossy()).ok());
            return TranscriptPath {
                session_id,
                subagent_id: Some(id.to_string()),
            };
        }
        TranscriptPath {
            session_id: Uuid::parse_str(&stem).ok(),
            subagent_id: None,
        }
    }
}

/// Claude Code's directory name for a working directory: every character that
/// is not alphanumeric becomes a dash.
pub fn slug_for_cwd(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_LINE: &str = r#"{"parentUuid":"a8c5c406-d25e-45b7-999b-13da9cfa2d6e","isSidechain":false,"message":{"id":"msg_011CfAWzquDS9w4hVFA2","model":"claude-fable-5-1","role":"assistant","content":[{"type":"thinking","thinking":"…"}],"usage":{"input_tokens":2,"cache_creation_input_tokens":8722,"cache_read_input_tokens":3061,"output_tokens":5,"output_tokens_details":{"thinking_tokens":78},"service_tier":"standard","inference_geo":"not_available"}},"apiBlockIndex":0,"requestId":"req_011CfAWzoGTWBZE2VgepT8KA","type":"assistant","uuid":"0987d2f4-b1ce-4873-92ba-0771ded77d0c","timestamp":"2026-09-18T13:39:36.006Z","effort":"high","session_id":null,"userType":"external","entrypoint":"cli","cwd":"/home/sheguey/.dev/malapa/orchestra-ide_v2","sessionId":"56faa7f2-8582-463b-936c-aee6ae9590b1","version":"2.1.276","gitBranch":"main","slug":"phase-0"}"#;

    #[test]
    fn a_real_assistant_line_parses_completely() {
        assert!(is_assistant_line(REAL_LINE.as_bytes()));
        let line = TranscriptLine::parse(REAL_LINE).unwrap();
        let a = line.as_assistant().unwrap();
        assert_eq!(
            a.session_id.to_string(),
            "56faa7f2-8582-463b-936c-aee6ae9590b1"
        );
        assert_eq!(a.block_index, Some(0));
        assert_eq!(a.version.as_deref(), Some("2.1.276"));
        assert_eq!(a.git_branch.as_deref(), Some("main"));
        assert!(!a.is_sidechain);
        assert_eq!(
            a.cwd.as_deref(),
            Some(Path::new("/home/sheguey/.dev/malapa/orchestra-ide_v2"))
        );

        let rec = a.usage_record().unwrap();
        assert_eq!(rec.message_id, "msg_011CfAWzquDS9w4hVFA2");
        assert_eq!(rec.model, "claude-fable-5-1");
        assert_eq!(rec.tokens.output, 5);
        assert_eq!(rec.tokens.thinking, 78);
        assert_eq!(rec.block_index, Some(0));
        assert!(rec.ts.is_some());
        // Fields we do not model are kept rather than dropped.
        assert!(a.rest.contains_key("userType"));
        assert!(a.rest.contains_key("slug"));
    }

    #[test]
    fn non_assistant_lines_are_skipped_cheaply() {
        for line in [
            r#"{"type":"attachment","content":"…"}"#,
            r#"{"type":"ai-title","title":"x"}"#,
            r#"{"type":"cost-state","sessionId":"x","totalCostUSD":0}"#,
            r#"{"type":"file-history-snapshot","messageId":"x"}"#,
            r#"{"type":"user","message":{"role":"user"}}"#,
        ] {
            assert!(
                !is_assistant_line(line.as_bytes()),
                "ne devrait pas être pré-filtré : {line}"
            );
            assert_eq!(TranscriptLine::parse(line).unwrap(), TranscriptLine::Other);
        }
    }

    #[test]
    fn the_prefilter_accepts_a_space_after_the_colon() {
        assert!(is_assistant_line(br#"{"type": "assistant","message":{}}"#));
        assert!(!is_assistant_line(b""));
        assert!(!is_assistant_line(b"{"));
    }

    #[test]
    fn subagent_paths_carry_the_parent_session() {
        let p = Path::new(
            "/home/u/.claude/projects/-home-u-proj/a43b0ee9-c2aa-469f-87aa-c3a88d3636da/subagents/agent-a307747007d952a44.jsonl",
        );
        let parsed = TranscriptPath::parse(p);
        assert_eq!(
            parsed.session_id.map(|u| u.to_string()).as_deref(),
            Some("a43b0ee9-c2aa-469f-87aa-c3a88d3636da")
        );
        assert_eq!(parsed.subagent_id.as_deref(), Some("a307747007d952a44"));
    }

    #[test]
    fn session_paths_are_named_after_the_session() {
        let p = Path::new(
            "/home/u/.claude/projects/-home-u-proj/56faa7f2-8582-463b-936c-aee6ae9590b1.jsonl",
        );
        let parsed = TranscriptPath::parse(p);
        assert_eq!(
            parsed.session_id.map(|u| u.to_string()).as_deref(),
            Some("56faa7f2-8582-463b-936c-aee6ae9590b1")
        );
        assert!(parsed.subagent_id.is_none());
    }

    #[test]
    fn an_unexpected_file_name_is_not_fatal() {
        let parsed = TranscriptPath::parse(Path::new("/tmp/notes.jsonl"));
        assert!(parsed.session_id.is_none());
        assert!(parsed.subagent_id.is_none());
    }

    #[test]
    fn a_subagent_line_reports_its_agent_and_parent_session() {
        let line = r#"{"type":"assistant","sessionId":"a43b0ee9-c2aa-469f-87aa-c3a88d3636da","agentId":"a307747007d952a44","isSidechain":true,"cwd":"/home/u/proj","attributionAgent":"Plan","apiBlockIndex":0,"message":{"id":"msg_1","model":"claude-sonnet-5","content":[],"usage":{"input_tokens":1,"output_tokens":2}}}"#;
        let parsed = TranscriptLine::parse(line).unwrap();
        let a = parsed.as_assistant().unwrap();
        assert!(a.is_sidechain);
        assert_eq!(a.agent_id.as_deref(), Some("a307747007d952a44"));
        assert_eq!(a.attribution_agent.as_deref(), Some("Plan"));
    }

    #[test]
    fn the_cwd_slug_matches_claude_codes_layout() {
        assert_eq!(
            slug_for_cwd(Path::new("/home/sheguey/.dev/malapa/orchestra-ide_v2")),
            "-home-sheguey--dev-malapa-orchestra-ide-v2"
        );
    }

    #[test]
    fn a_line_missing_its_session_is_rejected_not_panicking() {
        let line = r#"{"type":"assistant","message":{"id":"m","content":[]}}"#;
        assert!(TranscriptLine::parse(line).is_err());
    }
}
