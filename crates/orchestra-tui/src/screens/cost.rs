//! The cost screen: where the tokens went.
//!
//! The dollar figures are indicative. On a Claude subscription nothing is
//! billed per request, so the screen leads with tokens and labels every amount
//! as an estimate.

use orchestra_core::pricing::{fmt_tokens, fmt_usd};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Paragraph, Row, Sparkline, Table, Wrap};
use ratatui::Frame;

use crate::app::{App, CostView, Period};

use super::pane_block;

/// Below this width the table drops its least useful columns.
const WIDE: u16 = 92;

pub fn render(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let has_trend = app.cost.daily.len() > 1 && area.height >= 12;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if has_trend {
            [Constraint::Min(4), Constraint::Length(4)]
        } else {
            [Constraint::Min(4), Constraint::Length(0)]
        })
        .split(area);

    // The screen's three settings (« m », « p », « u ») are announced by the
    // key bar, each with the value it stands at: a control line here would
    // have repeated them, and the table's title already spells them out.
    render_table(app, frame, chunks[0]);
    if has_trend {
        render_trend(app, frame, chunks[1]);
    }
}

fn render_table(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let c = &app.cost;
    let title = format!(
        "Coût par {} — {} — {} (indicatif)",
        c.group.label_fr(),
        c.period.label_fr(),
        c.totals
            .cost_usd
            .map(fmt_usd)
            .unwrap_or_else(|| "coût inconnu".into())
    );

    if c.rows.is_empty() {
        let hint = if c.include_unmanaged {
            "Aucune consommation sur cette période.\nLance une session Claude Code : elle sera comptée ici automatiquement."
        } else {
            "Aucun agent Orchestra n'a encore tourné.\n`u` réaffiche tes propres sessions Claude Code."
        };
        frame.render_widget(
            Paragraph::new(hint)
                .style(Style::default().add_modifier(Modifier::DIM))
                .block(pane_block(&title, true))
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    let wide = area.width >= WIDE;
    let rows: Vec<Row> = c
        .rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let style = if i == c.selected {
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else {
                Style::default()
            };
            let name = r
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
            let mut cells = vec![
                name,
                fmt_tokens(r.tokens.input),
                fmt_tokens(r.tokens.output),
                fmt_tokens(r.tokens.cache_read),
            ];
            if wide {
                cells.push(fmt_tokens(r.tokens.cache_creation));
                cells.push(fmt_tokens(r.tokens.thinking));
                cells.push(r.messages.to_string());
            }
            cells.push(cost);
            Row::new(cells).style(style)
        })
        .collect();

    let mut header = vec!["", "entrée", "sortie", "cache lu"];
    let mut widths = vec![
        Constraint::Min(14),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(9),
    ];
    if wide {
        header.extend(["cache écrit", "réflexion", "msg"]);
        widths.extend([
            Constraint::Length(11),
            Constraint::Length(9),
            Constraint::Length(6),
        ]);
    }
    header.push("coût");
    widths.push(Constraint::Length(10));

    let table = Table::new(rows, widths)
        .header(Row::new(header).style(Style::default().add_modifier(Modifier::BOLD)))
        .block(pane_block(&title, true));
    frame.render_widget(table, area);
}

fn render_trend(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let daily = &app.cost.daily;
    // Sparkline takes integers, so costs are shown in cents.
    let data: Vec<u64> = daily
        .iter()
        .map(|(_, cost)| (cost * 100.0).round().max(0.0) as u64)
        .collect();
    let peak = daily.iter().map(|(_, c)| *c).fold(0.0_f64, f64::max);
    let span = match (daily.first(), daily.last()) {
        (Some((a, _)), Some((b, _))) if a != b => format!("{a} → {b}"),
        (Some((a, _)), _) => a.clone(),
        _ => String::new(),
    };
    let title = format!("Par jour — {span} — pic {}", fmt_usd(peak));
    frame.render_widget(
        Sparkline::default()
            .data(&data)
            .block(pane_block(&title, false)),
        area,
    );
}

/// Footer note shown on the status bar while the cost screen is open.
pub fn estimate_note(cost: &CostView) -> &'static str {
    if cost.rows.iter().any(|r| r.cost_estimated) {
        "* tarif approché"
    } else {
        ""
    }
}

/// Label of the period, for the status bar.
pub fn period_label(p: Period) -> &'static str {
    p.label_fr()
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::model::Tokens;
    use orchestra_core::protocol::{UsageRow, UsageTotals};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::collections::BTreeMap;

    fn row(name: &str, out: u64, cost: Option<f64>, estimated: bool) -> UsageRow {
        let mut keys = BTreeMap::new();
        keys.insert("project".to_string(), name.to_string());
        UsageRow {
            keys,
            tokens: Tokens {
                input: 100,
                output: out,
                cache_read: 50_000,
                cache_creation: 1_000,
                thinking: out / 2,
            },
            cost_usd: cost,
            messages: 12,
            cost_estimated: estimated,
        }
    }

    fn app_with_rows(rows: Vec<UsageRow>) -> App {
        let mut app = App::new();
        app.screen = crate::app::Screen::Cost;
        app.cost.totals = UsageTotals {
            tokens: Tokens {
                input: 100,
                output: 1000,
                cache_read: 50_000,
                cache_creation: 1_000,
                thinking: 500,
            },
            cost_usd: Some(12.34),
            messages: 42,
            reported_cost_usd: None,
        };
        app.cost.rows = rows;
        app
    }

    fn draw(app: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| crate::screens::render(app, f)).unwrap();
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

    #[test]
    fn shows_rows_and_labels_the_amount_as_indicative() {
        let app = app_with_rows(vec![
            row("orchestra-ide", 287_000, Some(26.24), false),
            row("tools-library", 195_800, Some(15.68), false),
        ]);
        let out = draw(&app, 110, 24);
        assert!(out.contains("orchestra-ide"));
        assert!(out.contains("tools-library"));
        assert!(
            out.contains("indicatif"),
            "le coût doit être annoncé comme estimé"
        );
        assert!(out.contains("sortie"));
    }

    #[test]
    fn an_approximated_rate_is_flagged() {
        let app = app_with_rows(vec![row("modèle-inconnu", 1000, Some(0.5), true)]);
        let out = draw(&app, 110, 24);
        assert!(out.contains('*'), "un tarif approché doit être signalé");
        assert_eq!(estimate_note(&app.cost), "* tarif approché");
    }

    #[test]
    fn the_controls_show_the_current_view() {
        let mut app = app_with_rows(vec![row("p", 10, Some(1.0), false)]);
        app.cost.period = Period::Today;
        app.cost.include_unmanaged = false;
        let out = draw(&app, 110, 24);
        assert!(out.contains("aujourd'hui"));
        assert!(out.contains("agents seulement"));
    }

    #[test]
    fn an_empty_period_explains_itself() {
        let mut app = app_with_rows(vec![]);
        app.cost.totals = UsageTotals::default();
        let out = draw(&app, 100, 24);
        assert!(out.contains("Aucune consommation"));
    }

    #[test]
    fn a_narrow_pane_drops_columns_instead_of_overflowing() {
        let app = app_with_rows(vec![row("orchestra-ide", 287_000, Some(26.24), false)]);
        let narrow = draw(&app, 64, 24);
        assert!(narrow.contains("orchestra-ide"));
        assert!(narrow.contains("sortie"));
        // The least useful columns are gone at this width.
        assert!(!narrow.contains("réflexion"));
        let wide = draw(&app, 120, 24);
        assert!(wide.contains("réflexion"));
    }

    #[test]
    fn the_trend_appears_only_with_enough_days_and_room() {
        let mut app = app_with_rows(vec![row("p", 10, Some(1.0), false)]);
        app.cost.daily = vec![
            ("2026-09-17".into(), 1.0),
            ("2026-09-18".into(), 4.0),
            ("2026-09-19".into(), 2.0),
        ];
        let out = draw(&app, 110, 24);
        assert!(out.contains("Par jour"));
        assert!(out.contains("pic"));
        // A single day is not a trend.
        app.cost.daily = vec![("2026-09-19".into(), 2.0)];
        assert!(!draw(&app, 110, 24).contains("Par jour"));
    }

    #[test]
    fn tiny_terminals_do_not_panic() {
        let app = app_with_rows(vec![row("p", 10, Some(1.0), false)]);
        for (w, h) in [(20u16, 6u16), (10, 4), (40, 10), (200, 60)] {
            let _ = draw(&app, w, h);
        }
    }
}
