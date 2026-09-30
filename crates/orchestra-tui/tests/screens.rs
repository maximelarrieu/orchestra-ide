//! The phase 2 screens, rendered into a test terminal.
//!
//! These check what the user actually sees: that a ticket shows its team, that
//! the form refuses an empty brief, that the editor warns before an invalid
//! team can be accepted, and that none of it panics in a narrow pane.

use std::path::PathBuf;

use orchestra_core::events::{Event, EventKind, NewEvent};
use orchestra_core::model::{
    Agent, AgentStatus, Effort, Project, ProjectKind, RoleDefinition, RoleScope, Size, Team,
    TeamMember, TeamProposal, Ticket, TicketStatus, Tokens,
};
use orchestra_core::protocol::{
    AgentSummary, Command, Reply, ReviewOutcome, TicketDetail, TicketSummary,
};
use orchestra_core::review::Verdict;
use orchestra_tui::app::{App, Screen};
use orchestra_tui::forms::TicketField;
use orchestra_tui::keymap::Action;
use orchestra_tui::Msg;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use uuid::Uuid;

fn draw(app: &App, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| orchestra_tui::screens::render(app, f))
        .unwrap();
    let buf = term.backend().buffer().clone();
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn role(name: &str) -> RoleDefinition {
    RoleDefinition {
        name: name.into(),
        description: format!("rôle {name}"),
        model: None,
        effort: None,
        allowed_tools: vec![],
        disallowed_tools: vec![],
        max_budget_usd: None,
        subagents: None,
        tags: vec![],
        git: None,
        system_prompt: "consigne".into(),
        source: PathBuf::from("/tmp/r.md"),
        scope: RoleScope::Global,
    }
}

fn member(name: &str, deps: &[&str]) -> TeamMember {
    TeamMember {
        role: name.into(),
        objective: format!("objectif détaillé de {name}"),
        depends_on: deps.iter().map(|s| s.to_string()).collect(),
        model: None,
        effort: None,
        max_budget_usd: None,
        parallel_ok: false,
    }
}

fn detail(with_proposal: bool, with_team: bool) -> TicketDetail {
    let project_id = Uuid::new_v4();
    let ticket_id = Uuid::new_v4();
    let members = vec![member("architect", &[]), member("backend", &["architect"])];
    TicketDetail {
        ticket: Ticket {
            id: ticket_id,
            project_id,
            number: 12,
            title: "Ajouter un cache mémoire".into(),
            brief: "Le rendu recalcule tout à chaque frappe. On veut un cache.".into(),
            status: TicketStatus::Draft,
            branch: None,
            worktree_path: None,
            proposal: with_proposal.then(|| TeamProposal {
                summary: "Deux rôles suffisent pour ce cache.".into(),
                members: members.clone(),
                risks: vec!["invalidation du cache".into()],
                estimated_size: Size::M,
            }),
            team: with_team.then(|| Team {
                members: members.clone(),
                stages: vec![vec!["architect".into()], vec!["backend".into()]],
            }),
            created_at: orchestra_core::now(),
            updated_at: orchestra_core::now(),
        },
        project: Project {
            id: project_id,
            name: "mon-projet".into(),
            path: PathBuf::from("/tmp/p"),
            default_branch: "main".into(),
            zellij_tab: None,
            kind: ProjectKind::Managed,
            created_at: orchestra_core::now(),
        },
        agents: vec![AgentSummary {
            agent: Agent {
                id: Uuid::new_v4(),
                ticket_id,
                project_id,
                role: "orchestrator".into(),
                objective: "composer l'équipe".into(),
                stage: 0,
                session_id: Uuid::new_v4(),
                model: "haiku".into(),
                effort: Effort::High,
                max_budget_usd: None,
                status: AgentStatus::Done,
                exit_reason: None,
                pid: None,
                pane_id: None,
                attempt: 1,
                handoff: None,
                started_at: Some(orchestra_core::now()),
                ended_at: Some(orchestra_core::now()),
            },
            tokens: Tokens {
                input: 105,
                output: 26,
                cache_read: 400_000,
                cache_creation: 32_000,
                thinking: 10,
            },
            cost_usd: Some(0.097),
            turns: 13,
        }],
        tokens: Tokens {
            input: 105,
            output: 26,
            cache_read: 400_000,
            cache_creation: 32_000,
            thinking: 10,
        },
        cost_usd: Some(0.097),
        recent_events: vec![],
        review: None,
        checks: None,
        pull_request: None,
        merge_blocked: None,
        integration_mode: orchestra_core::config::IntegrationMode::Merge,
    }
}

fn app_on_ticket(with_proposal: bool, with_team: bool) -> App {
    let mut app = App::new();
    app.connected = true;
    app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(detail(with_proposal, with_team)),
    })));
    app.screen = Screen::Ticket;
    app
}

#[test]
fn the_ticket_screen_shows_brief_team_and_cost() {
    let app = app_on_ticket(false, true);
    let out = draw(&app, 120, 30);
    assert!(out.contains("#12"));
    assert!(out.contains("Ajouter un cache"));
    assert!(out.contains("recalcule tout"), "le brief est affiché");
    assert!(out.contains("architect"));
    assert!(out.contains("backend"));
    assert!(out.contains("Équipe acceptée"));
    assert!(out.contains("orchestrator"), "les agents sont listés");
    assert!(
        out.contains("indicatif"),
        "le coût reste annoncé comme estimé"
    );
}

#[test]
fn a_pending_proposal_is_shown_with_its_risks() {
    let app = app_on_ticket(true, false);
    let out = draw(&app, 120, 30);
    assert!(out.contains("Proposition"));
    assert!(out.contains("Deux rôles suffisent"));
    assert!(out.contains("invalidation"), "les risques sont visibles");
    assert!(out.contains("[a] relire"), "la touche est proposée");
}

#[test]
fn a_ticket_without_a_team_says_what_to_press() {
    let app = app_on_ticket(false, false);
    let out = draw(&app, 120, 30);
    assert!(out.contains("pas encore d'équipe"));
    assert!(out.contains("planifier"));
}

#[test]
fn planning_is_visible_while_it_runs() {
    let mut app = app_on_ticket(false, false);
    let cmds = app.update(Msg::Key(Action::Char('p')));
    assert!(
        cmds.iter().any(|c| matches!(c, Command::PlanTicket { .. })),
        "« p » demande une planification"
    );
    let out = draw(&app, 120, 30);
    assert!(out.contains("réfléchit") || out.contains("compose"));
}

/// An event of the open ticket, whoever produced it.
fn ticket_event(app: &App, kind: EventKind) -> Event {
    let mut e = Event::from_new(1, NewEvent::new(kind));
    e.ticket_id = app.ticket.as_ref().map(|d| d.ticket.id);
    e
}

#[test]
fn a_verdict_and_a_fusion_are_seen_without_leaving_the_screen() {
    let mut app = app_on_ticket(false, true);
    let e = ticket_event(
        &app,
        EventKind::ReviewVerdict {
            round: 1,
            verdict: Verdict::Changes,
            blocking: vec!["backend : la boucle".into()],
            roles: vec!["backend".into()],
        },
    );
    let cmds = app.update(Msg::Event(Box::new(e)));
    assert!(app.status.contains("corrections"), "{}", app.status);
    assert!(cmds.iter().any(|c| matches!(c, Command::GetTicket { .. })));

    let e = ticket_event(
        &app,
        EventKind::TicketMerged {
            branch: "orch/12-cache".into(),
            into: "main".into(),
            commits: 3,
            pushed_to: Some("origin".into()),
        },
    );
    let cmds = app.update(Msg::Event(Box::new(e)));
    assert!(app.status.contains("fusionnée"), "{}", app.status);
    assert!(cmds.iter().any(|c| matches!(c, Command::GetTicket { .. })));
}

#[test]
fn the_open_ticket_refreshes_itself_while_it_is_shown() {
    // Everything on this screen — statut, agents, coût, temps écoulé — comes
    // from the ticket. Without the tick it only moved when an event happened
    // to arrive, so a status changed elsewhere was seen by leaving and coming
    // back.
    let mut app = app_on_ticket(false, true);
    let cmds = app.update(Msg::Tick);
    assert!(
        cmds.iter().any(|c| matches!(c, Command::GetTicket { .. })),
        "{cmds:?}"
    );

    // And while watching an agent, so its header stops saying « démarrage ».
    app.update(Msg::Key(Action::Select));
    assert_eq!(app.screen, Screen::Agent);
    let cmds = app.update(Msg::Tick);
    assert!(
        cmds.iter().any(|c| matches!(c, Command::GetTicket { .. })),
        "{cmds:?}"
    );
}

#[test]
fn while_it_thinks_the_screen_says_what_it_is_reading() {
    let mut app = app_on_ticket(false, false);
    let cmds = app.update(Msg::Key(Action::Char('p')));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Subscribe { filter, .. } if filter.ticket_id.is_some() && !filter.exclude_verbose
        )),
        "l'écran suit les événements du ticket, y compris les bavards"
    );

    for kind in [
        EventKind::ToolStarted {
            tool_use_id: "1".into(),
            tool: "Read".into(),
            summary: "src/store/rows.rs".into(),
        },
        EventKind::AgentThinking { chars: 1200 },
        EventKind::ToolStarted {
            tool_use_id: "2".into(),
            tool: "Grep".into(),
            summary: "« cache » dans src/".into(),
        },
    ] {
        let e = ticket_event(&app, kind);
        app.update(Msg::Event(Box::new(e)));
    }

    let out = draw(&app, 120, 30);
    assert!(out.contains("lit src/store/rows.rs"), "{out}");
    assert!(out.contains("réfléchit (1.2k caractères)"), "{out}");
    assert!(out.contains("cherche"), "{out}");
}

#[test]
fn a_refresh_no_longer_stops_the_thinking() {
    // The indicator used to vanish a second after the key was pressed: any
    // ticket refresh cleared it, while the orchestrator ran for another minute.
    let mut app = app_on_ticket(false, false);
    app.update(Msg::Key(Action::Char('p')));
    assert!(app.planning);

    let mut d = detail(false, false);
    d.agents[0].agent.status = AgentStatus::Running;
    d.agents[0].agent.ended_at = None;
    app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(d),
    })));

    assert!(app.planning, "il travaille encore");
    let out = draw(&app, 120, 30);
    assert!(out.contains("réfléchit"), "{out}");
}

#[test]
fn a_finished_orchestrator_run_ends_the_wait() {
    let mut app = app_on_ticket(false, false);
    app.update(Msg::Key(Action::Char('p')));

    // The run ends without the event reaching us: the ticket says so.
    let mut d = detail(false, false);
    d.agents[0].agent.status = AgentStatus::Done;
    d.agents[0].agent.ended_at = Some(orchestra_core::now());
    let cmds = app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(d),
    })));

    assert!(!app.planning);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Subscribe { filter, .. } if filter.exclude_verbose
        )),
        "on revient au flux du tableau"
    );
}

/// A board with one ticket in each column.
fn app_on_kanban() -> App {
    let base = detail(false, false);
    let make = |number: i64, title: &str, status: TicketStatus, pr: Option<&str>| {
        let mut t = base.ticket.clone();
        t.id = Uuid::new_v4();
        t.number = number;
        t.title = title.into();
        t.status = status;
        TicketSummary {
            ticket: t,
            agents_total: 4,
            agents_active: if status == TicketStatus::Running {
                1
            } else {
                0
            },
            agents_done: 3,
            tokens: Tokens::default(),
            cost_usd: Some(1.25),
            pull_request: pr.map(str::to_string),
            merge_blocked: None,
        }
    };
    let mut app = App::new();
    app.connected = true;
    app.update(Msg::Reply(Box::new(Reply::Tickets {
        tickets: vec![
            make(1, "brouillon de test", TicketStatus::Draft, None),
            make(2, "thème clair", TicketStatus::Running, None),
            make(3, "cache mémoire", TicketStatus::Review, None),
            make(
                4,
                "export CSV",
                TicketStatus::Review,
                Some("https://github.com/o/r/pull/4"),
            ),
            make(5, "runbook", TicketStatus::Done, None),
            make(6, "vieille idée", TicketStatus::Cancelled, None),
        ],
    })));
    app.board_pane = orchestra_tui::app::BoardPane::Tickets;
    app
}

#[test]
fn the_board_lays_the_tickets_out_in_columns() {
    let app = app_on_kanban();
    let out = draw(&app, 160, 30);
    for column in [
        "À faire",
        "En cours",
        "À relire",
        "PR à valider",
        "Fusionné",
        "Arrêtés",
    ] {
        assert!(
            out.contains(column),
            "colonne « {column} » absente :\n{out}"
        );
    }
    assert!(out.contains("#4 export CSV"), "{out}");
}

#[test]
fn a_column_nobody_uses_does_not_take_the_room() {
    // Un projet qui n'ouvre jamais de PR et n'annule rien n'a pas à porter deux
    // colonnes vides ; les quatre du flux normal restent, elles.
    let mut app = app_on_kanban();
    app.tickets.retain(|t| {
        !matches!(
            t.ticket.status,
            TicketStatus::Cancelled | TicketStatus::Failed
        ) && t.pull_request.is_none()
    });
    let lanes = app.lanes();
    let titles: Vec<&str> = lanes.iter().map(|(l, _)| l.title_fr()).collect();
    assert_eq!(titles, vec!["À faire", "En cours", "À relire", "Fusionné"]);
}

#[test]
fn the_cursor_walks_the_columns_and_comes_back_to_the_projects() {
    let mut app = app_on_kanban();
    app.ticket_selected = 0;
    assert_eq!(app.selected_lane(), Some((0, 0)));

    app.update(Msg::Key(Action::Right));
    assert_eq!(app.selected_ticket().unwrap().ticket.number, 2, "en cours");
    app.update(Msg::Key(Action::Right));
    app.update(Msg::Key(Action::Right));
    assert_eq!(
        app.selected_ticket().unwrap().ticket.number,
        4,
        "la PR est sa propre colonne"
    );

    // À gauche de la première colonne, on retourne aux projets.
    for _ in 0..4 {
        app.update(Msg::Key(Action::Left));
    }
    app.update(Msg::Key(Action::Left));
    assert_eq!(app.board_pane, orchestra_tui::app::BoardPane::Projects);
}

#[test]
fn up_and_down_stay_inside_a_column() {
    let mut app = app_on_kanban();
    // Deux tickets « à faire », pour avoir de quoi monter et descendre.
    let mut second = app.tickets[0].clone();
    second.ticket.id = Uuid::new_v4();
    second.ticket.number = 7;
    app.tickets.push(second);
    app.ticket_selected = 0;

    app.update(Msg::Key(Action::Down));
    assert_eq!(app.selected_ticket().unwrap().ticket.number, 7);
    app.update(Msg::Key(Action::Down));
    assert_eq!(
        app.selected_ticket().unwrap().ticket.number,
        7,
        "on ne déborde pas sur la colonne suivante"
    );
    app.update(Msg::Key(Action::Up));
    assert_eq!(app.selected_ticket().unwrap().ticket.number, 1);
}

#[test]
fn opening_a_ticket_from_the_board_asks_the_daemon_for_it() {
    let mut app = App::new();
    app.update(Msg::Reply(Box::new(Reply::Projects {
        projects: vec![detail(false, false).project],
    })));
    let summaries = vec![TicketSummary {
        ticket: detail(false, false).ticket,
        agents_total: 0,
        agents_active: 0,
        agents_done: 0,
        tokens: Tokens::default(),
        cost_usd: None,
        pull_request: None,
        merge_blocked: None,
    }];
    app.update(Msg::Reply(Box::new(Reply::Tickets { tickets: summaries })));
    app.update(Msg::Key(Action::Right));
    let cmds = app.update(Msg::Key(Action::Select));
    assert!(cmds.iter().any(|c| matches!(c, Command::GetTicket { .. })));
    assert_eq!(app.screen, Screen::Ticket);
}

#[test]
fn the_new_ticket_form_types_and_refuses_an_empty_brief() {
    let mut app = App::new();
    app.update(Msg::Reply(Box::new(Reply::Projects {
        projects: vec![detail(false, false).project],
    })));
    app.update(Msg::Key(Action::Char('n')));
    assert_eq!(app.screen, Screen::NewTicket);
    assert!(app.is_typing(), "le formulaire capte les touches");

    for c in "Ajouter un cache".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    assert_eq!(app.form.title, "Ajouter un cache");

    let cmds = app.update(Msg::Key(Action::Accept));
    assert!(cmds.is_empty(), "un brief vide ne crée rien");
    let out = draw(&app, 100, 24);
    assert!(out.contains("Brief"));
    assert!(out.contains('⚠'), "l'erreur est visible");

    app.update(Msg::Key(Action::NextField));
    assert_eq!(app.form_field, TicketField::Brief);
    for c in "Le rendu recalcule tout à chaque frappe.".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    app.update(Msg::Key(Action::Submit));
    assert!(
        app.form.brief.ends_with('\n'),
        "Entrée écrit une nouvelle ligne"
    );

    let cmds = app.update(Msg::Key(Action::Accept));
    assert!(matches!(
        cmds.first(),
        Some(Command::CreateTicket { title, .. }) if title == "Ajouter un cache"
    ));
    assert_eq!(app.screen, Screen::Board, "on revient au tableau");
}

#[test]
fn escaping_the_form_keeps_nothing_and_goes_back() {
    let mut app = App::new();
    app.update(Msg::Reply(Box::new(Reply::Projects {
        projects: vec![detail(false, false).project],
    })));
    app.update(Msg::Key(Action::Char('n')));
    for c in "brouillon".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    app.update(Msg::Key(Action::Cancel));
    assert_eq!(app.screen, Screen::Board);
    assert!(app.form.title.is_empty());
}

#[test]
fn the_team_editor_adjusts_and_accepts() {
    let mut app = app_on_ticket(true, false);
    app.update(Msg::Reply(Box::new(Reply::Roles {
        roles: vec![role("architect"), role("backend"), role("tests")],
        errors: vec![],
    })));

    app.update(Msg::Key(Action::Char('a')));
    assert_eq!(app.screen, Screen::Proposal);
    let out = draw(&app, 120, 30);
    assert!(out.contains("architect"));
    assert!(out.contains("ordre :"), "l'ordre d'exécution est montré");
    assert!(out.contains("objectif détaillé"));

    // A long summary must not push the order line out of its pane: that line
    // is the one saying whether the team can run at all.
    app.editor.summary = "phrase très longue. ".repeat(40);
    let crowded = draw(&app, 120, 30);
    assert!(
        crowded.contains("ordre :"),
        "l'ordre doit rester visible même avec un résumé bavard"
    );

    app.update(Msg::Key(Action::Char('m')));
    assert!(app.editor.members[0].model.is_some());

    app.update(Msg::Key(Action::Char('a')));
    assert_eq!(app.editor.members.len(), 3);

    let cmds = app.update(Msg::Key(Action::Char('y')));
    assert!(matches!(
        cmds.first(),
        Some(Command::AcceptProposal { team, .. })
            if team.members.len() == 3 && !team.stages.is_empty()
    ));
    assert_eq!(app.screen, Screen::Ticket);
}

#[test]
fn an_invalid_team_is_refused_with_its_reason() {
    let mut app = app_on_ticket(true, false);
    // A catalog missing `backend` makes the proposal invalid.
    app.update(Msg::Reply(Box::new(Reply::Roles {
        roles: vec![role("architect")],
        errors: vec![],
    })));
    app.update(Msg::Key(Action::Char('a')));

    let out = draw(&app, 120, 30);
    assert!(out.contains('⚠'), "l'écran prévient avant d'accepter");
    assert!(out.contains("backend"), "le rôle fautif est nommé");

    let cmds = app.update(Msg::Key(Action::Char('y')));
    assert!(cmds.is_empty(), "rien n'est envoyé tant que c'est invalide");
    assert_eq!(app.screen, Screen::Proposal, "on reste sur l'écran");
}

#[test]
fn an_objective_can_be_rewritten_in_place() {
    let mut app = app_on_ticket(true, false);
    app.update(Msg::Reply(Box::new(Reply::Roles {
        roles: vec![role("architect"), role("backend")],
        errors: vec![],
    })));
    app.update(Msg::Key(Action::Char('a')));

    app.update(Msg::Key(Action::Char('o')));
    assert!(app.is_typing(), "l'éditeur capte les touches");
    let out = draw(&app, 120, 30);
    assert!(out.contains("Ctrl-S garder"));

    for _ in 0..200 {
        app.update(Msg::Key(Action::Backspace));
    }
    for c in "écrire le cache".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    app.update(Msg::Key(Action::Accept));
    assert!(!app.is_typing());
    assert_eq!(app.editor.members[0].objective, "écrire le cache");
}

#[test]
fn going_back_walks_the_screens_rather_than_quitting() {
    let mut app = app_on_ticket(true, false);
    app.update(Msg::Reply(Box::new(Reply::Roles {
        roles: vec![role("architect"), role("backend")],
        errors: vec![],
    })));
    app.update(Msg::Key(Action::Char('a')));
    assert_eq!(app.screen, Screen::Proposal);

    app.update(Msg::Key(Action::Back));
    assert_eq!(app.screen, Screen::Ticket);
    assert!(!app.should_quit);

    app.update(Msg::Key(Action::Back));
    assert_eq!(app.screen, Screen::Board);
    assert!(!app.should_quit);

    app.update(Msg::Key(Action::Back));
    assert!(app.should_quit, "depuis le tableau, on quitte");
}

#[test]
fn a_proposal_event_reloads_the_open_ticket() {
    let mut app = app_on_ticket(false, false);
    app.planning = true;
    let ticket_id = app.ticket.as_ref().unwrap().ticket.id;

    let mut event = Event::from_new(
        1,
        NewEvent::new(EventKind::ProposalReady {
            proposal: Box::new(TeamProposal {
                summary: "s".into(),
                members: vec![member("backend", &[])],
                risks: vec![],
                estimated_size: Size::S,
            }),
        }),
    );
    event.ticket_id = Some(ticket_id);

    let cmds = app.update(Msg::Event(Box::new(event)));
    assert!(!app.planning, "la planification est terminée");
    assert!(cmds.iter().any(|c| matches!(c, Command::GetTicket { .. })));
    assert!(app.status.contains("proposition"), "{}", app.status);
}

#[test]
fn a_failed_proposal_says_why() {
    let mut app = app_on_ticket(false, false);
    app.planning = true;
    let mut event = Event::from_new(
        1,
        NewEvent::new(EventKind::ProposalFailed {
            error: "budget dépassé".into(),
        }),
    );
    event.ticket_id = Some(app.ticket.as_ref().unwrap().ticket.id);
    app.update(Msg::Event(Box::new(event)));
    assert!(!app.planning);
    assert!(app.status.contains("budget dépassé"), "{}", app.status);
}

#[test]
fn every_screen_survives_a_narrow_pane() {
    let mut app = app_on_ticket(true, true);
    app.update(Msg::Reply(Box::new(Reply::Roles {
        roles: vec![role("architect"), role("backend")],
        errors: vec![],
    })));
    for screen in [Screen::Ticket, Screen::NewTicket, Screen::Proposal] {
        app.screen = screen;
        for (w, h) in [(60u16, 20u16), (40, 12), (20, 8), (200, 60)] {
            let _ = draw(&app, w, h);
        }
    }
}

fn agent_event(app: &App, kind: EventKind) -> Event {
    let mut e = Event::from_new(1, NewEvent::new(kind));
    e.agent_id = app.watched_agent_id();
    e.ticket_id = app.ticket.as_ref().map(|d| d.ticket.id);
    e
}

fn app_watching_agent() -> App {
    let mut app = app_on_ticket(false, true);
    app.update(Msg::Key(Action::Select));
    app
}

/// A ticket with the orchestrator first, then a team agent.
fn detail_with_team_agent(team_status: AgentStatus) -> TicketDetail {
    let mut d = detail(false, true);
    let base = d.agents[0].clone();
    let mut worker = base.clone();
    worker.agent.id = Uuid::new_v4();
    worker.agent.role = "frontend".into();
    worker.agent.status = team_status;
    worker.agent.started_at = Some(orchestra_core::now());
    // The orchestrator comes first, as the store returns it.
    d.agents = vec![base, worker];
    d
}

#[test]
fn opening_a_ticket_lands_on_the_agent_that_is_working() {
    // Watching the orchestrator while the real agent runs looks exactly like
    // a frozen screen; that is what happened on the first real run.
    let mut app = App::new();
    app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(detail_with_team_agent(AgentStatus::Running)),
    })));
    app.screen = Screen::Ticket;

    app.update(Msg::Key(Action::Select));
    assert_eq!(app.screen, Screen::Agent);
    assert_eq!(
        app.watched_agent().unwrap().agent.role,
        "frontend",
        "on ouvre l'agent en cours, pas l'orchestrateur"
    );
}

#[test]
fn without_a_running_agent_the_last_worker_is_shown() {
    let mut app = App::new();
    app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(detail_with_team_agent(AgentStatus::Done)),
    })));
    app.screen = Screen::Ticket;
    app.update(Msg::Key(Action::Select));
    assert_eq!(app.watched_agent().unwrap().agent.role, "frontend");
}

#[test]
fn a_deliberate_choice_of_agent_is_respected() {
    let mut app = App::new();
    app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(detail_with_team_agent(AgentStatus::Running)),
    })));
    app.screen = Screen::Ticket;
    // The user moves to the orchestrator on purpose.
    app.update(Msg::Key(Action::Up));
    assert_eq!(app.agent_selected, 0);
    app.update(Msg::Key(Action::Select));
    assert_eq!(
        app.watched_agent().unwrap().agent.role,
        "frontend",
        "sans agent actif sélectionné, on suit celui qui travaille"
    );
}

#[test]
fn on_a_finished_ticket_the_chosen_agent_is_the_one_that_opens() {
    // Three agents, all done: whichever row the user picked, the screen used
    // to open the last one that ran.
    let mut app = App::new();
    let mut d = detail_with_team_agent(AgentStatus::Done);
    let mut docs = d.agents[1].clone();
    docs.agent.id = Uuid::new_v4();
    docs.agent.role = "docs".into();
    docs.agent.status = AgentStatus::Done;
    docs.agent.started_at = Some(orchestra_core::now());
    d.agents.push(docs);
    app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(d),
    })));
    app.screen = Screen::Ticket;

    // Sans rien choisir, le curseur est sur le dernier agent : c'est lui qui
    // a parlé en dernier. L'utilisateur remonte exprès sur « frontend ».
    assert_eq!(app.watched_agent().unwrap().agent.role, "docs");
    app.update(Msg::Key(Action::Up));
    app.update(Msg::Key(Action::Select));
    assert_eq!(app.watched_agent().unwrap().agent.role, "frontend");

    // And a refresh does not drag the screen back to the last agent.
    let mut again = detail_with_team_agent(AgentStatus::Done);
    let mut docs = again.agents[1].clone();
    docs.agent.id = app.ticket.as_ref().unwrap().agents[2].agent.id;
    docs.agent.role = "docs".into();
    docs.agent.status = AgentStatus::Done;
    again.agents[1].agent.id = app.ticket.as_ref().unwrap().agents[1].agent.id;
    again.agents.push(docs);
    app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(again),
    })));
    assert_eq!(app.watched_agent().unwrap().agent.role, "frontend");
}

#[test]
fn a_long_list_of_agents_scrolls_to_the_one_that_works() {
    // Un ticket passé par une relecture et ses corrections a une dizaine
    // d'agents ; le volet n'en montre que six. Celui qui travaille est le
    // dernier, donc invisible : la liste semblait entièrement terminée.
    let mut app = App::new();
    let mut d = detail(false, true);
    let base = d.agents[0].clone();
    d.agents.clear();
    for (i, role) in [
        "frontend",
        "docs",
        "integrator",
        "reviewer",
        "frontend",
        "reviewer",
    ]
    .iter()
    .enumerate()
    {
        let mut a = base.clone();
        a.agent.id = Uuid::new_v4();
        a.agent.role = (*role).into();
        a.agent.status = AgentStatus::Done;
        a.agent.started_at = Some(orchestra_core::now() + time::Duration::minutes(i as i64));
        d.agents.push(a);
    }
    let mut working = base.clone();
    working.agent.id = Uuid::new_v4();
    working.agent.role = "integrator".into();
    working.agent.status = AgentStatus::Running;
    working.agent.started_at = Some(orchestra_core::now() + time::Duration::minutes(10));
    d.agents.push(working);

    app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(d),
    })));
    app.screen = Screen::Ticket;

    let out = draw(&app, 120, 30);
    assert!(
        out.contains("Agents (7)"),
        "le volet dit combien il en cache :\n{out}"
    );
    assert!(
        out.contains("integrator · reprise 1"),
        "l'agent qui travaille est à l'écran :\n{out}"
    );
    assert!(out.contains("en cours"), "{out}");
}

#[test]
fn a_role_that_came_back_after_the_relecture_is_told_apart() {
    let mut app = App::new();
    let mut d = detail_with_team_agent(AgentStatus::Done);
    let mut again = d.agents[1].clone();
    again.agent.id = Uuid::new_v4();
    again.agent.status = AgentStatus::Running;
    d.agents.push(again);
    app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(d),
    })));
    app.screen = Screen::Ticket;
    let out = draw(&app, 120, 30);
    assert!(
        out.contains("frontend · reprise 1"),
        "deux lignes « frontend » identiques seraient illisibles :\n{out}"
    );
}

#[test]
fn the_screen_follows_the_team_from_one_agent_to_the_next() {
    let mut app = App::new();
    app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(detail_with_team_agent(AgentStatus::Running)),
    })));
    app.screen = Screen::Ticket;
    app.update(Msg::Key(Action::Select));
    let first = app.watched_agent_id();

    // The first agent finishes and a second one starts.
    let mut next = detail_with_team_agent(AgentStatus::Done);
    let mut third = next.agents[1].clone();
    third.agent.id = Uuid::new_v4();
    third.agent.role = "tests".into();
    third.agent.status = AgentStatus::Running;
    next.agents.push(third);
    let cmds = app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(next),
    })));

    assert_eq!(app.watched_agent().unwrap().agent.role, "tests");
    assert_ne!(app.watched_agent_id(), first);
    assert!(
        cmds.iter().any(|c| matches!(c, Command::Subscribe { .. })),
        "l'écran s'abonne au nouvel agent"
    );
    assert!(app.log.is_empty(), "le log repart propre");
}

#[test]
fn opening_an_agent_subscribes_to_it_and_clears_the_log() {
    let mut app = app_on_ticket(false, true);
    let cmds = app.update(Msg::Key(Action::Select));
    assert_eq!(app.screen, Screen::Agent);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Subscribe { filter, .. } if filter.agent_id == app.watched_agent_id()
        )),
        "l'écran s'abonne aux événements de cet agent"
    );
    assert!(app.log.is_empty());
}

#[test]
fn the_agent_screen_reads_like_a_conversation() {
    let mut app = app_watching_agent();
    for kind in [
        EventKind::AgentSpawned {
            role: "backend".into(),
            session_id: Uuid::new_v4(),
            pid: 4242,
            cmdline: "claude -p".into(),
        },
        EventKind::AgentText {
            text: "Je lance les tests.".into(),
        },
        EventKind::ToolStarted {
            tool_use_id: "t1".into(),
            tool: "Bash".into(),
            summary: "cargo test --workspace".into(),
        },
        EventKind::ToolFinished {
            tool_use_id: "t1".into(),
            ok: true,
            summary: "19 passed".into(),
        },
    ] {
        let e = agent_event(&app, kind);
        app.update(Msg::Event(Box::new(e)));
    }

    let out = draw(&app, 120, 30);
    assert!(out.contains("Je lance les tests."));
    assert!(out.contains("cargo test --workspace"));
    // The panel title says where the run stands and how much there is to read.
    assert!(
        out.contains("ligne(s)"),
        "le titre annonce la taille du journal"
    );
    assert!(out.contains("backend"));
}

#[test]
fn a_running_agent_says_so_and_shows_how_long() {
    let mut app = App::new();
    let mut detail = detail_with_team_agent(AgentStatus::Running);
    detail.agents[1].agent.started_at = Some(orchestra_core::now() - time::Duration::seconds(95));
    app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(detail),
    })));
    app.screen = Screen::Ticket;
    app.update(Msg::Key(Action::Select));

    let out = draw(&app, 120, 30);
    assert!(out.contains("en cours"), "le statut réel est affiché");
    // The exact second moves between building the fixture and drawing it, so
    // the assertion is on the shape, not the value.
    assert!(out.contains("1 min"), "et depuis combien de temps : {out}");
    assert!(out.contains("En direct"), "le journal se dit en direct");
}

#[test]
fn a_long_silence_is_named_rather_than_laissed_ambiguous() {
    let mut app = App::new();
    app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(detail_with_team_agent(AgentStatus::Running)),
    })));
    app.screen = Screen::Ticket;
    app.update(Msg::Key(Action::Select));

    // An event from a while ago: the screen should say how long it has been
    // quiet rather than look frozen.
    let mut event = agent_event(
        &app,
        EventKind::AgentText {
            text: "bonjour".into(),
        },
    );
    event.ts = orchestra_core::now() - time::Duration::seconds(40);
    app.update(Msg::Event(Box::new(event)));

    let out = draw(&app, 120, 30);
    assert!(out.contains("silencieux depuis"), "{out}");
}

#[test]
fn a_blocked_call_is_shown_as_such() {
    let mut app = app_watching_agent();
    let e = agent_event(
        &app,
        EventKind::HookBlocked {
            tool: "Bash".into(),
            reason: "« /home/u/ailleurs » est hors du worktree".into(),
        },
    );
    app.update(Msg::Event(Box::new(e)));
    let out = draw(&app, 120, 30);
    assert!(out.contains("bloqué"));
    assert!(out.contains("hors du worktree"));
}

#[test]
fn reasoning_shows_as_a_size_never_as_text() {
    let mut app = app_watching_agent();
    let e = agent_event(&app, EventKind::AgentThinking { chars: 2400 });
    app.update(Msg::Event(Box::new(e)));
    let out = draw(&app, 120, 30);
    assert!(out.contains("réfléchit"));
    assert!(out.contains("2.4k"));
}

#[test]
fn steering_types_then_sends() {
    let mut app = app_watching_agent();
    // The agent must be running for the key to do anything.
    if let Some(detail) = app.ticket.as_mut() {
        detail.agents[0].agent.status = AgentStatus::Running;
    }

    app.update(Msg::Key(Action::Char('s')));
    assert!(app.is_typing(), "la saisie capte les touches");
    // `j` must type here, not scroll.
    for c in "ajoute un test".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    let out = draw(&app, 120, 30);
    assert!(out.contains("ajoute un test"));
    assert!(out.contains("Consigne"));

    let cmds = app.update(Msg::Key(Action::Accept));
    assert!(matches!(
        cmds.first(),
        Some(Command::SteerAgent { text, hard: false, .. }) if text == "ajoute un test"
    ));
    assert!(!app.is_typing());
}

#[test]
fn a_hard_redirect_is_distinguished_from_a_queued_message() {
    let mut app = app_watching_agent();
    if let Some(detail) = app.ticket.as_mut() {
        detail.agents[0].agent.status = AgentStatus::Running;
    }
    app.update(Msg::Key(Action::Char('S')));
    let out = draw(&app, 120, 30);
    assert!(out.contains("Rediriger"));
    for c in "stop".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    let cmds = app.update(Msg::Key(Action::Accept));
    assert!(matches!(
        cmds.first(),
        Some(Command::SteerAgent { hard: true, .. })
    ));
}

#[test]
fn a_finished_agent_offers_no_steering() {
    let mut app = app_watching_agent();
    // The fixture's agent is done.
    app.update(Msg::Key(Action::Char('s')));
    assert!(!app.is_typing(), "on ne pilote pas un agent terminé");
    let cmds = app.update(Msg::Key(Action::Char('x')));
    assert!(cmds.is_empty(), "ni ne l'annule");
    let out = draw(&app, 120, 30);
    assert!(
        !out.contains("s consigne"),
        "les touches ne sont pas proposées"
    );
}

#[test]
fn cancelling_a_running_agent_asks_the_daemon_once_confirmed() {
    let mut app = app_watching_agent();
    if let Some(detail) = app.ticket.as_mut() {
        detail.agents[0].agent.status = AgentStatus::Running;
    }
    assert!(app.update(Msg::Key(Action::Char('x'))).is_empty());
    let cmds = app.update(Msg::Key(Action::Char('o')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::CancelAgent { .. })));
}

#[test]
fn launching_a_ticket_goes_through_the_ticket_screen() {
    let mut app = app_on_ticket(false, true);
    let cmds = app.update(Msg::Key(Action::Char('L')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::LaunchTicket { .. })));
}

#[test]
fn leaving_an_agent_returns_to_the_whole_ticket() {
    let mut app = app_watching_agent();
    let cmds = app.update(Msg::Key(Action::Back));
    assert_eq!(app.screen, Screen::Ticket);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Subscribe { filter, .. } if filter.agent_id.is_none()
        )),
        "l'abonnement redevient global"
    );
}

#[test]
fn typing_exit_out_of_reflex_cancels_nothing() {
    // `x` sits inside the word someone types to leave a program, and it is
    // the cancel key. Nothing irreversible may go through unanswered.
    let mut app = app_on_ticket(false, true);
    let mut sent = Vec::new();
    for c in "exit".chars() {
        sent.extend(app.update(Msg::Key(Action::Char(c))));
    }
    assert!(
        !sent
            .iter()
            .any(|c| matches!(c, Command::CancelTicket { .. })),
        "un ticket ne s'annule pas par réflexe"
    );
    // The `x` raises the question and the `i` that follows declines it.
    assert!(app.confirm.is_none());
    assert_eq!(app.screen, Screen::Ticket, "et rien n'a bougé");

    // On its own, `x` does ask, naming what it would stop.
    app.update(Msg::Key(Action::Char('x')));
    let out = draw(&app, 120, 30);
    assert!(out.contains("Confirmer"));
    assert!(
        out.contains("#12"),
        "la question nomme ce qu'elle va arrêter"
    );
}

/// A ticket back from its team, with the verdict the relecture rendered.
fn app_on_reviewed_ticket(verdict: Verdict) -> App {
    let mut d = detail(false, true);
    d.ticket.status = TicketStatus::Review;
    d.ticket.branch = Some("orch/12-cache".into());
    d.review = Some(ReviewOutcome {
        round: 1,
        verdict,
        blocking: match verdict {
            Verdict::Ready => vec![],
            Verdict::Changes => vec!["backend : la boucle".into()],
        },
    });
    let mut app = App::new();
    app.connected = true;
    app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(d),
    })));
    app.screen = Screen::Ticket;
    app
}

#[test]
fn a_cleared_ticket_offers_the_integration_and_asks_before_merging() {
    let mut app = app_on_reviewed_ticket(Verdict::Ready);
    assert!(app.can_integrate());
    let out = draw(&app, 120, 30);
    assert!(
        out.contains("[f] intégrer"),
        "la touche est annoncée :\n{out}"
    );
    assert!(
        out.contains("rien ne bloque"),
        "le verdict est lisible :\n{out}"
    );

    let cmds = app.update(Msg::Key(Action::Char('f')));
    assert!(cmds.is_empty(), "une fusion ne part pas sans confirmation");
    assert!(app.confirm.is_some());
    let cmds = app.update(Msg::Key(Action::Char('o')));
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::IntegrateTicket { .. })),
        "« o » lance l'intégrateur"
    );
}

#[test]
fn in_pull_request_mode_the_key_says_what_it_does() {
    let mut app = app_on_reviewed_ticket(Verdict::Ready);
    if let Some(d) = app.ticket.as_mut() {
        d.integration_mode = orchestra_core::config::IntegrationMode::Pr;
    }
    let out = draw(&app, 120, 30);
    assert!(out.contains("[f] ouvrir la pull request"), "{out}");

    app.update(Msg::Key(Action::Char('f')));
    let question = app.confirm.as_ref().map(|c| c.question.clone()).unwrap();
    assert!(question.contains("pull request"), "{question}");
}

#[test]
fn a_request_already_waiting_is_shown_and_not_reopened() {
    let mut app = app_on_reviewed_ticket(Verdict::Ready);
    if let Some(d) = app.ticket.as_mut() {
        d.integration_mode = orchestra_core::config::IntegrationMode::Pr;
        d.pull_request = Some("https://github.com/o/r/pull/12".into());
    }
    assert!(
        !app.can_integrate(),
        "elle attend son humain, pas une deuxième ouverture"
    );
    let out = draw(&app, 130, 30);
    assert!(out.contains("PR ouverte"), "{out}");
    assert!(out.contains("pull/12"), "{out}");
}

#[test]
fn a_refused_merge_says_why_and_offers_to_retry_it() {
    let mut app = app_on_reviewed_ticket(Verdict::Ready);
    if let Some(d) = app.ticket.as_mut() {
        d.merge_blocked = Some(
            "le dépôt principal a 1 fichier(s) modifié(s) non commité(s) (docker-compose.yml)"
                .into(),
        );
    }
    assert!(app.can_integrate(), "la branche est prête : on peut réessayer");
    let out = draw(&app, 140, 30);
    assert!(out.contains("⏸ fusion en attente"), "{out}");
    assert!(out.contains("docker-compose.yml"), "{out}");
    assert!(out.contains("[f] réessayer la fusion"), "{out}");

    app.update(Msg::Key(Action::Char('f')));
    let question = app.confirm.as_ref().map(|c| c.question.clone()).unwrap();
    assert!(question.contains("Réessayer la fusion"), "{question}");
}

#[test]
fn a_ticket_the_relecture_blocks_does_not_offer_the_integration() {
    let mut app = app_on_reviewed_ticket(Verdict::Changes);
    assert!(!app.can_integrate());
    let out = draw(&app, 120, 30);
    assert!(!out.contains("[f] intégrer"), "{out}");
    assert!(out.contains("corrections demandées"), "{out}");

    let cmds = app.update(Msg::Key(Action::Char('f')));
    assert!(cmds.is_empty());
    assert!(app.confirm.is_none(), "la touche ne fait rien");
}

#[test]
fn a_closed_ticket_offers_to_be_reopened() {
    let mut app = app_on_reviewed_ticket(Verdict::Ready);
    if let Some(d) = app.ticket.as_mut() {
        d.ticket.status = TicketStatus::Done;
    }
    let out = draw(&app, 120, 30);
    assert!(out.contains("[o] rouvrir"), "{out}");
    assert!(
        !out.contains("[f] ouvrir"),
        "un ticket fermé ne s'intègre pas"
    );

    app.update(Msg::Key(Action::Char('o')));
    assert!(app.confirm.is_some(), "on demande avant");
    let cmds = app.update(Msg::Key(Action::Char('o')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::ReopenTicket { .. })));
}

#[test]
fn a_branch_merged_by_hand_is_closed_from_the_ticket() {
    let mut app = app_on_reviewed_ticket(Verdict::Changes);
    let out = draw(&app, 120, 30);
    assert!(out.contains("[t] marquer terminé"), "{out}");
    app.update(Msg::Key(Action::Char('t')));
    assert!(app.confirm.is_some());
    let cmds = app.update(Msg::Key(Action::Char('o')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::FinishTicket { .. })));
}

#[test]
fn a_cancellation_happens_once_it_is_confirmed() {
    let mut app = app_on_ticket(false, true);
    app.update(Msg::Key(Action::Char('x')));
    assert!(app.confirm.is_some());

    let cmds = app.update(Msg::Key(Action::Char('o')));
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::CancelTicket { .. })),
        "« o » confirme"
    );
    assert!(app.confirm.is_none());
}

#[test]
fn any_other_key_declines_the_question() {
    let mut app = app_on_ticket(false, true);
    app.update(Msg::Key(Action::Char('x')));
    let cmds = app.update(Msg::Key(Action::Cancel));
    assert!(cmds.is_empty());
    assert!(app.confirm.is_none());
    assert!(app.status.contains("abandon"), "{}", app.status);

    // And the screen has not moved.
    assert_eq!(app.screen, Screen::Ticket);
}

#[test]
fn stopping_an_agent_also_asks_first() {
    let mut app = App::new();
    app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(detail_with_team_agent(AgentStatus::Running)),
    })));
    app.screen = Screen::Ticket;
    app.update(Msg::Key(Action::Select));

    let cmds = app.update(Msg::Key(Action::Char('x')));
    assert!(cmds.is_empty());
    let question = &app.confirm.as_ref().unwrap().question;
    assert!(question.contains("frontend"), "{question}");

    let cmds = app.update(Msg::Key(Action::Select));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::CancelAgent { .. })));
}

#[test]
fn how_to_leave_is_written_on_the_screen() {
    // Someone who does not know the key types "exit"; the answer should be
    // in front of them.
    let mut board = App::new();
    board.connected = true;
    assert!(draw(&board, 120, 30).contains("[q] quitter"));

    // Un écran chargé n'a pas la place de tout écrire : ce qui reste, coûte
    // que coûte, c'est « ? », et l'aide, elle, dit comment sortir.
    let mut app = app_on_ticket(false, true);
    assert!(draw(&app, 120, 30).contains("[?] aide"));
    app.update(Msg::Key(Action::Help));
    let help = draw(&app, 120, 30);
    assert!(help.contains("[Q] quitter"), "{help}");
}

#[test]
fn keys_without_meaning_do_nothing() {
    let mut app = app_on_ticket(false, false);
    let before = app.screen;
    for c in ['z', 'w', '§'] {
        let cmds = app.update(Msg::Key(Action::Char(c)));
        assert!(cmds.is_empty());
    }
    assert_eq!(app.screen, before);
}

#[test]
fn the_key_bar_holds_one_line_and_never_loses_the_way_out() {
    // L'écran Ticket est le plus chargé : c'est là que la coupe se joue.
    for width in [60u16, 90, 120, 200] {
        let app = app_on_reviewed_ticket(Verdict::Ready);
        let out = draw(&app, width, 30);
        let lines: Vec<&str> = out.lines().collect();
        let bar = lines[lines.len() - 2];
        assert!(
            bar.contains("[?] aide") || bar.contains("[q] retour"),
            "à {width} colonnes, plus rien pour s'en sortir :\n{bar}"
        );
        assert!(
            bar.chars().count() <= width as usize,
            "le bandeau déborde à {width}"
        );
        // Ce que l'état du ticket rend possible passe avant le reste.
        assert!(
            bar.contains("[f] intégrer"),
            "la décision du moment a sauté à {width} :\n{bar}"
        );
    }
}

#[test]
fn the_keys_are_written_once_not_twice() {
    // Les touches vivent dans le bandeau ; un écran qui les redit en plus
    // ferait deux listes qui divergent.
    let app = app_on_ticket(false, true);
    let out = draw(&app, 120, 30);
    assert_eq!(
        out.matches("[p] planifier").count(),
        1,
        "« p » est annoncé deux fois :\n{out}"
    );
}

#[test]
fn the_help_leads_with_the_screen_one_is_on() {
    let mut app = app_on_ticket(false, true);
    app.update(Msg::Key(Action::Help));
    let out = draw(&app, 120, 30);
    assert!(out.contains("Aide — Ticket"), "{out}");
    assert!(out.contains("Sur cet écran"), "{out}");
    assert!(out.contains("[L] lancer"), "{out}");
    // Les touches d'un autre écran n'ont rien à faire là.
    assert!(!out.contains("sessions libres"), "{out}");
    assert!(out.contains("[Q] quitter"), "la sortie est écrite : {out}");
}

#[test]
fn a_form_does_not_advertise_keys_that_would_be_typed() {
    let mut app = App::new();
    app.connected = true;
    app.screen = Screen::NewTicket;
    let out = draw(&app, 100, 24);
    let lines: Vec<&str> = out.lines().collect();
    let bar = lines[lines.len() - 2];
    assert!(bar.contains("[Ctrl-S] créer"), "{bar}");
    assert!(
        !bar.contains("[?] aide"),
        "« ? » s'écrirait dans le champ : {bar}"
    );
}

fn red_check() -> orchestra_core::checks::ChecksOutcome {
    orchestra_core::checks::ChecksOutcome {
        round: 1,
        runs: vec![orchestra_core::checks::CheckRun {
            command: "cargo test --workspace".into(),
            ok: false,
            code: Some(101),
            duration_ms: 12_000,
            tail: "error[E0308]: mismatched types\nerror: could not compile".into(),
        }],
    }
}

#[test]
fn a_branch_its_own_checks_refuse_is_not_offered_for_integration() {
    // La relecture peut très bien dire « prêt » : ce que la machine a mesuré
    // passe avant ce qu'un agent a conclu.
    let mut app = app_on_reviewed_ticket(Verdict::Ready);
    if let Some(d) = app.ticket.as_mut() {
        d.checks = Some(red_check());
    }
    assert!(!app.can_integrate(), "un build rouge ferme la porte");
    let out = draw(&app, 120, 30);
    assert!(!out.contains("[f] intégrer"), "{out}");
    assert!(out.contains("cargo test --workspace"), "{out}");
    assert!(
        out.contains("could not compile"),
        "la dernière ligne de la commande dit pourquoi :\n{out}"
    );

    // Et la touche ne fait rien non plus.
    let cmds = app.update(Msg::Key(Action::Char('f')));
    assert!(cmds.is_empty());
    assert!(app.confirm.is_none());
}

#[test]
fn a_green_gate_says_so_and_leaves_the_integration_open() {
    let mut app = app_on_reviewed_ticket(Verdict::Ready);
    if let Some(d) = app.ticket.as_mut() {
        d.checks = Some(orchestra_core::checks::ChecksOutcome {
            round: 1,
            runs: vec![orchestra_core::checks::CheckRun {
                command: "cargo test --workspace".into(),
                ok: true,
                code: Some(0),
                duration_ms: 42_000,
                tail: String::new(),
            }],
        });
    }
    assert!(app.can_integrate());
    let out = draw(&app, 120, 30);
    assert!(out.contains("✓ vérifié"), "{out}");
    assert!(out.contains("[f] intégrer"), "{out}");
}

#[test]
fn a_check_that_refuses_is_said_out_loud() {
    let mut app = app_on_ticket(false, true);
    let event = ticket_event(
        &app,
        EventKind::CheckFinished {
            round: 1,
            run: Box::new(orchestra_core::checks::CheckRun {
                command: "cargo build".into(),
                ok: false,
                code: Some(101),
                duration_ms: 3_000,
                tail: "error: could not compile".into(),
            }),
        },
    );
    app.update(Msg::Event(Box::new(event)));
    assert!(app.status.contains("cargo build"), "{}", app.status);
    assert!(app.status.contains("échoue"), "{}", app.status);
}

/// The app watching one agent of a ticket, in the state it is given.
fn app_watching(status: AgentStatus) -> App {
    let mut app = App::new();
    app.connected = true;
    app.update(Msg::Reply(Box::new(Reply::Ticket {
        detail: Box::new(detail_with_team_agent(status)),
    })));
    app.screen = Screen::Ticket;
    app.update(Msg::Key(Action::Select));
    assert_eq!(app.screen, Screen::Agent);
    app
}

#[test]
fn an_agent_can_be_shown_in_its_own_pane() {
    let mut app = app_watching(AgentStatus::Running);
    let cmds = app.update(Msg::Key(Action::Char('o')));
    assert!(
        cmds.iter().any(|c| matches!(c, Command::OpenPane { .. })),
        "« o » demande le pane : {cmds:?}"
    );
    let out = draw(&app, 120, 30);
    assert!(out.contains("[o] son pane"), "{out}");
}

#[test]
fn taking_over_waits_for_the_agent_to_have_stopped() {
    // Deux mains sur une même session Claude se défont l'une l'autre, donc la
    // touche ne s'annonce ni ne part tant que l'agent tourne.
    let mut app = app_watching(AgentStatus::Running);
    let cmds = app.update(Msg::Key(Action::Char('T')));
    assert!(cmds.is_empty(), "{cmds:?}");
    assert!(!draw(&app, 120, 30).contains("[T] reprendre"));

    let mut app = app_watching(AgentStatus::Failed);
    let cmds = app.update(Msg::Key(Action::Char('T')));
    assert!(
        cmds.iter().any(|c| matches!(c, Command::TakeOver { .. })),
        "un agent arrêté se reprend à la main : {cmds:?}"
    );
    assert!(draw(&app, 120, 30).contains("[T] reprendre la main"));
}

#[test]
fn an_opened_pane_is_named_in_the_status_line() {
    let mut app = app_watching(AgentStatus::Running);
    app.update(Msg::Reply(Box::new(Reply::Pane {
        pane_id: "terminal_7".into(),
    })));
    assert!(app.status.contains("terminal_7"), "{}", app.status);
}
