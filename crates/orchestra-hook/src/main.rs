//! Hook shim.
//!
//! Claude Code runs this on every hook event of a managed agent. It reads the
//! hook JSON on stdin, wraps it in an Orchestra request and writes one line to
//! the daemon socket.
//!
//! Three rules, because this sits on the critical path of every tool call:
//! start fast (no dependencies, not even serde), never block for long, and
//! never fail the agent for an Orchestra problem. Exit code 0 means "allow";
//! only an explicit deny from the daemon exits 2.
//!
//! Phase 0 ships the transport; the guard that answers deny lands in phase 3.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// Claude Code kills a hook that overruns; stay well under its timeout.
const TIMEOUT: Duration = Duration::from_millis(1500);

fn main() {
    // Whatever happens, an Orchestra failure must not break the agent.
    let verdict = std::panic::catch_unwind(run).unwrap_or(Verdict::Allow);
    match verdict {
        Verdict::Allow => std::process::exit(0),
        Verdict::Deny(reason) => {
            // Exit 2 with a reason on stderr is how a hook refuses a tool call.
            eprintln!("{reason}");
            std::process::exit(2);
        }
    }
}

enum Verdict {
    Allow,
    Deny(String),
}

fn run() -> Verdict {
    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() {
        return Verdict::Allow;
    }
    let payload = payload.trim();
    if payload.is_empty() {
        return Verdict::Allow;
    }

    let Some(socket) = std::env::var_os("ORCHESTRA_SOCK") else {
        // Not launched by Orchestra: nothing to report, let the call through.
        return Verdict::Allow;
    };
    let agent_id = std::env::var("ORCHESTRA_AGENT_ID").ok();

    match send(&socket, payload, agent_id.as_deref()) {
        Some(line) => parse_verdict(&line),
        None => Verdict::Allow,
    }
}

/// Write one request line and read one response line.
fn send(socket: &std::ffi::OsStr, payload: &str, agent_id: Option<&str>) -> Option<String> {
    let mut stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;

    let agent = match agent_id {
        Some(id) => format!("\"agent_id\":\"{}\",", escape(id)),
        None => String::new(),
    };
    // The payload is already valid JSON from Claude Code, embedded as-is.
    let request = format!("{{\"id\":1,\"cmd\":\"hook\",{agent}\"payload\":{payload}}}\n");
    stream.write_all(request.as_bytes()).ok()?;
    stream.flush().ok()?;

    // The daemon greets with a Hello line, then answers; read a few lines and
    // keep the first response.
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if let Some(line) = find_response(&buf) {
                    return Some(line);
                }
                if buf.len() > 1 << 20 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    find_response(&buf)
}

/// First complete line that is a response rather than the greeting.
fn find_response(buf: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(buf).ok()?;
    for line in text.split('\n') {
        if line.is_empty() {
            continue;
        }
        if line.contains("\"type\":\"hello\"") {
            continue;
        }
        if line.contains("\"outcome\"") {
            return Some(line.to_string());
        }
    }
    None
}

/// Minimal reading of the reply: deny only on an explicit refusal.
fn parse_verdict(line: &str) -> Verdict {
    if !line.contains("\"reply\":\"hook\"") || !line.contains("\"allow\":false") {
        return Verdict::Allow;
    }
    let reason = field(line, "reason").unwrap_or_else(|| "refusé par Orchestra".to_string());
    Verdict::Deny(reason)
}

/// Pull a JSON string field out of a flat object without a JSON parser.
fn field(line: &str, name: &str) -> Option<String> {
    let needle = format!("\"{name}\":\"");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => break,
            },
            other => out.push(other),
        }
    }
    None
}

fn escape(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_socket_allows_the_call() {
        // No ORCHESTRA_SOCK in the environment of a normal `claude` run.
        assert!(send(std::ffi::OsStr::new("/nonexistent.sock"), "{}", None).is_none());
    }

    #[test]
    fn only_an_explicit_refusal_denies() {
        let allow = r#"{"id":1,"outcome":"ok","reply":"hook","allow":true}"#;
        assert!(matches!(parse_verdict(allow), Verdict::Allow));

        let err = r#"{"id":1,"outcome":"err","error":{"code":"internal","message":"boom"}}"#;
        assert!(matches!(parse_verdict(err), Verdict::Allow));

        let deny =
            r#"{"id":1,"outcome":"ok","reply":"hook","allow":false,"reason":"hors worktree"}"#;
        match parse_verdict(deny) {
            Verdict::Deny(r) => assert_eq!(r, "hors worktree"),
            Verdict::Allow => panic!("aurait dû refuser"),
        }
    }

    #[test]
    fn the_greeting_is_skipped() {
        let buf = b"{\"type\":\"hello\",\"version\":\"0.1.0\"}\n{\"id\":1,\"outcome\":\"ok\",\"reply\":\"hook\",\"allow\":true}\n";
        let line = find_response(buf).unwrap();
        assert!(line.contains("\"reply\":\"hook\""));
    }

    #[test]
    fn partial_lines_are_not_parsed() {
        assert_eq!(
            find_response(b"{\"type\":\"hello\"}\n{\"id\":1,\"outc"),
            None
        );
    }

    #[test]
    fn fields_are_extracted_with_escapes() {
        assert_eq!(
            field(r#"{"reason":"a\"b"}"#, "reason").as_deref(),
            Some("a\"b")
        );
        assert_eq!(
            field(r#"{"reason":"x\ny"}"#, "reason").as_deref(),
            Some("x\ny")
        );
        assert_eq!(field(r#"{"other":"z"}"#, "reason"), None);
    }

    #[test]
    fn agent_ids_are_sanitised() {
        assert_eq!(escape("abc-123"), "abc-123");
        assert_eq!(escape("a\"b\\c"), "abc");
    }
}
