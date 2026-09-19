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
