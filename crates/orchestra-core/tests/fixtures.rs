//! The parsers run against lines taken from real Claude Code transcripts.
//!
//! The fixtures are anonymised copies: structure, ids and token counts are
//! untouched, free text is replaced. They exist because the format belongs to
//! another program, and a change there must fail here rather than silently
//! mis-count tokens in production.

use std::collections::BTreeMap;

use orchestra_core::claude::{
    is_assistant_line, merge_tokens, StreamLine, TranscriptLine, UsageRecord,
};
use orchestra_core::model::Tokens;

const RESPONSE_BLOCKS: &str = include_str!("fixtures/response_blocks.jsonl");
const OTHER_LINES: &str = include_str!("fixtures/other_lines.jsonl");
const SUBAGENT: &str = include_str!("fixtures/subagent.jsonl");
/// A reviewer run under `--json-schema` (claude 2.1.289): the verdict comes
/// as `structured_output`, and `result` carries the same JSON as a string.
const REVIEWER_RESULT: &str = include_str!("fixtures/reviewer_result.jsonl");

fn records(jsonl: &str) -> Vec<UsageRecord> {
    jsonl
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            TranscriptLine::parse(l)
                .expect("ligne illisible")
                .as_assistant()?
                .usage_record()
        })
        .collect()
}

/// What the store does: one entry per message id, fields merged.
fn fold(recs: &[UsageRecord]) -> BTreeMap<String, Tokens> {
    let mut out: BTreeMap<String, Tokens> = BTreeMap::new();
    for r in recs {
        let entry = out.entry(r.message_id.clone()).or_default();
        *entry = merge_tokens(*entry, r.tokens);
    }
    out
}

#[test]
fn one_api_response_spans_several_lines_and_costs_one_response() {
    let recs = records(RESPONSE_BLOCKS);
    assert!(recs.len() >= 3, "la fixture doit couvrir plusieurs blocs");

    // Every line belongs to the same response.
    let ids: std::collections::BTreeSet<&str> =
        recs.iter().map(|r| r.message_id.as_str()).collect();
    assert_eq!(ids.len(), 1, "la fixture est une seule réponse API");

    // Naively summing the lines would multiply the bill.
    let naive: u64 = recs.iter().map(|r| r.tokens.output).sum();
    let folded = fold(&recs);
    let real = folded.values().next().unwrap();
    assert!(
        naive > real.output * 2,
        "la fixture doit démontrer le sur-comptage : naïf {naive}, réel {}",
        real.output
    );

    // The merged value is the complete one, not the first one seen.
    let max_seen = recs.iter().map(|r| r.tokens.output).max().unwrap();
    assert_eq!(real.output, max_seen);
    // Input and cache are stable across the lines of one response.
    assert_eq!(real.input, recs[0].tokens.input);
    assert_eq!(real.cache_read, recs[0].tokens.cache_read);
}

#[test]
fn a_partial_line_never_lowers_an_already_complete_total() {
    // The same block index appears twice in the wild: once while streaming
    // (iterations empty, tiny output) and once complete. Whichever we read
    // first, the answer must be the complete one.
    let recs = records(RESPONSE_BLOCKS);
    let mut forward = Tokens::default();
    for r in &recs {
        forward = merge_tokens(forward, r.tokens);
    }
    let mut backward = Tokens::default();
    for r in recs.iter().rev() {
        backward = merge_tokens(backward, r.tokens);
    }
    assert_eq!(forward, backward);
}

#[test]
fn every_other_line_type_is_ignored_without_error() {
    let mut count = 0;
    for line in OTHER_LINES.lines().filter(|l| !l.trim().is_empty()) {
        count += 1;
        assert!(
            !is_assistant_line(line.as_bytes()),
            "pré-filtre trop large sur : {}",
            &line[..line.len().min(60)]
        );
        let parsed = TranscriptLine::parse(line)
            .unwrap_or_else(|e| panic!("ligne illisible ({e}) : {}", &line[..line.len().min(80)]));
        assert!(parsed.as_assistant().is_none());
    }
    assert!(
        count >= 10,
        "la fixture doit couvrir les types réellement rencontrés, vu {count}"
    );
}

#[test]
fn a_subagent_line_bills_its_parent_session() {
    let line = SUBAGENT.lines().next().unwrap();
    assert!(is_assistant_line(line.as_bytes()));
    let parsed = TranscriptLine::parse(line).unwrap();
    let a = parsed.as_assistant().unwrap();
    assert!(a.is_sidechain, "une ligne de sous-agent est un sidechain");
    assert!(a.agent_id.is_some(), "un sous-agent porte son identifiant");
    // The session id is the parent's, which is how sub-agent costs roll up.
    assert!(a.usage_record().is_some());
}

#[test]
fn the_prefilter_agrees_with_the_parser_on_every_fixture() {
    for jsonl in [RESPONSE_BLOCKS, OTHER_LINES, SUBAGENT] {
        for line in jsonl.lines().filter(|l| !l.trim().is_empty()) {
            let is_assistant = TranscriptLine::parse(line)
                .map(|p| p.as_assistant().is_some())
                .unwrap_or(false);
            assert_eq!(
                is_assistant_line(line.as_bytes()),
                is_assistant,
                "désaccord pré-filtre/parseur sur : {}",
                &line[..line.len().min(60)]
            );
        }
    }
}

#[test]
fn a_headless_run_parses_from_init_to_result() {
    // Shape observed from `claude -p --output-format stream-json --verbose`.
    let run = [
        r#"{"type":"system","subtype":"init","cwd":"/tmp/x","session_id":"1e28a3c1-ed24-4070-934d-452b203ef99e","tools":["Bash"],"model":"claude-opus-5","permissionMode":"bypassPermissions","claude_code_version":"2.1.276"}"#,
        r#"{"type":"assistant","message":{"id":"msg_1","model":"claude-opus-5","role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}],"usage":{"input_tokens":4,"output_tokens":90,"cache_read_input_tokens":1000,"cache_creation_input_tokens":0}},"session_id":"1e28a3c1-ed24-4070-934d-452b203ef99e"}"#,
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]},"session_id":"1e28a3c1-ed24-4070-934d-452b203ef99e"}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"terminé","total_cost_usd":0.01,"num_turns":2,"duration_ms":900,"session_id":"1e28a3c1-ed24-4070-934d-452b203ef99e"}"#,
    ];
    let mut session = None;
    let mut tokens = Tokens::default();
    let mut finished = false;
    for line in run {
        match StreamLine::parse(line).expect("ligne de flux illisible") {
            StreamLine::System(s) => {
                session = s.session_id;
                assert_eq!(s.version.as_deref(), Some("2.1.276"));
            }
            StreamLine::Assistant { message, .. } => {
                let rec = message.usage_record(None).unwrap();
                tokens = merge_tokens(tokens, rec.tokens);
            }
            StreamLine::Result(r) => {
                finished = r.is_success();
                assert_eq!(r.result.as_deref(), Some("terminé"));
            }
            _ => {}
        }
    }
    assert!(session.is_some());
    assert!(finished);
    assert_eq!(tokens.output, 90);
    assert_eq!(tokens.cache_read, 1000);
}

#[test]
fn a_structured_verdict_comes_out_of_a_real_result_line() {
    use orchestra_core::review::{parse_review, structured_to_text, Verdict};
    let StreamLine::Result(r) = StreamLine::parse(REVIEWER_RESULT.trim()).unwrap() else {
        panic!("une ligne result était attendue");
    };
    assert!(r.is_success());
    let text = structured_to_text(r.structured_output.as_ref().expect("structured_output"))
        .expect("un verdict lisible");
    let review = parse_review(&text).unwrap();
    assert_eq!(review.verdict, Verdict::Changes);
    assert_eq!(review.roles_to_fix(&["backend", "tests"]), vec!["backend", "tests"]);
    // The text copy is the fallback: on its own, it is JSON, not a block.
    assert!(parse_review(r.result.as_deref().unwrap()).is_none());
}
