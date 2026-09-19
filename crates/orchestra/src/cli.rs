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
    /// Installe le catalogue de rôles et la configuration.
    Init {
        /// Réécrit les rôles livrés, même modifiés. La configuration n'est
        /// jamais écrasée.
        #[arg(long)]
        force: bool,
    },
    /// Gestion des tickets de feature.
    Ticket {
        #[command(subcommand)]
        action: TicketAction,
    },
    /// Pilotage d'un agent en cours.
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Suit un agent en direct dans ce terminal.
    Tail {
        /// Identifiant de l'agent, ou son rôle sur le ticket donné.
        agent: String,
        #[arg(long)]
        ticket: Option<String>,
    },
    /// Liste les rôles disponibles.
    Roles {
        /// Inclut les rôles propres à ce projet.
        #[arg(long)]
        project: Option<String>,
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
enum TicketAction {
    /// Crée un ticket.
    New {
        /// Projet : identifiant ou fragment de son nom.
        #[arg(long)]
        project: String,
        #[arg(long)]
        title: String,
        /// Le brief, en ligne.
        #[arg(long, conflicts_with = "brief_file")]
        brief: Option<String>,
        /// Le brief, depuis un fichier ; « - » pour l'entrée standard.
        #[arg(long)]
        brief_file: Option<String>,
        /// Enchaîne la planification.
        #[arg(long)]
        plan: bool,
    },
    /// Liste les tickets.
    List {
        #[arg(long)]
        project: Option<String>,
    },
    /// Affiche un ticket, son équipe et son coût.
    Show { ticket: String },
    /// Demande une proposition d'équipe à l'orchestrateur.
    Plan {
        ticket: String,
        /// N'attend pas la fin de la planification.
        #[arg(long)]
        detach: bool,
    },
    /// Accepte la proposition telle quelle.
    Accept { ticket: String },
    /// Lance l'équipe acceptée dans un worktree dédié.
    Launch {
        ticket: String,
        /// Suit le déroulement jusqu'au bout.
        #[arg(long)]
        follow: bool,
    },
    /// Arrête le ticket et ses agents.
    Cancel { ticket: String },
}

#[derive(Subcommand, Debug)]
enum AgentAction {
    /// Envoie une consigne à un agent en cours.
    Steer {
        agent: String,
        /// Le texte de la consigne.
        text: String,
        /// Interrompt le tour en cours au lieu d'attendre.
        #[arg(long)]
        hard: bool,
    },
    /// Arrête un agent.
    Cancel { agent: String },
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
            Sub::Init { force } => {
                let report = orchestra_daemon::init::init(force)?;
                for path in &report.written {
                    println!("écrit   {}", path.display());
                }
                for path in &report.kept {
                    println!("conservé {}", path.display());
                }
                if report.written.is_empty() {
                    println!("\nRien à installer : tout est déjà en place.");
                    println!("« orchestra init --force » restaure les rôles livrés.");
                } else {
                    println!(
                        "\nCatalogue installé dans {}.",
                        orchestra_core::config::Paths::roles_dir().display()
                    );
                    println!("Édite ces fichiers : ce sont les consignes que suivront tes agents.");
                }
                Ok(())
            }
            Sub::Roles { project } => {
                let mut client = Client::connect_or_spawn(&socket).await?;
                let project_id = match project {
                    Some(name) => Some(resolve_project(&mut client, &name).await?.id),
                    None => None,
                };
                match client.call(Cmd::ListRoles { project_id }).await? {
                    Reply::Roles { roles } => {
                        if roles.is_empty() {
                            println!("aucun rôle — lance « orchestra init »");
                        }
                        for r in roles {
                            let scope = match r.scope {
                                orchestra_core::model::RoleScope::Project => " (projet)",
                                orchestra_core::model::RoleScope::Global => "",
                            };
                            println!(
                                "{:<12} {:<9} {}{scope}",
                                r.name,
                                r.model.as_deref().unwrap_or("défaut"),
                                r.description
                            );
                        }
                        Ok(())
                    }
                    other => bail!("réponse inattendue : {other:?}"),
                }
            }
            Sub::Ticket { action } => run_ticket(action, &socket).await,
            Sub::Agent { action } => run_agent(action, &socket).await,
            Sub::Tail { agent, ticket } => {
                let mut client = Client::connect_or_spawn(&socket).await?;
                let agent_id = resolve_agent(&mut client, &agent, ticket.as_deref()).await?;
                drop(client);
                orchestra_tui::tail(&socket, agent_id).await
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

async fn run_ticket(action: TicketAction, socket: &std::path::Path) -> Result<()> {
    let mut client = Client::connect_or_spawn(socket).await?;
    match action {
        TicketAction::New {
            project,
            title,
            brief,
            brief_file,
            plan,
        } => {
            let brief = read_brief(brief, brief_file)?;
            let project = resolve_project(&mut client, &project).await?;
            let reply = client
                .call(Cmd::CreateTicket {
                    project_id: project.id,
                    title: title.clone(),
                    brief,
                })
                .await?;
            let ticket = match reply {
                Reply::Tickets { tickets } => tickets
                    .into_iter()
                    .next()
                    .context("le daemon n'a pas renvoyé le ticket créé")?,
                other => bail!("réponse inattendue : {other:?}"),
            };
            println!(
                "ticket #{} « {} » créé dans {}",
                ticket.ticket.number, ticket.ticket.title, project.name
            );
            if plan {
                plan_ticket(&mut client, ticket.ticket.id, false).await?;
            } else {
                println!(
                    "planifie-le : orchestra ticket plan {}",
                    ticket.ticket.number
                );
            }
            Ok(())
        }
        TicketAction::List { project } => {
            let project_id = match project {
                Some(name) => Some(resolve_project(&mut client, &name).await?.id),
                None => None,
            };
            match client
                .call(Cmd::ListTickets {
                    project_id,
                    status: None,
                })
                .await?
            {
                Reply::Tickets { tickets } => {
                    if tickets.is_empty() {
                        println!("aucun ticket — « orchestra ticket new --project <p> --title <t> --brief <b> »");
                    }
                    for t in tickets {
                        let cost = t.cost_usd.map(fmt_usd).unwrap_or_else(|| "-".into());
                        println!(
                            "#{:<4} {:<10} {:<40} {:>10}",
                            t.ticket.number,
                            t.ticket.status.label_fr(),
                            truncate(&t.ticket.title, 40),
                            cost
                        );
                    }
                    Ok(())
                }
                other => bail!("réponse inattendue : {other:?}"),
            }
        }
        TicketAction::Show { ticket } => {
            let id = resolve_ticket(&mut client, &ticket).await?;
            show_ticket(&mut client, id).await
        }
        TicketAction::Plan { ticket, detach } => {
            let id = resolve_ticket(&mut client, &ticket).await?;
            plan_ticket(&mut client, id, detach).await
        }
        TicketAction::Launch { ticket, follow } => {
            let id = resolve_ticket(&mut client, &ticket).await?;
            client
                .call(Cmd::LaunchTicket {
                    ticket_id: id,
                    open_panes: false,
                })
                .await?;
            let detail = ticket_detail(&mut client, id).await?;
            println!(
                "ticket #{} lancé{}",
                detail.ticket.number,
                detail
                    .ticket
                    .branch
                    .as_ref()
                    .map(|b| format!(" sur {b}"))
                    .unwrap_or_default()
            );
            if !follow {
                println!("suis-le : orchestra ticket show {}", detail.ticket.number);
                return Ok(());
            }
            follow_ticket(&mut client, id).await
        }
        TicketAction::Cancel { ticket } => {
            let id = resolve_ticket(&mut client, &ticket).await?;
            client.call(Cmd::CancelTicket { ticket_id: id }).await?;
            println!("arrêt demandé");
            Ok(())
        }
        TicketAction::Accept { ticket } => {
            let id = resolve_ticket(&mut client, &ticket).await?;
            let detail = ticket_detail(&mut client, id).await?;
            let proposal = detail
                .ticket
                .proposal
                .clone()
                .context("ce ticket n'a pas de proposition — lance d'abord « ticket plan »")?;
            let team = orchestra_core::model::Team {
                members: proposal.members.clone(),
                stages: Vec::new(),
            };
            match client
                .call(Cmd::AcceptProposal {
                    ticket_id: id,
                    team,
                })
                .await?
            {
                Reply::Ticket { detail } => {
                    println!(
                        "ticket #{} accepté — {} rôle(s), statut {}",
                        detail.ticket.number,
                        detail
                            .ticket
                            .team
                            .as_ref()
                            .map(|t| t.members.len())
                            .unwrap_or(0),
                        detail.ticket.status.label_fr()
                    );
                    Ok(())
                }
                other => bail!("réponse inattendue : {other:?}"),
            }
        }
    }
}

async fn run_agent(action: AgentAction, socket: &std::path::Path) -> Result<()> {
    let mut client = Client::connect_or_spawn(socket).await?;
    match action {
        AgentAction::Steer { agent, text, hard } => {
            let agent_id = resolve_agent(&mut client, &agent, None).await?;
            client
                .call(Cmd::SteerAgent {
                    agent_id,
                    text,
                    hard,
                })
                .await?;
            println!(
                "{} transmise",
                if hard { "redirection" } else { "consigne" }
            );
            Ok(())
        }
        AgentAction::Cancel { agent } => {
            let agent_id = resolve_agent(&mut client, &agent, None).await?;
            client.call(Cmd::CancelAgent { agent_id }).await?;
            println!("arrêt demandé");
            Ok(())
        }
    }
}

/// Accept an agent identifier, or a role name on a ticket. With neither, the
/// only agent currently running.
async fn resolve_agent(
    client: &mut Client,
    spec: &str,
    ticket: Option<&str>,
) -> Result<uuid::Uuid> {
    if let Ok(id) = uuid::Uuid::parse_str(spec) {
        return Ok(id);
    }
    let ticket_id = match ticket {
        Some(t) => Some(resolve_ticket(client, t).await?),
        None => None,
    };

    let mut candidates: Vec<(uuid::Uuid, String, bool)> = Vec::new();
    let tickets = match client
        .call(Cmd::ListTickets {
            project_id: None,
            status: None,
        })
        .await?
    {
        Reply::Tickets { tickets } => tickets,
        other => bail!("réponse inattendue : {other:?}"),
    };
    for summary in tickets {
        if ticket_id.is_some_and(|id| id != summary.ticket.id) {
            continue;
        }
        if ticket_id.is_none() && summary.agents_active == 0 {
            continue;
        }
        let detail = ticket_detail(client, summary.ticket.id).await?;
        for a in detail.agents {
            candidates.push((a.agent.id, a.agent.role.clone(), a.agent.status.is_active()));
        }
    }

    let lowered = spec.trim().to_lowercase();
    let matching: Vec<&(uuid::Uuid, String, bool)> = candidates
        .iter()
        .filter(|(_, role, _)| lowered.is_empty() || role.to_lowercase() == lowered)
        .collect();
    // An agent still running is what the user almost always means.
    let active: Vec<&&(uuid::Uuid, String, bool)> =
        matching.iter().filter(|(_, _, active)| *active).collect();
    let pool: Vec<&(uuid::Uuid, String, bool)> = if active.is_empty() {
        matching.clone()
    } else {
        active.into_iter().copied().collect()
    };

    match pool.as_slice() {
        [one] => Ok(one.0),
        [] => bail!("aucun agent ne correspond à « {spec} »"),
        many => bail!(
            "« {spec} » correspond à {} agents — donne son identifiant",
            many.len()
        ),
    }
}

/// Run the planning call and, unless detached, wait for its outcome.
async fn plan_ticket(client: &mut Client, ticket_id: uuid::Uuid, detach: bool) -> Result<()> {
    client.call(Cmd::PlanTicket { ticket_id }).await?;
    if detach {
        println!("planification lancée en arrière-plan.");
        return Ok(());
    }
    println!("l'orchestrateur compose l'équipe… (cela prend souvent une minute)");

    // Poll rather than subscribe: the CLI is short-lived and this keeps the
    // command readable. The TUI uses the event stream.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        let detail = ticket_detail(client, ticket_id).await?;
        if let Some(proposal) = &detail.ticket.proposal {
            print_proposal(proposal);
            println!(
                "\naccepte-la : orchestra ticket accept {}",
                detail.ticket.number
            );
            return Ok(());
        }
        for event in detail.recent_events.iter().rev() {
            if let orchestra_core::events::EventKind::ProposalFailed { error } = &event.kind {
                bail!("la planification a échoué : {error}");
            }
        }
        if std::time::Instant::now() > deadline {
            bail!("la planification n'a pas abouti dans le temps imparti");
        }
    }
}

/// Poll a running ticket until it settles, printing what changes.
async fn follow_ticket(client: &mut Client, ticket_id: uuid::Uuid) -> Result<()> {
    let mut last = String::new();
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        let detail = ticket_detail(client, ticket_id).await?;
        let line = detail
            .agents
            .iter()
            .map(|a| format!("{}:{}", a.agent.role, a.agent.status.label_fr()))
            .collect::<Vec<_>>()
            .join("  ");
        if line != last {
            println!("  {line}");
            last = line;
        }
        if detail.ticket.status.is_terminal()
            || detail.ticket.status == orchestra_core::model::TicketStatus::Review
        {
            println!(
                "\nticket #{} : {} — {} ({} tokens)",
                detail.ticket.number,
                detail.ticket.status.label_fr(),
                detail
                    .cost_usd
                    .map(fmt_usd)
                    .unwrap_or_else(|| "coût inconnu".into()),
                fmt_tokens(detail.tokens.total())
            );
            if let Some(branch) = &detail.ticket.branch {
                println!(
                    "relis la branche : git log {}..{branch}",
                    detail.project.default_branch
                );
            }
            return Ok(());
        }
    }
}

fn print_proposal(proposal: &orchestra_core::model::TeamProposal) {
    println!("\n{}", proposal.summary);
    println!("\nampleur estimée : {}", proposal.estimated_size.as_str());
    println!("\néquipe proposée :");
    for m in &proposal.members {
        let deps = if m.depends_on.is_empty() {
            String::new()
        } else {
            format!("  (après {})", m.depends_on.join(", "))
        };
        println!("  {:<12}{}", m.role, deps);
        println!("      {}", m.objective);
    }
    if !proposal.risks.is_empty() {
        println!("\npoints d'attention :");
        for r in &proposal.risks {
            println!("  - {r}");
        }
    }
}

async fn show_ticket(client: &mut Client, ticket_id: uuid::Uuid) -> Result<()> {
    let detail = ticket_detail(client, ticket_id).await?;
    let t = &detail.ticket;
    println!("#{} — {}", t.number, t.title);
    println!("projet   {}", detail.project.name);
    println!("statut   {}", t.status.label_fr());
    if let Some(branch) = &t.branch {
        println!("branche  {branch}");
    }
    println!(
        "coût     {} ({} tokens)",
        detail.cost_usd.map(fmt_usd).unwrap_or_else(|| "-".into()),
        fmt_tokens(detail.tokens.total())
    );
    println!("\nbrief :\n{}", t.brief.trim());
    if let Some(team) = &t.team {
        println!("\néquipe acceptée :");
        for (stage, m) in team.ordered() {
            println!("  étape {stage}  {:<12} {}", m.role, m.objective);
        }
    } else if let Some(proposal) = &t.proposal {
        println!("\nproposition en attente d'acceptation :");
        print_proposal(proposal);
    }
    if !detail.agents.is_empty() {
        println!("\nagents :");
        for a in &detail.agents {
            println!(
                "  {:<12} {:<10} {:>8} tokens {:>10}",
                a.agent.role,
                a.agent.status.label_fr(),
                fmt_tokens(a.tokens.total()),
                a.cost_usd.map(fmt_usd).unwrap_or_else(|| "-".into())
            );
        }
    }
    Ok(())
}

async fn ticket_detail(
    client: &mut Client,
    ticket_id: uuid::Uuid,
) -> Result<orchestra_core::protocol::TicketDetail> {
    match client.call(Cmd::GetTicket { ticket_id }).await? {
        Reply::Ticket { detail } => Ok(*detail),
        other => bail!("réponse inattendue : {other:?}"),
    }
}

/// Accept an identifier, a `#12`, or a fragment of the title.
async fn resolve_ticket(client: &mut Client, spec: &str) -> Result<uuid::Uuid> {
    if let Ok(id) = uuid::Uuid::parse_str(spec) {
        return Ok(id);
    }
    let tickets = match client
        .call(Cmd::ListTickets {
            project_id: None,
            status: None,
        })
        .await?
    {
        Reply::Tickets { tickets } => tickets,
        other => bail!("réponse inattendue : {other:?}"),
    };
    let needle = spec.trim_start_matches('#');
    if let Ok(number) = needle.parse::<i64>() {
        let matches: Vec<_> = tickets
            .iter()
            .filter(|t| t.ticket.number == number)
            .collect();
        match matches.as_slice() {
            [one] => return Ok(one.ticket.id),
            [] => bail!("aucun ticket #{number}"),
            many => bail!(
                "#{number} existe dans {} projets — précise l'identifiant",
                many.len()
            ),
        }
    }
    let lowered = needle.to_lowercase();
    let matches: Vec<_> = tickets
        .iter()
        .filter(|t| t.ticket.title.to_lowercase().contains(&lowered))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.ticket.id),
        [] => bail!("aucun ticket ne correspond à « {spec} »"),
        many => bail!("« {spec} » correspond à {} tickets — précise", many.len()),
    }
}

/// Accept an identifier, a path, or a fragment of the project name.
async fn resolve_project(
    client: &mut Client,
    spec: &str,
) -> Result<orchestra_core::model::Project> {
    let projects = match client.call(Cmd::ListProjects).await? {
        Reply::Projects { projects } => projects,
        other => bail!("réponse inattendue : {other:?}"),
    };
    if let Ok(id) = uuid::Uuid::parse_str(spec) {
        return projects
            .into_iter()
            .find(|p| p.id == id)
            .context("aucun projet avec cet identifiant");
    }
    let lowered = spec.to_lowercase();
    let matches: Vec<_> = projects
        .into_iter()
        .filter(|p| {
            p.name.to_lowercase().contains(&lowered)
                || p.path.to_string_lossy().to_lowercase().contains(&lowered)
        })
        .collect();
    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        0 => bail!("aucun projet ne correspond à « {spec} » — « orchestra project list »"),
        _ => bail!(
            "« {spec} » correspond à {} projets : {}",
            matches.len(),
            matches
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn read_brief(inline: Option<String>, file: Option<String>) -> Result<String> {
    let brief = match (inline, file) {
        (Some(text), _) => text,
        (None, Some(path)) if path == "-" => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("lecture du brief sur l'entrée standard")?;
            buf
        }
        (None, Some(path)) => {
            std::fs::read_to_string(&path).with_context(|| format!("lecture de {path}"))?
        }
        (None, None) => bail!("donne un brief : --brief \"…\" ou --brief-file <fichier>"),
    };
    let brief = brief.trim().to_string();
    anyhow::ensure!(!brief.is_empty(), "le brief est vide");
    Ok(brief)
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
