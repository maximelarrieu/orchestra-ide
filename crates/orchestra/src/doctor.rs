//! `orchestra doctor`: what the farm needs, and which of it is missing.
//!
//! Each line carries a symbol as well as its words: ✔ fine, ! degraded but
//! working, ✘ the farm cannot do its job (rule 11: never colour alone).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Result};
use orchestra_core::config::{Config, Paths};
use orchestra_core::sandbox::SandboxMode;
use orchestra_tui::Client;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Ok,
    Warn,
    Fail,
}

struct Line {
    level: Level,
    what: &'static str,
    detail: String,
}

fn line(level: Level, what: &'static str, detail: impl Into<String>) -> Line {
    Line { level, what, detail: detail.into() }
}

/// First line of `<bin> --version`, or why it could not run.
fn version_of(bin: &str) -> Result<String, String> {
    match Command::new(bin).arg("--version").output() {
        Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string()),
        Ok(out) => Err(format!("code de sortie {:?}", out.status.code())),
        Err(e) => Err(e.to_string()),
    }
}

pub async fn run(config_path: Option<&Path>, socket: &Path) -> Result<()> {
    let mut lines = Vec::new();

    let loaded = match config_path {
        Some(p) => Config::load_from(p),
        None => Config::load(),
    };
    let config = match loaded.and_then(|c| c.validate().map(|()| c)) {
        Ok(c) => {
            lines.push(line(Level::Ok, "configuration", "lisible et valide"));
            c
        }
        Err(e) => {
            lines.push(line(Level::Fail, "configuration", e.to_string()));
            Config::default()
        }
    };

    match version_of(&config.daemon.claude_bin) {
        Ok(v) => lines.push(line(Level::Ok, "claude", v)),
        Err(e) => lines.push(line(
            Level::Fail,
            "claude",
            format!("« {} » ne se lance pas : {e}", config.daemon.claude_bin),
        )),
    }

    let hook = PathBuf::from(orchestra_daemon::hooks::hook_binary());
    if hook.is_absolute() && hook.is_file() {
        lines.push(line(Level::Ok, "garde-fou", hook.display().to_string()));
    } else {
        lines.push(line(
            Level::Fail,
            "garde-fou",
            "orchestra-hook est introuvable : sans lui aucun agent n'est surveillé — \
             `cargo install --path crates/orchestra-hook`",
        ));
    }

    match version_of("git") {
        Ok(v) => lines.push(line(Level::Ok, "git", v)),
        Err(e) => lines.push(line(Level::Fail, "git", e)),
    }

    match version_of("zellij") {
        Ok(v) => lines.push(line(Level::Ok, "zellij", v)),
        Err(_) => lines.push(line(Level::Warn, "zellij", "absent : pas de pane par agent")),
    }

    let pr_mode = config.integration.mode.is_pr();
    match version_of("gh") {
        Ok(v) => lines.push(line(Level::Ok, "gh", v)),
        Err(e) if pr_mode => lines.push(line(
            Level::Fail,
            "gh",
            format!("integration.mode = \"pr\" en a besoin : {e}"),
        )),
        Err(_) => lines.push(line(Level::Warn, "gh", "absent : pas de pull request")),
    }

    match config.sandbox.mode {
        SandboxMode::Off => lines.push(line(Level::Warn, "bac à sable", "désactivé (sandbox.mode = \"off\")")),
        SandboxMode::Bwrap => match version_of("bwrap") {
            Ok(v) => lines.push(line(Level::Ok, "bac à sable", v)),
            Err(e) => lines.push(line(
                Level::Fail,
                "bac à sable",
                format!("sandbox.mode = \"bwrap\" mais bwrap ne se lance pas : {e}"),
            )),
        },
    }

    let roles = std::fs::read_dir(Paths::roles_dir())
        .map(|d| d.filter_map(|e| e.ok()).filter(|e| e.path().extension().is_some_and(|x| x == "md")).count())
        .unwrap_or(0);
    if roles > 0 {
        lines.push(line(Level::Ok, "rôles", format!("{roles} installé(s)")));
    } else {
        lines.push(line(Level::Fail, "rôles", "aucun rôle installé — `orchestra init`"));
    }

    match Client::connect(socket).await {
        Ok(client) => lines.push(line(Level::Ok, "daemon", format!("répond (v{})", client.daemon_version))),
        Err(_) => lines.push(line(Level::Warn, "daemon", "arrêté : il démarre tout seul à la première commande")),
    }

    for l in &lines {
        let symbol = match l.level {
            Level::Ok => "✔",
            Level::Warn => "!",
            Level::Fail => "✘",
        };
        println!("{symbol} {:<14} {}", l.what, l.detail);
    }
    let failed = lines.iter().filter(|l| l.level == Level::Fail).count();
    if failed > 0 {
        bail!("{failed} point(s) bloquant(s)");
    }
    Ok(())
}
