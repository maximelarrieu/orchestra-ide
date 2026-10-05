//! Desktop notifications, via `notify-send` (libnotify over D-Bus).
//!
//! Best-effort only, on the model of `zellij.rs`: a systemd timer that failed
//! every morning because the binary is missing or D-Bus does not answer would
//! be worse than a notification that silently did not show. Used by the
//! morning `orchestra todo notify`, and by the daemon when a ticket starts
//! waiting on the user (`attention::notify_on_attention`).

use std::process::Stdio;
use std::time::Duration;

/// Local IPC to a notification daemon answers in milliseconds; three
/// seconds is a deadline nothing healthy ever reaches.
const DEADLINE: Duration = Duration::from_secs(3);

pub async fn send(title: &str, body: &str) {
    let mut command = tokio::process::Command::new("notify-send");
    command
        .arg("--app-name=Orchestra")
        .arg(title)
        .arg(body)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            tracing::debug!("notify-send introuvable : {e}");
            return;
        }
    };
    match tokio::time::timeout(DEADLINE, child.wait_with_output()).await {
        Ok(Ok(out)) if out.status.success() => {}
        Ok(Ok(out)) => tracing::debug!(
            "notify-send a refusé : {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Ok(Err(e)) => tracing::debug!("notify-send illisible : {e}"),
        Err(_) => tracing::warn!("notify-send n'a pas répondu en {:?}", DEADLINE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_notification_never_hangs_or_panics() {
        // Whether or not `notify-send` exists on the machine running the
        // tests, this must return well inside the deadline.
        let start = std::time::Instant::now();
        send("test", "corps").await;
        assert!(start.elapsed() < DEADLINE + Duration::from_secs(1));
    }
}
