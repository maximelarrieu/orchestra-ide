//! The ticket screen: brief, team, agents, cost, recent activity.

use orchestra_core::pricing::{fmt_tokens, fmt_usd};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::App;

use super::pane_block;
use crate::theme;

pub fn render(app: &App, frame: &mut Frame<'_>, area: Rect) {
    if app.ticket.is_none() {
        frame.render_widget(
            Paragraph::new("Aucun ticket ouvert. Reviens au tableau et appuie sur Entrée.")
                .style(Style::default().add_modifier(Modifier::DIM))
                .block(pane_block("Ticket", true))
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    // The agents pane takes only what it needs, so the team and its risks keep
    // the room: a fixed split pushed the orchestrator's warnings off screen.
    let agents = app.ticket.as_ref().map(|d| d.agents.len()).unwrap_or(0);
    let agents_height = (agents as u16 + 3).clamp(3, 9);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(6),
            Constraint::Length(agents_height),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(app, frame, chunks[0]);
    render_brief(app, frame, chunks[1]);
    render_agents(app, frame, chunks[2]);
    render_keys(app, frame, chunks[3]);
}

fn render_header(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let detail = app.ticket.as_ref().expect("ticket ouvert");
    let t = &detail.ticket;
    let cost = detail
        .cost_usd
        .map(fmt_usd)
        .unwrap_or_else(|| "≈$0".to_string());
    let badge = theme::ticket(t.status);
    let mut first = vec![
        Span::styled(format!("{} ", badge.symbol), badge.style()),
        Span::styled(
            format!("#{} ", t.number),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            t.title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ];
    if app.planning {
        let since = app
            .planning_since
            .map(|s| format!(" depuis {}", theme::elapsed(s, orchestra_core::now())))
            .unwrap_or_default();
        first.push(Span::styled(
            format!(
                "   {} l'orchestrateur réfléchit{since}",
                theme::spinner(app.ticks)
            ),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    // The verdict decides whether the branch can be integrated, so it belongs
    // where the branch is named. A symbol carries it, never a colour alone.
    let review = detail
        .review
        .as_ref()
        .map(|r| {
            format!(
                " · {} relecture {} : {}",
                if r.is_ready() { "✓" } else { "!" },
                r.round,
                r.verdict.label_fr()
            )
        })
        .unwrap_or_default();
    let second = format!(
        "{} · {} · {} tokens · {cost} (indicatif){}{review}",
        detail.project.name,
        t.status.label_fr(),
        fmt_tokens(detail.tokens.total()),
        t.branch
            .as_ref()
            .map(|b| format!(" · {b}"))
            .unwrap_or_default()
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(first),
            Line::from(Span::styled(
                second,
                Style::default().add_modifier(Modifier::DIM),
            )),
        ]),
        area,
    );
}

fn render_brief(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let detail = app.ticket.as_ref().expect("ticket ouvert");
    let t = &detail.ticket;

    let columns = Layout::default()
        .direction(if area.width >= 100 {
            Direction::Horizontal
        } else {
            Direction::Vertical
        })
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    frame.render_widget(
        Paragraph::new(t.brief.clone())
            .block(pane_block("Brief", false))
            .wrap(Wrap { trim: true }),
        columns[0],
    );

    // The accepted team if there is one, otherwise the pending proposal.
    let (title, lines) = match (&t.team, &t.proposal) {
        (Some(team), _) => {
            let mut lines = Vec::new();
            for (stage, member) in team.ordered() {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{stage}  "),
                        Style::default().add_modifier(Modifier::DIM),
                    ),
                    Span::styled(
                        member.role.clone(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]));
                lines.push(Line::from(Span::raw(format!("   {}", member.objective))));
            }
            ("Équipe acceptée".to_string(), lines)
        }
        (None, Some(proposal)) => {
            let mut lines = vec![
                Line::from(Span::raw(proposal.summary.clone())),
                Line::from(""),
            ];
            for member in &proposal.members {
                lines.push(Line::from(vec![
                    Span::styled(
                        member.role.clone(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        if member.depends_on.is_empty() {
                            String::new()
                        } else {
                            format!("  après {}", member.depends_on.join(", "))
                        },
                        Style::default().add_modifier(Modifier::DIM),
                    ),
                ]));
                lines.push(Line::from(Span::raw(format!("   {}", member.objective))));
            }
            if !proposal.risks.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "points d'attention",
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                for risk in &proposal.risks {
                    lines.push(Line::from(Span::raw(format!("- {risk}"))));
                }
            }
            ("Proposition — « a » pour la relire".to_string(), lines)
        }
        (None, None) if app.planning => (
            "L'orchestrateur compose l'équipe".to_string(),
            planning_lines(app),
        ),
        (None, None) => (
            "Équipe".to_string(),
            vec![Line::from(Span::styled(
                "pas encore d'équipe — « p » demande une proposition",
                Style::default().add_modifier(Modifier::DIM),
            ))],
        ),
    };

    frame.render_widget(
        Paragraph::new(lines)
            .block(pane_block(&title, false))
            .wrap(Wrap { trim: true }),
        columns[1],
    );
}

/// What the orchestrator is doing, while it does it.
///
/// A run reads the repository for half a minute before answering. Shown as a
/// still line, that is indistinguishable from a frozen screen — and the first
/// reflex is to press keys at it.
fn planning_lines(app: &App) -> Vec<Line<'static>> {
    let dim = Style::default().add_modifier(Modifier::DIM);
    let mut lines = vec![Line::from(Span::raw(
        "il lit le dépôt, ses conventions et ce qui existe déjà.",
    ))];

    // The counters are what say « vivant » when the trace is quiet.
    let mut counters = Vec::new();
    if let Some(since) = app.planning_since {
        counters.push(theme::elapsed(since, orchestra_core::now()));
    }
    if let Some(agent) = app.orchestrator_agent() {
        let tokens = agent.tokens.total();
        if tokens > 0 {
            counters.push(format!("{} tokens", fmt_tokens(tokens)));
        }
        if let Some(cost) = agent.cost_usd {
            counters.push(format!("{} (indicatif)", fmt_usd(cost)));
        }
    }
    if !counters.is_empty() {
        lines.push(Line::from(Span::styled(counters.join(" · "), dim)));
    }
    lines.push(Line::from(""));

    if app.planning_trace.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("{} démarrage du run…", theme::spinner(app.ticks)),
            dim,
        )));
        return lines;
    }
    let last = app.planning_trace.len() - 1;
    for (i, step) in app.planning_trace.iter().enumerate() {
        // The newest line is the one happening now; the rest is done.
        let marker = if i == last {
            theme::spinner(app.ticks)
        } else {
            "·"
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{marker} "), dim),
            Span::styled(step.clone(), if i == last { Style::default() } else { dim }),
        ]));
    }
    lines
}

fn render_agents(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let detail = app.ticket.as_ref().expect("ticket ouvert");
    if detail.agents.is_empty() {
        frame.render_widget(
            Paragraph::new("Aucun agent n'a encore tourné sur ce ticket.")
                .style(Style::default().add_modifier(Modifier::DIM))
                .block(pane_block("Agents", false)),
            area,
        );
        return;
    }

    let rows: Vec<Row> = detail
        .agents
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let style = if i == app.agent_selected {
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else if a.agent.status.is_active() {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let badge = theme::agent(a.agent.status);
            // A working agent shows how long it has been at it, which is the
            // difference between "thinking" and "stuck".
            let since = match (a.agent.status.is_active(), a.agent.started_at) {
                (true, Some(started)) => theme::elapsed(started, orchestra_core::now()),
                _ => a.turns.to_string(),
            };
            Row::new(vec![
                Cell::from(crate::app::agent_label(&detail.agents, i)),
                Cell::from(badge.label(a.agent.status.label_fr())).style(badge.style()),
                Cell::from(since),
                Cell::from(fmt_tokens(a.tokens.total())),
                Cell::from(a.cost_usd.map(fmt_usd).unwrap_or_else(|| "-".into())),
            ])
            .style(style)
        })
        .collect();

    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Min(18),
                Constraint::Length(14),
                Constraint::Length(11),
                Constraint::Length(9),
                Constraint::Length(10),
            ],
        )
        .header(
            Row::new(vec!["rôle", "statut", "depuis", "tokens", "coût"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .block(pane_block(
            "Agents — j/k choisir, Entrée suivre en direct",
            false,
        )),
        area,
    );
}

fn render_keys(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let detail = app.ticket.as_ref().expect("ticket ouvert");
    let mut keys: Vec<&str> = vec!["p planifier"];
    if detail.ticket.proposal.is_some() || detail.ticket.team.is_some() {
        keys.push("a relire l'équipe");
    }
    keys.push("L lancer");
    keys.push("x arrêter");
    if app.can_integrate() {
        keys.push("f intégrer et terminer");
    }
    if detail.ticket.status == orchestra_core::model::TicketStatus::Review {
        keys.push("t marquer terminé");
    }
    keys.push("n nouveau ticket");
    keys.push("q retour");
    keys.push("Q quitter");
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            keys.join("   "),
            Style::default().add_modifier(Modifier::DIM),
        ))),
        area,
    );
}
