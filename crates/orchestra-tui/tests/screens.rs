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
use uuid::Uuid;

fn draw(app: &App, w: u16, h: u16) -> String {
    orchestra_tui::screens::text_of(w, h, |f| orchestra_tui::screens::render(app, f))
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
        acceptance: Vec::new(),
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
            attention: None,
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
        attention: None,
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
fn the_team_editor_shows_what_done_means_for_each_member() {
    let mut d = detail(true, false);
    let proposal = d.ticket.proposal.as_mut().unwrap();
    proposal.members[0].acceptance = vec![
        "docs/cache.md décrit l'invalidation".into(),
        "le plan nomme les fichiers touchés".into(),
    ];
    let mut app = App::new();
    app.connected = true;
    app.update(Msg::Reply(Box::new(Reply::Ticket { detail: Box::new(d) })));
    app.screen = Screen::Ticket;
    app.update(Msg::Reply(Box::new(Reply::Roles {
        roles: vec![role("architect"), role("backend")],
        errors: vec![],
    })));
    app.update(Msg::Key(Action::Char('a')));
    assert_eq!(app.screen, Screen::Proposal);

    let out = draw(&app, 120, 30);
    assert!(out.contains("fini quand :"), "{out}");
    assert!(out.contains("☐ docs/cache.md décrit l'invalidation"));
    assert!(out.contains("☐ le plan nomme les fichiers touchés"));
    // A small terminal still draws without panicking.
    draw(&app, 50, 16);
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

    app.update(Msg::Key(Action::Char('e')));
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
    let cmds = app.update(Msg::Key(Action::Char('y')));
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
fn tabbing_away_from_an_agent_stops_following_it() {
    let mut app = app_watching_agent();
    let cmds = app.update(Msg::Key(Action::Screen(1)));
    assert_eq!(app.screen, Screen::Board);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Subscribe { filter, .. } if filter.agent_id.is_none()
        )),
        "le tableau ne doit pas rester filtré sur un seul agent"
    );
}

#[test]
fn tabbing_onto_the_agent_screen_follows_the_agent_of_the_ticket() {
    let mut app = app_on_ticket(false, true);
    app.log.push_event(&Event::from_new(
        1,
        NewEvent::new(EventKind::AgentText { text: "reste d'un autre agent".into() }),
    ));
    assert!(!app.log.is_empty());
    let cmds = app.update(Msg::Key(Action::NextScreen));
    assert_eq!(app.screen, Screen::Agent);
    let watched = app.watched_agent_id();
    assert!(watched.is_some());
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Subscribe { filter, .. } if filter.agent_id == watched
        )),
        "l'écran s'abonne à l'agent suivi"
    );
    assert!(app.log.is_empty(), "le journal d'un autre agent ne reste pas affiché");
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
    let cmds = app.update(Msg::Key(Action::Char('y')));
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::IntegrateTicket { .. })),
        "« y » lance l'intégrateur"
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
    let cmds = app.update(Msg::Key(Action::Char('y')));
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
    let cmds = app.update(Msg::Key(Action::Char('y')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::FinishTicket { .. })));
}

#[test]
fn a_cancellation_happens_once_it_is_confirmed() {
    let mut app = app_on_ticket(false, true);
    app.update(Msg::Key(Action::Char('x')));
    assert!(app.confirm.is_some());

    let cmds = app.update(Msg::Key(Action::Char('y')));
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::CancelTicket { .. })),
        "« y » confirme"
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

/// A board of four tickets, three of which wait on the user.
fn board_waiting() -> App {
    use orchestra_core::attention::Attention;
    let base = detail(false, false);
    let make = |number: i64, title: &str, status: TicketStatus, attention: Option<Attention>| {
        let mut t = base.ticket.clone();
        t.id = Uuid::new_v4();
        t.number = number;
        t.title = title.into();
        t.status = status;
        TicketSummary {
            ticket: t,
            agents_total: 0,
            agents_active: 0,
            agents_done: 0,
            tokens: Tokens::default(),
            cost_usd: None,
            pull_request: None,
            merge_blocked: None,
            attention,
        }
    };
    let mut app = App::new();
    app.connected = true;
    app.update(Msg::Reply(Box::new(Reply::Tickets {
        tickets: vec![
            make(1, "équipe proposée", TicketStatus::Draft, Some(Attention::ProposalReady)),
            make(2, "au travail", TicketStatus::Running, None),
            make(3, "branche refusée", TicketStatus::Review, Some(Attention::MergeBlocked)),
            make(4, "relu sans défaut", TicketStatus::Review, Some(Attention::ReadyToIntegrate)),
        ],
    })));
    app
}

#[test]
fn the_board_counts_and_marks_what_waits_on_the_user() {
    let app = board_waiting();
    let out = draw(&app, 140, 30);
    assert!(out.contains("⚑ 3 à toi"), "le compte, avec le symbole du plus urgent : {out}");
    assert!(out.contains("⚑ fusion bloquée"));
    assert!(out.contains("▶ équipe à relire"));
    assert!(out.contains("▶ prêt à intégrer"));
    assert!(out.contains("[!] à toi (3)"));
}

#[test]
fn bang_walks_the_queue_most_urgent_first_and_comes_back_round() {
    let mut app = board_waiting();
    app.screen = Screen::Cost;
    let mut seen = Vec::new();
    for _ in 0..4 {
        app.update(Msg::Key(Action::Char('!')));
        assert_eq!(app.screen, Screen::Board, "le saut ramène au tableau");
        seen.push(app.selected_ticket().unwrap().ticket.number);
    }
    assert_eq!(seen, vec![3, 1, 4, 3], "un problème passe avant une étape prête");
    assert!(app.status.contains("fusion bloquée"), "{}", app.status);

    let mut calm = App::new();
    calm.update(Msg::Key(Action::Char('!')));
    assert_eq!(calm.status, "rien ne t'attend");
}

#[test]
fn a_short_terminal_keeps_its_rows_for_the_screen() {
    let mut app = board_waiting();
    let rows_of = |out: &str| out.lines().position(|l| l.contains("Activité"));
    let tall = draw(&app, 140, 40);
    let short = draw(&app, 140, 24);
    // The strip shrinks on 24 rows: it starts lower, relative to the height.
    assert_eq!(rows_of(&tall), Some(40 - 2 - 7));
    assert_eq!(rows_of(&short), Some(24 - 2 - 4));

    app.update(Msg::Key(Action::Char('A')));
    assert!(app.activity_hidden);
    assert!(!draw(&app, 140, 24).contains("Activité"), "replié sur demande");
    app.update(Msg::Key(Action::Char('A')));
    assert!(draw(&app, 140, 24).contains("Activité"));
}

#[test]
fn narrow_tabs_keep_their_numbers_and_the_open_one_its_name() {
    let app = board_waiting();
    let wide = draw(&app, 180, 30);
    let first_line = |out: &str| out.lines().next().unwrap().to_string();
    assert!(first_line(&wide).contains("4 Coût"));
    let narrow = first_line(&draw(&app, 80, 24));
    assert!(!narrow.contains("Coût"), "{narrow}");
    assert!(narrow.contains(" 4 "));
    assert!(narrow.contains("1 Tableau"), "l'écran ouvert garde son nom : {narrow}");
}

#[test]
fn the_two_global_keys_are_not_claimed_by_any_screen() {
    // `!` and `A` work everywhere; a screen that read them for itself would
    // silently win on that screen alone.
    for screen in [
        Screen::Board,
        Screen::Ticket,
        Screen::Agent,
        Screen::Cost,
        Screen::Proposal,
        Screen::Todo,
        Screen::Rules,
    ] {
        let mut app = app_on_ticket(true, true);
        app.screen = screen;
        let cmds = app.update(Msg::Key(Action::Char('A')));
        assert!(app.activity_hidden, "{screen:?}");
        assert!(cmds.is_empty(), "{screen:?} : {cmds:?}");
    }
}

// Snapshots: the whole frame, character for character, at the sizes people
// actually use. A `contains` says something is there; a snapshot says where,
// and fails on the layout drifting. Only fixtures with nothing time- or
// random-dependent on screen are snapshotted. Review a change with
// `cargo insta review`, or by reading the `.snap.new` next to the old one.

#[test]
fn snapshot_board_with_a_queue() {
    let app = board_waiting();
    insta::assert_snapshot!("board_80x24", draw(&app, 80, 24));
    insta::assert_snapshot!("board_120x40", draw(&app, 120, 40));
}

#[test]
fn snapshot_team_editor() {
    let mut app = app_on_ticket(true, false);
    app.update(Msg::Reply(Box::new(Reply::Roles {
        roles: vec![role("architect"), role("backend"), role("tests")],
        errors: vec![],
    })));
    app.update(Msg::Key(Action::Char('a')));
    insta::assert_snapshot!("team_editor_80x24", draw(&app, 80, 24));
    insta::assert_snapshot!("team_editor_120x40", draw(&app, 120, 40));
}

#[test]
fn snapshot_help_overlay() {
    let mut app = board_waiting();
    app.update(Msg::Key(Action::Help));
    insta::assert_snapshot!("help_120x40", draw(&app, 120, 40));
}

/// The single-character keys a bar or a help line promises.
fn promised_keys(app: &App) -> std::collections::HashSet<char> {
    let mut keys: std::collections::HashSet<char> = orchestra_tui::keys::screen_hints(app)
        .iter()
        .chain(orchestra_tui::keys::global_hints(app).iter())
        .flat_map(|h| {
            h.key
                .split(['/', ' '])
                .filter(|k| k.chars().count() == 1)
                .filter_map(|k| k.chars().next())
                .collect::<Vec<_>>()
        })
        .collect();
    // What every screen answers, written in the help's « Se déplacer » and
    // « Partout » tables rather than on the bar.
    keys.extend(['q', 'Q', '?', 'R', ':', 'j', 'k', 'h', 'l', 'g', 'G', '!', 'A']);
    keys.extend('1'..='8');
    keys
}

/// Everything a key could have changed, in one comparable value.
fn footprint(app: &App) -> String {
    format!(
        "{:?}|{}|{}|{}|{}|{}|{:?}|{}",
        app.screen,
        app.confirm.is_some(),
        app.steer.is_some(),
        app.editor.is_editing(),
        app.show_help,
        app.palette.is_some(),
        app.editor.members.iter().map(|m| (&m.role, &m.model, m.effort)).collect::<Vec<_>>(),
        app.status,
    )
}

#[test]
fn a_key_the_screen_does_not_announce_does_nothing() {
    // Every key a screen answers must be on its bar or in the help: a key
    // that acts without being written anywhere is one nobody finds, or one
    // somebody hits by accident.
    for name in ["tableau", "ticket", "ticket relu", "agent", "agent arrêté", "équipe", "coût"] {
        let promised = promised_keys(&fixture_named(name));
        for c in ('a'..='z').chain('A'..='Z') {
            if promised.contains(&c) {
                continue;
            }
            // Rebuilt for each key: a fixture is not Clone.
            let mut app = fixture_named(name);
            let before = footprint(&app);
            let cmds = app.update(Msg::Key(Action::Char(c)));
            assert!(
                cmds.is_empty() && footprint(&app) == before,
                "« {c} » agit sur l'écran {name} sans figurer dans sa barre ni dans l'aide : {cmds:?}"
            );
        }
    }
}

fn fixture_named(name: &str) -> App {
    match name {
        "tableau" => board_waiting(),
        "ticket" => app_on_ticket(true, true),
        "ticket relu" => app_on_reviewed_ticket(Verdict::Ready),
        "agent" => app_watching(AgentStatus::Running),
        "agent arrêté" => app_watching(AgentStatus::Done),
        "équipe" => {
            let mut app = app_on_ticket(true, false);
            app.update(Msg::Reply(Box::new(Reply::Roles {
                roles: vec![role("architect"), role("backend"), role("tests")],
                errors: vec![],
            })));
            app.update(Msg::Key(Action::Char('a')));
            app
        }
        "coût" => {
            let mut app = App::new();
            app.screen = Screen::Cost;
            app
        }
        _ => unreachable!(),
    }
}

#[test]
fn a_page_in_the_log_is_the_height_the_log_was_drawn_at() {
    let mut app = app_watching(AgentStatus::Running);
    for i in 0..200 {
        app.log.push_event(&Event::from_new(
            1,
            NewEvent::new(EventKind::AgentText { text: format!("ligne {i:03}") }),
        ));
    }
    draw(&app, 100, 40);
    let height = app.log_height.get();
    assert!(height > 5 && height < 40, "la hauteur vient du rendu : {height}");

    app.update(Msg::Key(Action::PageUp));
    assert!(!app.log.is_following());
    let last = app.log.window(height).last().unwrap().text.clone();
    // One line of the previous page stays in view, as in every pager.
    assert_eq!(last, format!("ligne {:03}", 199 - (height - 1)));

    app.update(Msg::Key(Action::PageDown));
    assert!(app.log.is_following(), "une page plus bas revient au direct");
}

#[test]
fn criteria_are_rewritten_one_per_line_and_reach_the_accepted_team() {
    let mut app = fixture_named("équipe");
    app.editor.members[0].acceptance = vec!["le plan existe".into()];

    app.update(Msg::Key(Action::Char('c')));
    assert!(app.is_typing(), "les lettres vont dans le texte");
    let out = draw(&app, 100, 30);
    assert!(out.contains("Critères, un par ligne"), "{out}");
    assert!(out.contains("[Entrée] critère suivant"));
    // Rewrite: drop the old line, type two.
    for _ in 0.."le plan existe".chars().count() {
        app.update(Msg::Key(Action::Backspace));
    }
    for c in "- docs/plan.md existe".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    app.update(Msg::Key(Action::Submit));
    assert!(app.editor.is_editing(), "Entrée passe au critère suivant");
    for c in "il nomme les fichiers".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    app.update(Msg::Key(Action::Accept));
    assert!(!app.editor.is_editing());
    assert_eq!(
        app.editor.members[0].acceptance,
        vec!["docs/plan.md existe".to_string(), "il nomme les fichiers".to_string()],
        "le tiret tapé par habitude est retiré"
    );

    // Escape leaves the list as it was.
    app.update(Msg::Key(Action::Char('c')));
    app.update(Msg::Key(Action::Char('x')));
    app.update(Msg::Key(Action::Cancel));
    assert_eq!(app.editor.members[0].acceptance.len(), 2);

    let cmds = app.update(Msg::Key(Action::Char('y')));
    let sent = cmds.iter().find_map(|c| match c {
        Command::AcceptProposal { team, .. } => Some(team.members[0].acceptance.clone()),
        _ => None,
    });
    assert_eq!(sent.unwrap().len(), 2, "les critères partent avec l'équipe");
}

#[test]
fn the_ticket_tells_its_story_on_demand() {
    let mut d = detail(false, true);
    d.recent_events = vec![
        Event::from_new(
            1,
            NewEvent::new(EventKind::TicketStatusChanged {
                from: TicketStatus::Planned,
                to: TicketStatus::Running,
            }),
        ),
        Event::from_new(
            2,
            NewEvent::new(EventKind::ReviewVerdict {
                round: 1,
                verdict: Verdict::Ready,
                blocking: vec![],
                roles: vec![],
            }),
        ),
    ];
    let mut app = App::new();
    app.connected = true;
    app.update(Msg::Reply(Box::new(Reply::Ticket { detail: Box::new(d) })));
    app.screen = Screen::Ticket;
    let has = |app: &App, label: &str| {
        orchestra_tui::keys::screen_hints(app)
            .iter()
            .any(|h| h.key == "T" && h.label == label)
    };
    assert!(has(&app, "chronologie"));

    app.update(Msg::Key(Action::Char('T')));
    let out = draw(&app, 120, 30);
    assert!(out.contains("Chronologie"), "{out}");
    let shown: Vec<String> = app
        .ticket
        .as_ref()
        .unwrap()
        .recent_events
        .iter()
        .filter_map(orchestra_tui::app::describe)
        .collect();
    assert_eq!(shown.len(), 2);
    for line in &shown {
        let words: String = line.chars().skip(10).take(20).collect();
        assert!(out.contains(words.trim()), "« {words} » absent : {out}");
    }
    assert!(has(&app, "le brief"));

    app.update(Msg::Key(Action::Char('T')));
    assert!(!draw(&app, 120, 30).contains("Chronologie"));
}

#[test]
fn the_diff_opens_over_the_ticket_scrolls_and_closes() {
    use orchestra_core::protocol::{DiffFile, TicketDiff};
    let mut app = app_on_ticket(false, true);
    // No branch yet: nothing to compare, nothing offered, nothing asked.
    assert!(!orchestra_tui::keys::screen_hints(&app).iter().any(|h| h.key == "D"));
    assert!(app.update(Msg::Key(Action::Char('D'))).is_empty());

    app.ticket.as_mut().unwrap().ticket.branch = Some("orch/12-cache".into());
    assert!(orchestra_tui::keys::screen_hints(&app).iter().any(|h| h.key == "D"));
    let cmds = app.update(Msg::Key(Action::Char('D')));
    assert!(matches!(cmds.as_slice(), [Command::GetDiff { .. }]), "{cmds:?}");

    let patch: String = (0..80).map(|i| format!("+ligne {i}\n")).collect();
    app.update(Msg::Reply(Box::new(Reply::Diff {
        diff: Box::new(TicketDiff {
            branch: "orch/12-cache".into(),
            base: "main".into(),
            files: vec![
                DiffFile { path: "src/cache.rs".into(), added: Some(80), removed: Some(0) },
                DiffFile { path: "logo.png".into(), added: None, removed: None },
            ],
            patch,
            truncated: false,
        }),
    })));
    let out = draw(&app, 100, 30);
    assert!(out.contains("Diff — main ← orch/12-cache · 2 fichier(s), +80 -0"), "{out}");
    assert!(out.contains("src/cache.rs") && out.contains("binaire"));
    assert!(out.contains("+ligne 0"));
    assert!(out.contains("[q] fermer le diff"));

    app.update(Msg::Key(Action::PageDown));
    let later = draw(&app, 100, 30);
    assert!(!later.contains("+ligne 0 "), "une page plus bas");

    app.update(Msg::Key(Action::Back));
    assert!(app.diff_view.is_none());
    assert_eq!(app.screen, Screen::Ticket, "q ferme le diff, pas l'écran");
}

#[test]
fn the_agent_log_is_searched_filtered_and_its_neighbours_reached() {
    let mut app = app_watching(AgentStatus::Running);
    for i in 0..40 {
        let kind = if i % 10 == 3 {
            EventKind::HookBlocked { tool: "Bash".into(), reason: format!("refus {i}") }
        } else {
            EventKind::AgentText { text: format!("réflexion {i}") }
        };
        app.log.push_event(&Event::from_new(i, NewEvent::new(kind)));
    }

    // `/` opens a search; letters go into it, not to the screen's keys.
    app.update(Msg::Key(Action::Char('/')));
    assert!(app.is_typing());
    for c in "réflexion 2".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    assert!(draw(&app, 100, 30).contains("/réflexion 2▏"));
    app.update(Msg::Key(Action::Submit));
    assert!(!app.is_typing());
    assert!(app.status.contains("n plus ancien"), "{}", app.status);
    let bottom = |app: &App| app.log.window(app.log_height.get()).last().unwrap().text.clone();
    draw(&app, 100, 30);
    assert_eq!(bottom(&app), "réflexion 29", "la plus récente d'abord");
    app.update(Msg::Key(Action::Char('n')));
    assert_eq!(bottom(&app), "réflexion 28");

    // `f` keeps only what went wrong.
    app.update(Msg::Key(Action::Char('f')));
    let out = draw(&app, 100, 30);
    assert!(out.contains("filtre : échecs et refus"), "{out}");
    let body: Vec<&str> = out.lines().filter(|l| l.starts_with('│')).collect();
    assert!(body.iter().all(|l| !l.contains("réflexion")), "la prose disparaît : {out}");
    assert_eq!(body.iter().filter(|l| l.contains("bloqué")).count(), 4);

    // `[` / `]` reach a neighbour in the team, and the screen stays on it.
    let team = app.ticket.as_ref().unwrap().agents.len();
    assert!(team > 1, "la fixture a une équipe");
    let before = app.watched_agent_id();
    let key = if app.agent_selected + 1 < team { ']' } else { '[' };
    let cmds = app.update(Msg::Key(Action::Char(key)));
    assert_ne!(app.watched_agent_id(), before);
    assert!(app.agent_hand_picked);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Subscribe { filter, .. } if filter.agent_id == app.watched_agent_id()
    )));
    assert!(app.log.is_empty(), "le journal du voisin repart de zéro");
}

#[test]
fn the_palette_opens_a_ticket_completes_and_remembers() {
    let mut app = board_waiting();
    app.update(Msg::Key(Action::CommandPalette));
    for c in "t 3".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    let cmds = app.update(Msg::Key(Action::Submit));
    assert_eq!(app.screen, Screen::Ticket);
    assert!(matches!(cmds.as_slice(), [Command::GetTicket { .. }]), "{cmds:?}");
    assert_eq!(app.selected_ticket().unwrap().ticket.number, 3);

    // Tab finishes a command's name when only one fits.
    app.update(Msg::Key(Action::CommandPalette));
    for c in "conv".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    app.update(Msg::Key(Action::NextField));
    assert_eq!(app.palette.as_deref(), Some("convention "));
    app.update(Msg::Key(Action::Cancel));

    // ↑ brings back what was run.
    app.update(Msg::Key(Action::CommandPalette));
    app.update(Msg::Key(Action::Up));
    assert_eq!(app.palette.as_deref(), Some("t 3"));
    app.update(Msg::Key(Action::Down));
    assert_eq!(app.palette.as_deref(), Some(""));
}
