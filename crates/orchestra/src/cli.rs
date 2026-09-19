//! Command line surface.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use orchestra_core::config::{Config, Paths};
use orchestra_core::pricing::{fmt_tokens, fmt_usd};
use orchestra_core::protocol::{Command as Cmd, GroupBy, Reply, TimeRange, UsageQuery};
use orchestra_tui::Client;

#[derive(Parser, Debug)]
#[command(
    name = "orchestra",
    version,
    about = "Tableau de bord d'une ferme d'agents Claude Code",
    long_about = None
)]
pub struct Cli {
    /// Chemin du socket du daemon.
    #[arg(long, global = true)]
    socket: Option<PathBuf>,
    /// Fichier de configuration.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Sub>,
}

#[derive(Subcommand, Debug)]
enum Sub {
    /// Lance le daemon au premier plan.
    Daemon {
        /// Ne rien écrire sur la sortie standard (utilisé au démarrage auto).
        #[arg(long)]
        quiet: bool,
    },
    /// Ouvre le tableau de bord (par défaut).
    Tui,
    /// Vérifie que le daemon répond.
    Ping,
    /// État du daemon.
    Status,
    /// Gestion des projets.
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Consommation de tokens.
    Usage {
        /// Regroupement : project, ticket, agent, role, model, day.
        #[arg(long = "by", default_value = "project")]
        by: String,
        /// Fenêtre : 7d, 24h, 30m (minutes), today, all.
        #[arg(long, default_value = "all")]
        since: String,
        /// Exclure les sessions Claude Code non pilotées par Orchestra.
        #[arg(long)]
        managed_only: bool,
    },
}

#[derive(Subcommand, Debug)]
enum ProjectAction {
    /// Ajoute un projet.
    Add {
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
    },
    /// Liste les projets connus.
    List,
}

impl Cli {
    pub async fn run(self) -> Result<()> {
        let config = match &self.config {
            Some(p) => Config::load_from(p)?,
            None => Config::load()?,
        };
        let socket = self.socket.clone().unwrap_or_else(Paths::socket);

        match self.command.unwrap_or(Sub::Tui) {
            Sub::Daemon { quiet } => run_daemon(config, quiet).await,
            Sub::Tui => orchestra_tui::run(&socket).await,
            Sub::Ping => {
                let mut client = Client::connect(&socket)
                    .await
                    .context("le daemon ne répond pas — lance `orchestra daemon`")?;
                match client.call(Cmd::Ping).await? {
                    Reply::Pong => {
                        println!("pong (daemon v{})", client.daemon_version);
                        Ok(())
                    }
                    other => bail!("réponse inattendue : {other:?}"),
                }
            }
            Sub::Status => {
                let mut client = Client::connect(&socket).await?;
                match client.call(Cmd::Status).await? {
                    Reply::Status { status } => {
                        println!("version        {}", status.version);
                        println!("protocole      {}", status.protocol);
                        println!("démarré        {}", status.started_at);
                        println!("projets        {}", status.projects);
                        println!("tickets actifs {}", status.tickets_running);
                        println!("agents actifs  {}", status.agents_running);
                        println!("événements     {}", status.last_seq);
                        println!("transcripts    {}", status.watched_files);
                        Ok(())
                    }
                    other => bail!("réponse inattendue : {other:?}"),
                }
            }
            Sub::Project { action } => {
                let mut client = Client::connect_or_spawn(&socket).await?;
                match action {
                    ProjectAction::Add { path, name } => {
                        match client.call(Cmd::AddProject { path, name }).await? {
                            Reply::Project { project } => {
                                println!(
                                    "projet « {} » ajouté ({}, branche {})",
                                    project.name,
                                    project.path.display(),
                                    project.default_branch
                                );
                                Ok(())
                            }
                            other => bail!("réponse inattendue : {other:?}"),
                        }
                    }
                    ProjectAction::List => match client.call(Cmd::ListProjects).await? {
                        Reply::Projects { projects } => {
                            if projects.is_empty() {
                                println!("aucun projet — `orchestra project add <chemin>`");
                            }
                            for p in projects {
                                let kind =
                                    if p.kind == orchestra_core::model::ProjectKind::Discovered {
                                        " (découvert)"
                                    } else {
                                        ""
                                    };
                                println!("{:<24} {}{kind}", p.name, p.path.display());
                            }
                            Ok(())
                        }
                        other => bail!("réponse inattendue : {other:?}"),
                    },
                }
            }
            Sub::Usage {
                by,
                since,
                managed_only,
            } => {
                let group =
                    GroupBy::parse(&by).with_context(|| format!("regroupement inconnu : {by}"))?;
                let range = parse_since(&since)?;
                let mut client = Client::connect_or_spawn(&socket).await?;
                let query = UsageQuery {
                    group_by: vec![group],
                    range,
                    include_unmanaged: !managed_only,
                    ..Default::default()
                };
                match client.call(Cmd::GetUsage { query }).await? {
                    Reply::Usage { rows, totals } => {
                        println!(
                            "{:<32} {:>9} {:>9} {:>9} {:>8} {:>11}",
                            group.label_fr(),
                            "entrée",
                            "sortie",
                            "cache",
                            "msg",
                            "coût"
                        );
                        for r in &rows {
                            let key = r
                                .keys
                                .values()
                                .next()
                                .cloned()
                                .unwrap_or_else(|| "-".into());
                            let cost = match (r.cost_usd, r.cost_estimated) {
                                (Some(v), true) => format!("{}*", fmt_usd(v)),
                                (Some(v), false) => fmt_usd(v),
                                (None, _) => "-".into(),
                            };
                            println!(
                                "{:<32} {:>9} {:>9} {:>9} {:>8} {:>11}",
                                truncate(&key, 32),
                                fmt_tokens(r.tokens.input),
                                fmt_tokens(r.tokens.output),
                                fmt_tokens(r.tokens.cache_read + r.tokens.cache_creation),
                                r.messages,
                                cost,
                            );
                        }
                        println!(
                            "\n{} messages, {} tokens, {} (indicatif)",
                            totals.messages,
                            fmt_tokens(totals.tokens.total()),
                            totals
                                .cost_usd
                                .map(fmt_usd)
                                .unwrap_or_else(|| "coût inconnu".into())
                        );
                        if rows.iter().any(|r| r.cost_estimated) {
                            println!(
                                "* tarif approché : modèle absent de la grille de config.toml"
                            );
                        }
                        Ok(())
                    }
                    other => bail!("réponse inattendue : {other:?}"),
                }
            }
        }
    }
}

async fn run_daemon(config: Config, quiet: bool) -> Result<()> {
    init_tracing(quiet)?;
    let shutdown = async {
        let mut term =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
        let terminate = async {
            match term.as_mut() {
                Some(s) => {
                    s.recv().await;
                }
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate => {}
        }
    };
    orchestra_daemon::run(config, shutdown).await
}

fn init_tracing(quiet: bool) -> Result<()> {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("ORCHESTRA_LOG")
        .unwrap_or_else(|_| EnvFilter::new("orchestra=info,orchestra_daemon=info,warn"));
    let state = Paths::state_dir();
    std::fs::create_dir_all(&state)?;
    let file = tracing_appender::rolling::daily(&state, "daemon.log");
    let builder = fmt().with_env_filter(filter).with_target(false);
    if quiet {
        builder.with_writer(file).with_ansi(false).init();
    } else {
        builder.with_writer(std::io::stderr).init();
    }
    Ok(())
}

/// `7d`, `24h`, `90m`, `today`, `all`.
fn parse_since(spec: &str) -> Result<TimeRange> {
    let spec = spec.trim().to_lowercase();
    if spec.is_empty() || spec == "all" || spec == "tout" || spec == "0" {
        return Ok(TimeRange::all());
    }
    if spec == "today" || spec == "aujourd'hui" {
        let now = orchestra_core::now();
        return Ok(TimeRange {
            since: Some(now.replace_time(time::Time::MIDNIGHT)),
            until: None,
        });
    }
    let (digits, unit) = spec.split_at(spec.len() - 1);
    let n: i64 = digits.parse().with_context(|| {
        format!("fenêtre illisible : {spec} (exemples : 7d, 24h, 90m, today, all)")
    })?;
    anyhow::ensure!(n > 0, "la fenêtre doit être positive");
    let duration = match unit {
        "d" | "j" => time::Duration::days(n),
        "h" => time::Duration::hours(n),
        "m" => time::Duration::minutes(n),
        "w" | "s" => time::Duration::weeks(n),
        other => anyhow::bail!("unité inconnue « {other} » (d, h, m, w)"),
    };
    Ok(TimeRange {
        since: Some(orchestra_core::now() - duration),
        until: None,
    })
}

fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        s.to_string()
    } else {
        let keep: String = s.chars().take(width.saturating_sub(1)).collect();
        format!("{keep}…")
    }
}
