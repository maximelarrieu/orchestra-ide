//! Reading what Claude Code emits.
//!
//! Two sources describe the same work and must agree:
//!
//! * the `--output-format stream-json` stdout of an agent we started, and
//! * the transcript Claude Code writes under `~/.claude/projects`, which also
//!   covers sessions we did not start.
//!
//! Both are parsed here, tolerantly: unknown fields are kept in `rest`, unknown
//! variants fall into `Other`, and a line that cannot be parsed is reported
//! rather than fatal. The format belongs to another program and will drift.

pub mod stream;
pub mod transcript;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::model::Tokens;

pub use stream::{ApiMessage, ApiUsage, ContentBlock, ResultLine, StreamLine, SystemLine};
pub use transcript::{
    is_assistant_line, slug_for_cwd, TranscriptAssistant, TranscriptLine, TranscriptPath,
};

/// Everything one API response tells us about its cost.
///
/// A single response reaches us several times: once per content block in the
/// transcript, and again on the stdout of a managed agent. Those copies are
/// **not** identical, which is the whole reason this type exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageRecord {
    /// The API message id (`msg_…`). The dedupe key.
    pub message_id: String,
    pub model: String,
    pub tokens: Tokens,
    pub ts: Option<OffsetDateTime>,
    /// Block index within the response, when the source reports one.
    pub block_index: Option<u32>,
}

impl ApiMessage {
    /// Build a usage record, or `None` when the message carries no usage.
    pub fn usage_record(&self, block_index: Option<u32>) -> Option<UsageRecord> {
        let usage = self.usage.as_ref()?;
        let id = self.id.as_ref()?;
        Some(UsageRecord {
            message_id: id.clone(),
            model: self.model.clone().unwrap_or_default(),
            tokens: usage.tokens(),
            ts: None,
            block_index,
        })
    }
}

/// How two copies of the same response are combined.
///
/// Claude Code writes a transcript line per content block as the response
/// streams, and the early lines carry a **partial** `output_tokens`: a response
/// that ends at 787 output tokens appears as 5, 5, 5, 787. Keeping the first
/// copy would under-report the cost by two orders of magnitude, so we keep the
/// largest value seen for each field.
///
/// Input and cache counts are stable across the copies, so taking the maximum
/// is a no-op for them. The operation is commutative, which matters because the
/// stream and the transcript arrive in no particular order.
pub fn merge_tokens(a: Tokens, b: Tokens) -> Tokens {
    Tokens {
        input: a.input.max(b.input),
        output: a.output.max(b.output),
        cache_read: a.cache_read.max(b.cache_read),
        cache_creation: a.cache_creation.max(b.cache_creation),
        thinking: a.thinking.max(b.thinking),
    }
}

/// True when `new` adds anything to `old`.
pub fn adds_tokens(old: Tokens, new: Tokens) -> bool {
    merge_tokens(old, new) != old
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: u64, output: u64, cache_read: u64) -> Tokens {
        Tokens {
            input,
            output,
            cache_read,
            cache_creation: 0,
            thinking: 0,
        }
    }

    #[test]
    fn merging_keeps_the_complete_copy() {
        // The real shape: blocks 0..2 report 5 output tokens, block 3 reports 787.
        let partial = tokens(2, 5, 3061);
        let complete = tokens(2, 787, 3061);
        assert_eq!(merge_tokens(partial, complete), complete);
        // Order must not matter: the stream may arrive before the transcript.
        assert_eq!(merge_tokens(complete, partial), complete);
    }

    #[test]
    fn merging_is_idempotent() {
        let t = tokens(2, 787, 3061);
        assert_eq!(merge_tokens(t, t), t);
        assert!(!adds_tokens(t, t));
        assert!(adds_tokens(tokens(2, 5, 3061), t));
        assert!(!adds_tokens(t, tokens(2, 5, 3061)));
    }
}
