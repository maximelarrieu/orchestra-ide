use super::*;
use orchestra_core::events::NewEvent;
use orchestra_core::protocol::Command;
use uuid::Uuid;

fn app_with_projects(n: usize) -> App {
    let mut app = App::new();
    app.projects = (0..n)
        .map(|i| ProjectRow {
            id: Uuid::new_v4(),
            name: format!("p{i}"),
            path: format!("/tmp/p{i}"),
            discovered: false,
        })
        .collect();
    app
}

#[test]
fn screens_cycle_both_ways() {
    assert_eq!(Screen::Board.next(), Screen::Ticket);
    assert_eq!(Screen::Board.prev(), Screen::Epic);
    assert_eq!(Screen::from_number(1), Some(Screen::Board));
    assert_eq!(Screen::from_number(6), Some(Screen::Proposal));
    assert_eq!(Screen::from_number(7), Some(Screen::Todo));
    assert_eq!(Screen::from_number(8), Some(Screen::Rules));
    assert_eq!(Screen::from_number(9), Some(Screen::Epic));
    assert_eq!(Screen::from_number(0), None);
    assert_eq!(Screen::from_number(10), None);
}

#[test]
fn selection_stays_in_bounds() {
    let mut app = app_with_projects(3);
    app.update(Msg::Key(Action::Up));
    assert_eq!(app.project_selected, 0);
    for _ in 0..10 {
        app.update(Msg::Key(Action::Down));
    }
    // 0 is "Tous les projets", 1..=3 the three real ones.
    assert_eq!(app.project_selected, 3);
    app.update(Msg::Key(Action::Top));
    assert_eq!(app.project_selected, 0);
    app.update(Msg::Key(Action::Bottom));
    assert_eq!(app.project_selected, 3);
}

#[test]
fn the_first_project_list_lands_on_the_first_project_not_tous() {
    let mut app = App::new();
    assert!(app.selected_project().is_none(), "rien encore reçu");
    let projects = vec![
        orchestra_core::model::Project {
            id: Uuid::new_v4(),
            name: "seul".into(),
            path: "/tmp/seul".into(),
            default_branch: "main".into(),
            zellij_tab: None,
            kind: orchestra_core::model::ProjectKind::Managed,
            created_at: orchestra_core::now(),
        },
        orchestra_core::model::Project {
            id: Uuid::new_v4(),
            name: "autre".into(),
            path: "/tmp/autre".into(),
            default_branch: "main".into(),
            zellij_tab: None,
            kind: orchestra_core::model::ProjectKind::Managed,
            created_at: orchestra_core::now(),
        },
    ];
    app.update(Msg::Reply(Box::new(Reply::Projects {
        projects: projects.clone(),
    })));
    assert_eq!(
        app.selected_project().unwrap().id,
        projects[0].id,
        "un premier chargement ouvre directement sur un projet"
    );

    // A later refresh, with "Tous les projets" now chosen on purpose,
    // does not snap back to a project.
    app.project_selected = 0;
    app.update(Msg::Reply(Box::new(Reply::Projects { projects })));
    assert!(app.selected_project().is_none(), "« Tous » est respecté");
}

#[test]
fn tous_les_projets_asks_for_every_ticket() {
    let mut app = app_with_projects(2);
    assert_eq!(app.project_selected, 0);
    let cmds = app.update(Msg::Key(Action::Select));
    assert!(cmds.iter().any(
        |c| matches!(c, Command::ListTickets { project_id: None, .. })
    ));
}

#[test]
fn empty_lists_do_not_panic() {
    let mut app = App::new();
    for a in [
        Action::Down,
        Action::Up,
        Action::Top,
        Action::Bottom,
        Action::Select,
    ] {
        app.update(Msg::Key(a));
    }
    assert_eq!(app.project_selected, 0);
}

#[test]
fn moving_between_projects_asks_for_their_tickets() {
    let mut app = app_with_projects(2);
    let cmds = app.update(Msg::Key(Action::Down));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::ListTickets { .. })));
}

#[test]
fn palette_captures_keys_then_runs() {
    let mut app = App::new();
    app.update(Msg::Key(Action::CommandPalette));
    assert!(app.palette.is_some());
    // `j` must type, not move.
    for c in "project add /tmp/x".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    assert_eq!(app.palette.as_deref(), Some("project add /tmp/x"));
    let cmds = app.update(Msg::Key(Action::Submit));
    assert!(app.palette.is_none());
    assert!(matches!(
        cmds.first(),
        Some(Command::AddProject { path, .. }) if path.as_os_str() == "/tmp/x"
    ));
}

#[test]
fn palette_reports_unknown_commands() {
    let mut app = App::new();
    app.update(Msg::Key(Action::CommandPalette));
    for c in "danse".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    app.update(Msg::Key(Action::Submit));
    assert!(app.status.contains("inconnue"), "{}", app.status);
}

#[test]
fn d_on_tous_les_projets_forgets_nothing() {
    let mut app = app_with_projects(2);
    assert_eq!(app.project_selected, 0, "démarre sur « Tous les projets »");
    app.update(Msg::Key(Action::Char('d')));
    assert!(app.confirm.is_none());
    assert!(app.status.contains("aucun projet"), "{}", app.status);
}

#[test]
fn d_asks_before_forgetting_the_selected_project() {
    let mut app = app_with_projects(2);
    app.project_selected = 1;
    let id = app.projects[0].id;
    let cmds = app.update(Msg::Key(Action::Char('d')));
    // Nothing is sent yet — a question is, and it names the project.
    assert!(cmds.is_empty());
    let confirm = app.confirm.as_ref().expect("a question was asked");
    assert!(confirm.question.contains("p0"), "{}", confirm.question);
    assert_eq!(confirm.command, Command::ForgetProject { project_id: id });

    let cmds = app.update(Msg::Key(Action::Char('y')));
    assert!(matches!(
        cmds.as_slice(),
        [Command::ForgetProject { project_id }] if *project_id == id
    ));
    assert!(app.confirm.is_none());
}

#[test]
fn d_on_the_tickets_pane_does_not_touch_projects() {
    let mut app = app_with_projects(1);
    app.board_pane = BoardPane::Tickets;
    app.update(Msg::Key(Action::Char('d')));
    assert!(app.confirm.is_none());
}

#[test]
fn palette_project_forget_resolves_a_project_by_name() {
    let mut app = app_with_projects(2);
    let id = app.projects[1].id;
    app.update(Msg::Key(Action::CommandPalette));
    for c in "project forget p1".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    app.update(Msg::Key(Action::Submit));
    let confirm = app.confirm.as_ref().expect("a question was asked");
    assert_eq!(confirm.command, Command::ForgetProject { project_id: id });
}

#[test]
fn palette_project_forget_reports_no_match() {
    let mut app = app_with_projects(1);
    app.update(Msg::Key(Action::CommandPalette));
    for c in "project forget nope".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    app.update(Msg::Key(Action::Submit));
    assert!(app.confirm.is_none());
    assert!(app.status.contains("nope"), "{}", app.status);
}

#[test]
fn a_forgotten_project_refreshes_the_list() {
    let mut app = App::new();
    let e = Event::from_new(
        1,
        NewEvent::new(EventKind::ProjectForgotten { name: "p0".into() }),
    );
    let cmds = app.update(Msg::Event(Box::new(e)));
    assert!(cmds.iter().any(|c| matches!(c, Command::ListProjects)));
}

fn app_with_todos(n: usize) -> App {
    let mut app = App::new();
    let now = orchestra_core::now();
    app.screen = Screen::Todo;
    app.todos = (0..n)
        .map(|i| Todo {
            id: Uuid::new_v4(),
            title: format!("t{i}"),
            notes: String::new(),
            status: TodoStatus::Open,
            urgent: false,
            due_at: None,
            promoted_ticket_id: None,
            created_at: now,
            updated_at: now,
        })
        .collect();
    app
}

#[test]
fn d_asks_before_deleting_the_selected_todo() {
    let mut app = app_with_todos(2);
    let id = app.todos[0].id;
    let cmds = app.update(Msg::Key(Action::Char('d')));
    assert!(cmds.is_empty());
    let confirm = app.confirm.as_ref().expect("a question was asked");
    assert!(confirm.question.contains("t0"), "{}", confirm.question);
    assert_eq!(confirm.command, Command::DeleteTodo { todo_id: id });

    let cmds = app.update(Msg::Key(Action::Char('y')));
    assert!(matches!(
        cmds.as_slice(),
        [Command::DeleteTodo { todo_id }] if *todo_id == id
    ));
}

fn app_with_rules() -> App {
    use orchestra_core::conventions::RuleMode;
    let mut app = App::new();
    app.screen = Screen::Rules;
    app.rules = ["commits", "paginer"]
        .iter()
        .map(|n| Rule {
            name: n.to_string(),
            kind: RuleKind::Convention,
            title: format!("titre {n}"),
            status: if *n == "paginer" {
                RuleStatus::Proposed
            } else {
                RuleStatus::Accepted
            },
            applies_to: vec![],
            mode: RuleMode::Any,
            checks: vec![],
            supersedes: None,
            proposed_by: None,
            body: "corps".into(),
            source: format!("/nulle-part/{n}.md").into(),
            scope: orchestra_core::model::RoleScope::Global,
        })
        .collect();
    app
}

#[test]
fn y_accepts_the_selected_rule_and_x_asks_first() {
    let mut app = app_with_rules();
    assert_eq!(app.pending_rules(), 1);
    app.update(Msg::Key(Action::Down));
    let cmds = app.update(Msg::Key(Action::Char('y')));
    assert!(matches!(
        cmds.as_slice(),
        [Command::SetRuleStatus { name, status: RuleStatus::Accepted, .. }] if name == "paginer"
    ));

    let cmds = app.update(Msg::Key(Action::Char('x')));
    assert!(cmds.is_empty(), "rejeter se confirme");
    assert!(app.confirm.as_ref().unwrap().question.contains("titre paginer"));
    let cmds = app.update(Msg::Key(Action::Char('y')));
    assert!(matches!(
        cmds.as_slice(),
        [Command::SetRuleStatus { status: RuleStatus::Rejected, .. }]
    ));
}

#[test]
fn a_rule_that_is_not_on_disk_is_not_sent_to_an_editor() {
    let mut app = app_with_rules();
    app.update(Msg::Key(Action::Char('e')));
    assert!(app.take_edit_request().is_none());
    assert!(app.status.contains("orchestra init"), "{}", app.status);
}

#[test]
fn a_new_rule_opens_in_the_editor_once_written() {
    let mut app = App::new();
    app.update(Msg::Key(Action::CommandPalette));
    for c in "convention add Toujours paginer".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    let cmds = app.update(Msg::Key(Action::Submit));
    assert!(matches!(
        cmds.first(),
        Some(Command::CreateRule { rule_kind: RuleKind::Convention, title, project_id: None })
            if title == "Toujours paginer"
    ));
    let cmds = app.update(Msg::Reply(Box::new(Reply::RuleFile {
        path: "/x/toujours-paginer.md".into(),
    })));
    assert!(cmds.iter().any(|c| matches!(c, Command::ListRules { .. })));
    assert_eq!(
        app.take_edit_request().as_deref(),
        Some(std::path::Path::new("/x/toujours-paginer.md"))
    );
    // A file written for another reason does not open anything.
    app.update(Msg::Reply(Box::new(Reply::RuleFile { path: "/x/y.md".into() })));
    assert!(app.take_edit_request().is_none());
}

#[test]
fn an_adr_needs_a_project() {
    let mut app = App::new();
    app.update(Msg::Key(Action::CommandPalette));
    for c in "adr add SQLite".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    let cmds = app.update(Msg::Key(Action::Submit));
    assert!(!cmds.iter().any(|c| matches!(c, Command::CreateRule { .. })));
    assert!(app.status.contains("projet"), "{}", app.status);
}

#[test]
fn a_reread_keeps_the_selected_rule() {
    let mut app = app_with_rules();
    app.update(Msg::Key(Action::Down));
    let mut rules = app.rules.clone();
    rules.reverse();
    app.update(Msg::Reply(Box::new(Reply::Rules { rules, errors: vec![] })));
    assert_eq!(app.selected_rule().unwrap().name, "paginer");
}

#[test]
fn palette_todo_add_creates_a_todo() {
    let mut app = App::new();
    app.update(Msg::Key(Action::CommandPalette));
    for c in "todo add acheter du café".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    let cmds = app.update(Msg::Key(Action::Submit));
    assert!(matches!(
        cmds.first(),
        Some(Command::CreateTodo { title, .. }) if title == "acheter du café"
    ));
}

#[test]
fn palette_todo_status_resolves_by_position() {
    let mut app = app_with_todos(2);
    let id = app.todos[1].id;
    app.update(Msg::Key(Action::CommandPalette));
    for c in "todo done 2".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    let cmds = app.update(Msg::Key(Action::Submit));
    assert!(matches!(
        cmds.first(),
        Some(Command::SetTodoStatus { todo_id, status })
            if *todo_id == id && *status == TodoStatus::Done
    ));
}

#[test]
fn promoting_a_todo_seeds_the_form_and_sends_promote_on_submit() {
    let mut app = app_with_todos(1);
    app.projects = vec![ProjectRow {
        id: Uuid::new_v4(),
        name: "p0".into(),
        path: "/tmp/p0".into(),
        discovered: false,
    }];
    app.project_selected = 1;
    let todo_id = app.todos[0].id;
    let project_id = app.projects[0].id;

    app.update(Msg::Key(Action::Char('p')));
    assert_eq!(app.screen, Screen::NewTicket);
    assert_eq!(app.form.title, "t0");
    assert_eq!(app.promoting_todo, Some(todo_id));

    let cmds = app.update(Msg::Key(Action::NextField));
    assert!(cmds.is_empty());
    for c in "un brief assez long pour passer la validation".chars() {
        app.update(Msg::Key(Action::Char(c)));
    }
    let cmds = app.update(Msg::Key(Action::Accept));
    assert!(matches!(
        cmds.as_slice(),
        [Command::PromoteTodo { todo_id: t, project_id: p, .. }]
            if *t == todo_id && *p == project_id
    ));
    assert!(app.promoting_todo.is_none());
}

#[test]
fn a_promoted_todo_refreshes_todos_and_tickets() {
    let mut app = App::new();
    let e = Event::from_new(
        1,
        NewEvent::new(EventKind::TodoPromoted {
            title: "t0".into(),
            ticket_number: 1,
            project_name: "p0".into(),
        }),
    );
    let cmds = app.update(Msg::Event(Box::new(e)));
    assert!(cmds.iter().any(|c| matches!(c, Command::ListTodos)));
    assert!(cmds.iter().any(|c| matches!(c, Command::ListTickets { .. })));
}

#[test]
fn todo_digest_relays_to_the_shared_core_rule() {
    // The counting rule itself is tested exhaustively in
    // orchestra-core (shared with the CLI's own notification); this
    // only checks the relay reads `self.todos`.
    let mut app = App::new();
    assert!(app.todo_digest().is_empty());
    app.todos = vec![Todo {
        id: Uuid::new_v4(),
        title: "x".into(),
        notes: String::new(),
        status: TodoStatus::Open,
        urgent: true,
        due_at: None,
        promoted_ticket_id: None,
        created_at: orchestra_core::now(),
        updated_at: orchestra_core::now(),
    }];
    assert_eq!(app.todo_digest().urgent, 1);
}

#[test]
fn help_opens_and_any_key_closes_it() {
    let mut app = App::new();
    app.update(Msg::Key(Action::Help));
    assert!(app.show_help);
    app.update(Msg::Key(Action::Down));
    assert!(!app.show_help);
    assert!(!app.should_quit);
}

#[test]
fn quit_works_from_the_help_overlay() {
    let mut app = App::new();
    app.update(Msg::Key(Action::Help));
    app.update(Msg::Key(Action::Quit));
    assert!(app.should_quit);
}

#[test]
fn disconnection_is_visible() {
    let mut app = App::new();
    app.update(Msg::Reconnected("0.1.0".into()));
    assert!(app.connected);
    app.update(Msg::Disconnected);
    assert!(!app.connected);
    assert!(app.status.contains("déconnecté"));
}

#[test]
fn activity_is_bounded() {
    let mut app = App::new();
    for i in 0..(ACTIVITY_MAX + 50) {
        let e = Event::from_new(
            i as i64,
            NewEvent::new(EventKind::Warning {
                message: format!("w{i}"),
            }),
        );
        app.update(Msg::Event(Box::new(e)));
    }
    assert_eq!(app.activity.len(), ACTIVITY_MAX);
    assert!(app
        .activity
        .last()
        .unwrap()
        .contains(&format!("w{}", ACTIVITY_MAX + 49)));
}

#[test]
fn chatty_events_stay_out_of_the_activity_strip() {
    let e = Event::from_new(1, NewEvent::new(EventKind::AgentText { text: "x".into() }));
    assert_eq!(describe(&e), None);
    let e = Event::from_new(
        2,
        NewEvent::new(EventKind::TicketCreated {
            number: 3,
            title: "T".into(),
        }),
    );
    assert!(describe(&e).unwrap().contains("#3"));
}

#[test]
fn selection_survives_a_refresh_that_reorders_projects() {
    let mut app = app_with_projects(3);
    app.project_selected = 3;
    let keep = app.projects[2].clone();
    let projects = vec![
        orchestra_core::model::Project {
            id: keep.id,
            name: keep.name.clone(),
            path: keep.path.clone().into(),
            default_branch: "main".into(),
            zellij_tab: None,
            kind: orchestra_core::model::ProjectKind::Managed,
            created_at: orchestra_core::now(),
        },
        orchestra_core::model::Project {
            id: Uuid::new_v4(),
            name: "autre".into(),
            path: "/tmp/autre".into(),
            default_branch: "main".into(),
            zellij_tab: None,
            kind: orchestra_core::model::ProjectKind::Managed,
            created_at: orchestra_core::now(),
        },
    ];
    app.update(Msg::Reply(Box::new(Reply::Projects { projects })));
    assert_eq!(app.selected_project().unwrap().id, keep.id);
}

#[test]
fn every_screen_exists_now() {
    // Nothing announces a future phase any more.
    for n in 1..=6 {
        let mut app = App::new();
        app.update(Msg::Key(Action::Screen(n)));
        assert!(app.screen.is_implemented(), "écran {n}");
        assert!(!app.status.contains("phase"), "écran {n} : {}", app.status);
    }
}

#[test]
fn the_cost_screen_reacts_to_its_own_keys() {
    let mut app = App::new();
    app.update(Msg::Key(Action::Screen(4)));

    let cmds = app.update(Msg::Key(Action::Char('m')));
    assert_eq!(app.cost.group, GroupBy::Ticket, "m change le regroupement");
    assert_eq!(cmds.len(), 2, "le tableau et la courbe sont redemandés");

    app.update(Msg::Key(Action::Char('p')));
    assert_eq!(app.cost.period, Period::Month, "p change la période");

    assert!(app.cost.include_unmanaged);
    app.update(Msg::Key(Action::Char('u')));
    assert!(!app.cost.include_unmanaged, "u bascule les sessions libres");

    // Those keys do nothing on the board.
    app.update(Msg::Key(Action::Screen(1)));
    let before = app.cost.group;
    app.update(Msg::Key(Action::Char('m')));
    assert_eq!(app.cost.group, before);
}

#[test]
fn the_period_defines_the_window_it_asks_for() {
    let mut app = App::new();
    app.cost.period = Period::All;
    let [table, trend] = app.cost.queries();
    assert!(table.range.since.is_none(), "« tout » ne borne pas");
    assert_eq!(trend.group_by, vec![GroupBy::Day]);

    app.cost.period = Period::Today;
    let [table, _] = app.cost.queries();
    let since = table.range.since.expect("aujourd'hui commence à minuit");
    assert_eq!(since.time(), time::Time::MIDNIGHT);
}

#[test]
fn the_trend_reply_is_told_apart_from_the_table() {
    use std::collections::BTreeMap;
    let mut app = App::new();
    let mut daily_keys = BTreeMap::new();
    daily_keys.insert("day".to_string(), "2026-09-19".to_string());
    let daily = UsageRow {
        keys: daily_keys,
        tokens: Default::default(),
        cost_usd: Some(3.0),
        messages: 1,
        cost_estimated: false,
    };
    let mut table_keys = BTreeMap::new();
    table_keys.insert("project".to_string(), "orchestra".to_string());
    let table = UsageRow {
        keys: table_keys,
        tokens: Default::default(),
        cost_usd: Some(9.0),
        messages: 2,
        cost_estimated: false,
    };

    app.update(Msg::Reply(Box::new(Reply::Usage {
        rows: vec![daily],
        totals: UsageTotals::default(),
    })));
    app.update(Msg::Reply(Box::new(Reply::Usage {
        rows: vec![table],
        totals: UsageTotals {
            cost_usd: Some(9.0),
            ..Default::default()
        },
    })));

    assert_eq!(app.cost.daily.len(), 1, "la courbe est rangée à part");
    assert_eq!(app.cost.rows.len(), 1, "le tableau n'est pas écrasé");
    assert_eq!(app.cost.totals.cost_usd, Some(9.0));
}

#[test]
fn every_screen_refreshes_itself_on_the_tick() {
    // The loop used to only repaint on the tick, without handing it to the
    // app: nothing was polled, so a status changed elsewhere was only seen
    // by leaving the screen and coming back.
    let mut app = App::new();
    let cmds = app.update(Msg::Tick);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::ListTickets { .. })),
        "le tableau interroge sa liste à chaque seconde : {cmds:?}"
    );

    // And the counters that turn on their own advance with it.
    let before = app.ticks;
    app.update(Msg::Tick);
    assert_eq!(app.ticks, before + 1);
}

#[test]
fn the_cost_view_is_polled_rather_than_event_driven() {
    let mut app = App::new();
    app.update(Msg::Key(Action::Screen(4)));
    // One tick is not enough; the second asks.
    assert!(app.update(Msg::Tick).is_empty());
    let cmds = app.update(Msg::Tick);
    assert_eq!(cmds.len(), 2);

    // A usage event from a managed agent does not trigger its own query.
    let e = Event::from_new(
        1,
        NewEvent::new(EventKind::Usage {
            sample: Box::new(orchestra_core::model::UsageSample {
                message_id: "m".into(),
                session_id: Uuid::new_v4(),
                subagent_id: None,
                agent_id: None,
                ticket_id: None,
                project_id: None,
                model: "claude-opus-5".into(),
                tokens: Default::default(),
                ts: orchestra_core::now(),
                source: orchestra_core::model::UsageSource::Stream,
            }),
        }),
    );
    assert!(app.update(Msg::Event(Box::new(e))).is_empty());
}
