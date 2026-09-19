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
use orchestra_core::protocol::{AgentSummary, Command, Reply, TicketDetail, TicketSummary};
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
    assert!(out.contains("a relire"), "la touche est proposée");
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
    let app = app_on_ticket(false, true);
    assert!(draw(&app, 120, 30).contains("Q quitter"));
    let mut board = App::new();
    board.connected = true;
    assert!(draw(&board, 120, 30).contains("Q quitter"));
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
