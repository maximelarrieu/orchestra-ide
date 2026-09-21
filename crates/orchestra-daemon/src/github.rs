//! The bit of GitHub Orchestra needs, through the `gh` command line.
//!
//! No HTTP client and no token handling: `gh` is already installed, already
//! authenticated, and already knows which repository a directory belongs to.
//! Asking it is one process and no secret of ours to keep.
//!
//! Everything here is blocking, like `worktree`: these are short commands, and
//! the callers already run on their own task.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// A pull request, as `gh` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub url: String,
    pub number: Option<u64>,
}

/// Where an open request ended up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Merged,
    /// Closed without being merged: the user said no.
    Closed,
}

/// True when `gh` is installed and usable.
pub fn is_available() -> bool {
    Command::new("gh")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Open a pull request for `head` against `base`.
///
/// A branch that already has one is not an error: the existing request is
/// returned, so a second attempt after a crash lands on its feet.
pub fn create_pr(
    repo: &Path,
    base: &str,
    head: &str,
    title: &str,
    body: &str,
) -> Result<PullRequest> {
    if let Some(existing) = view_pr(repo, head)? {
        return Ok(existing);
    }
    let out = Command::new("gh")
        .current_dir(repo)
        .args([
            "pr", "create", "--base", base, "--head", head, "--title", title, "--body", body,
        ])
        .output()
        .context("exécution de « gh pr create »")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        // A request created between our check and this call: take it.
        if let Some(existing) = view_pr(repo, head)? {
            return Ok(existing);
        }
        bail!("gh pr create : {err}");
    }
    let url = first_url(&String::from_utf8_lossy(&out.stdout))
        .context("« gh pr create » n'a pas rendu d'URL")?;
    Ok(PullRequest {
        number: number_from_url(&url),
        url,
    })
}

/// The request open for this branch, if there is one.
pub fn view_pr(repo: &Path, head: &str) -> Result<Option<PullRequest>> {
    let out = Command::new("gh")
        .current_dir(repo)
        .args(["pr", "view", head, "--json", "url,number,state"])
        .output()
        .context("exécution de « gh pr view »")?;
    if !out.status.success() {
        // No request for this branch is the ordinary answer, not a failure.
        return Ok(None);
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let url = json.get("url").and_then(|v| v.as_str()).unwrap_or_default();
    if url.is_empty() {
        return Ok(None);
    }
    Ok(Some(PullRequest {
        url: url.to_string(),
        number: json.get("number").and_then(|v| v.as_u64()),
    }))
}

/// Where a request stands now.
pub fn pr_state(repo: &Path, url: &str) -> Result<PrState> {
    let out = Command::new("gh")
        .current_dir(repo)
        .args(["pr", "view", url, "--json", "state,mergedAt"])
        .output()
        .context("exécution de « gh pr view »")?;
    if !out.status.success() {
        bail!(
            "gh pr view : {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(parse_state(&String::from_utf8_lossy(&out.stdout)))
}

/// Read the state out of what `gh pr view --json state,mergedAt` prints.
///
/// `mergedAt` is trusted before `state`: a merged request reports `MERGED`,
/// but a repository that squashes and closes reports `CLOSED` with a date.
pub fn parse_state(json: &str) -> PrState {
    let value: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let merged_at = value.get("mergedAt").and_then(|v| v.as_str()).unwrap_or("");
    if !merged_at.is_empty() && merged_at != "null" {
        return PrState::Merged;
    }
    match value
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_uppercase()
        .as_str()
    {
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        // Anything else — OPEN, DRAFT, or a word we do not know — leaves the
        // ticket where it is. Guessing here would close a live request.
        _ => PrState::Open,
    }
}

/// The request still open for a ticket, read from its events.
///
/// Kept here rather than in a column: the event stream already says it, and a
/// closed request is simply a later event. Newest first, the first answer wins.
pub async fn open_pull_request(
    store: &crate::store::Store,
    ticket_id: orchestra_core::model::TicketId,
) -> Option<String> {
    let filter = orchestra_core::events::EventFilter {
        ticket_id: Some(ticket_id),
        tags: vec![
            orchestra_core::events::EventTag::PullRequestOpened,
            orchestra_core::events::EventTag::PullRequestClosed,
        ],
        ..Default::default()
    };
    let events = store.recent_events(filter, 20).await.unwrap_or_default();
    for event in events.iter().rev() {
        match &event.kind {
            orchestra_core::events::EventKind::PullRequestClosed { .. } => return None,
            orchestra_core::events::EventKind::PullRequestOpened { url, .. } => {
                return Some(url.clone())
            }
            _ => {}
        }
    }
    None
}

/// Every ticket that has a request open right now.
///
/// One query for the whole board: the list is polled every second, and asking
/// per ticket would multiply that by the number of tickets for nothing.
pub async fn open_pull_requests(
    store: &crate::store::Store,
) -> std::collections::HashMap<orchestra_core::model::TicketId, String> {
    let filter = orchestra_core::events::EventFilter {
        tags: vec![
            orchestra_core::events::EventTag::PullRequestOpened,
            orchestra_core::events::EventTag::PullRequestClosed,
        ],
        ..Default::default()
    };
    let mut out = std::collections::HashMap::new();
    // Oldest first: a later close removes what an earlier open put there.
    for event in store.recent_events(filter, 500).await.unwrap_or_default() {
        let Some(ticket_id) = event.ticket_id else {
            continue;
        };
        match event.kind {
            orchestra_core::events::EventKind::PullRequestOpened { url, .. } => {
                out.insert(ticket_id, url);
            }
            orchestra_core::events::EventKind::PullRequestClosed { .. } => {
                out.remove(&ticket_id);
            }
            _ => {}
        }
    }
    out
}

/// The first http(s) URL in a blob of output.
fn first_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|w| w.starts_with("https://") || w.starts_with("http://"))
        .map(|w| w.trim_end_matches(['.', ',', ')']).to_string())
}

/// `https://github.com/o/r/pull/12` → 12.
fn number_from_url(url: &str) -> Option<u64> {
    url.rsplit('/').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_is_read_from_what_gh_prints() {
        assert_eq!(
            parse_state(r#"{"state":"OPEN","mergedAt":null}"#),
            PrState::Open
        );
        assert_eq!(
            parse_state(r#"{"state":"MERGED","mergedAt":"2026-09-19T18:00:00Z"}"#),
            PrState::Merged
        );
        assert_eq!(
            parse_state(r#"{"state":"CLOSED","mergedAt":null}"#),
            PrState::Closed
        );
        // Squash-and-close: the date is what says it was taken.
        assert_eq!(
            parse_state(r#"{"state":"CLOSED","mergedAt":"2026-09-19T18:00:00Z"}"#),
            PrState::Merged
        );
    }

    #[test]
    fn an_answer_we_cannot_read_leaves_the_request_open() {
        // Closing a live request on a parse failure would be the worst of the
        // two mistakes.
        assert_eq!(parse_state(""), PrState::Open);
        assert_eq!(parse_state("pas du json"), PrState::Open);
        assert_eq!(parse_state(r#"{"state":"DRAFT"}"#), PrState::Open);
        assert_eq!(parse_state(r#"{"autre":1}"#), PrState::Open);
    }

    #[test]
    fn the_url_is_picked_out_of_the_output() {
        let out = "Creating pull request for orch/12 into main\n\nhttps://github.com/o/r/pull/12\n";
        assert_eq!(
            first_url(out).as_deref(),
            Some("https://github.com/o/r/pull/12")
        );
        assert_eq!(number_from_url("https://github.com/o/r/pull/12"), Some(12));
        assert_eq!(number_from_url("https://github.com/o/r/pull/x"), None);
        assert!(first_url("rien à voir ici").is_none());
    }
}
