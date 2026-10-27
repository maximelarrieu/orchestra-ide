//! The observer: what was spent over time, which monitors stand, and where a
//! ticket's time went.
//!
//! Nothing here rests on colour alone (rule 11): a step that failed carries
//! `✗`, one that passed `✓`, one still running `…`; an alert carries `⚠`.

use orchestra_core::observe::{Span, Trace};
use orchestra_core::pricing::{fmt_tokens, fmt_usd};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Paragraph, Sparkline, Wrap};
use ratatui::Frame;
use time::OffsetDateTime;

use crate::app::{App, ObserveTab};

use super::pane_block;

pub fn render(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3)])
        .split(area);
    frame.render_widget(Paragraph::new(tabs_line(app)), chunks[0]);
    match app.observe.tab {
        ObserveTab::Metrics => render_metrics(app, frame, chunks[1]),
        ObserveTab::Alerts => render_alerts(app, frame, chunks[1]),
        ObserveTab::Trace => render_trace(app, frame, chunks[1]),
    }
}

/// `[métriques]  alertes (1)  trace` — the open tab in brackets, so it is
/// told apart without colour.
fn tabs_line(app: &App) -> String {
    ObserveTab::ALL
        .iter()
        .map(|t| {
            let mut label = t.label_fr().to_string();
            if *t == ObserveTab::Alerts && !app.observe.active.is_empty() {
                label = format!("{label} ⚠ {}", app.observe.active.len());
            }
            if *t == app.observe.tab {
                format!("[{label}]")
            } else {
                format!(" {label} ")
            }
        })
        .collect::<Vec<_>>()
        .join("  ")
}

fn dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

// -- metrics ----------------------------------------------------------------

fn render_metrics(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let Some(series) = app.observe.series.as_ref() else {
        frame.render_widget(
            Paragraph::new("Lecture en cours…")
                .style(dim())
                .block(pane_block("Métriques", true)),
            area,
        );
        return;
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 3); 3])
        .split(area);
    let window = app.observe.window_label();
    let step = series.step_secs / 60;

    let tokens = &series.tokens;
    let peak = tokens.iter().copied().max().unwrap_or(0);
    let total: u64 = tokens.iter().sum();
    spark(
        frame,
        rows[0],
        &format!(
            "Tokens — {window}, par {step} min — total {} — pic {}",
            fmt_tokens(total),
            fmt_tokens(peak)
        ),
        tokens,
    );

    // Sparkline takes integers: dollars go in cents, as on the cost screen.
    let cents: Vec<u64> = series
        .cost_usd
        .iter()
        .map(|c| (c * 100.0).round().max(0.0) as u64)
        .collect();
    let cost_total: f64 = series.cost_usd.iter().sum();
    let cost_peak = series.cost_usd.iter().copied().fold(0.0_f64, f64::max);
    spark(
        frame,
        rows[1],
        &format!(
            "Coût (indicatif) — total {} — pic {}",
            fmt_usd(cost_total),
            fmt_usd(cost_peak)
        ),
        &cents,
    );

    let messages = &series.messages;
    let msg_total: u64 = messages.iter().sum();
    spark(
        frame,
        rows[2],
        &format!(
            "Messages — total {msg_total} — pic {}",
            messages.iter().copied().max().unwrap_or(0)
        ),
        messages,
    );
}

fn spark(frame: &mut Frame<'_>, area: Rect, title: &str, data: &[u64]) {
    frame.render_widget(
        Sparkline::default()
            .data(data)
            .block(pane_block(title, false)),
        area,
    );
}

// -- alerts -----------------------------------------------------------------

fn render_alerts(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let o = &app.observe;
    let mut lines: Vec<String> = Vec::new();
    if o.monitors == 0 {
        lines.push("Aucune alerte définie.".into());
        lines.push(
            "Ajoute des `[[monitors]]` à config.toml (modèle dans config.example.toml), \
             puis relance le daemon."
                .into(),
        );
    } else if o.active.is_empty() {
        lines.push(format!("✓ Rien au-dessus de son seuil ({} alerte(s) surveillée(s)).", o.monitors));
    }
    for f in &o.active {
        lines.push(format!("⚠ {} — {}", f.monitor, f.line_fr()));
    }
    if !o.history.is_empty() {
        lines.push(String::new());
        lines.push("Derniers franchissements".into());
        for h in &o.history {
            let t = h.ts.time();
            let stamp = format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                h.ts.year(),
                u8::from(h.ts.month()),
                h.ts.day(),
                t.hour(),
                t.minute()
            );
            lines.push(match &h.firing {
                Some(f) => format!("▲ {stamp}  {} — {}", h.monitor, f.line_fr()),
                None => format!("▽ {stamp}  {} — revenue sous son seuil", h.monitor),
            });
        }
    }
    let title = format!("Alertes — {} en cours", o.active.len());
    frame.render_widget(
        Paragraph::new(lines.join("\n"))
            .block(pane_block(&title, true))
            .wrap(Wrap { trim: false }),
        area,
    );
}

// -- trace ------------------------------------------------------------------

/// Width of the label column of the waterfall.
const LABEL: usize = 30;

fn render_trace(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let o = &app.observe;
    let ticket = o
        .trace_for
        .and_then(|id| app.tickets.iter().find(|t| t.ticket.id == id))
        .map(|t| format!("#{} {}", t.ticket.number, t.ticket.title));
    let title = format!("Trace — {}", ticket.as_deref().unwrap_or("aucun ticket"));
    let hint = match (&o.trace_for, &o.trace) {
        (None, _) => Some("Aucun ticket choisi : ouvre-en un sur le tableau, puis `O`."),
        (Some(_), None) => Some("Lecture en cours…"),
        (Some(_), Some(t)) if t.is_empty() => Some("Ce ticket n'a encore aucune étape."),
        _ => None,
    };
    let block = pane_block(&title, true);
    if let Some(hint) = hint {
        frame.render_widget(Paragraph::new(hint).style(dim()).block(block), area);
        return;
    }
    let Some(trace) = o.trace.as_ref() else { return };
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = (inner.width as usize).saturating_sub(LABEL + 12).max(8);
    let text = waterfall(trace, orchestra_core::now(), width).join("\n");
    frame.render_widget(Paragraph::new(text), inner);
}

/// One line per step: its mark, its label, a bar placed on the time axis of
/// the whole ticket, and its duration.
pub fn waterfall(trace: &Trace, now: OffsetDateTime, width: usize) -> Vec<String> {
    let Some((start, end)) = trace.bounds(now) else {
        return Vec::new();
    };
    let total_ms = (end - start).whole_milliseconds().max(1) as u64;
    let mut out = vec![format!(
        "{:LABEL$} 0 ─ {}",
        "",
        fmt_duration(total_ms),
    )];
    for span in &trace.spans {
        let offset = (span.start - start).whole_milliseconds().max(0) as u64;
        out.push(format!(
            "{:LABEL$} {} {}",
            label_of(span),
            bar(offset, span.duration_ms(now), total_ms, width),
            fmt_duration(span.duration_ms(now)),
        ));
    }
    out
}

fn label_of(span: &Span) -> String {
    let mark = match span.ok {
        Some(true) => '✓',
        Some(false) => '✗',
        None => '…',
    };
    let text = format!("{mark} {} {}", span.kind.label_fr(), span.label);
    let mut chars: Vec<char> = text.chars().collect();
    if chars.len() > LABEL {
        chars.truncate(LABEL - 1);
        chars.push('…');
    }
    chars.into_iter().collect()
}

/// `width` cells for the ticket's whole life; the step fills its share, and a
/// step too short to see still gets one cell.
pub fn bar(offset_ms: u64, duration_ms: u64, total_ms: u64, width: usize) -> String {
    let total = total_ms.max(1) as f64;
    let cell = |ms: u64| ((ms as f64 / total) * width as f64).round() as usize;
    let from = cell(offset_ms).min(width.saturating_sub(1));
    let len = cell(duration_ms).max(1).min(width - from);
    let mut s = String::with_capacity(width * 3);
    s.push_str(&"·".repeat(from));
    s.push_str(&"█".repeat(len));
    s.push_str(&"·".repeat(width - from - len));
    s
}

fn fmt_duration(ms: u64) -> String {
    let secs = ms / 1000;
    match secs {
        0 => format!("{ms} ms"),
        1..=119 => format!("{secs} s"),
        120..=7199 => format!("{} min", secs / 60),
        _ => format!("{} h {:02}", secs / 3600, secs % 3600 / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::observe::{AlertEntry, Firing, MonitorKind, Series, SpanKind};
    use time::Duration;

    fn draw(app: &App, w: u16, h: u16) -> String {
        crate::screens::text_of(w, h, |f| crate::screens::render(app, f))
    }

    fn app_on(tab: ObserveTab) -> App {
        let mut app = App::new();
        app.screen = crate::app::Screen::Observe;
        app.observe.tab = tab;
        app
    }

    fn firing() -> Firing {
        Firing {
            monitor: "budget".into(),
            kind: MonitorKind::CostPerHour,
            value: 4.2,
            threshold: 3.0,
        }
    }

    #[test]
    fn a_bar_sits_on_the_time_axis_and_never_vanishes() {
        assert_eq!(bar(0, 100, 100, 4), "████");
        assert_eq!(bar(50, 50, 100, 4), "··██");
        // A step of a few milliseconds is still one cell, at the right place.
        assert_eq!(bar(100, 1, 100, 4), "···█");
        assert_eq!(bar(0, 0, 0, 4).chars().count(), 4);
    }

    #[test]
    fn durations_read_in_the_unit_that_fits() {
        assert_eq!(fmt_duration(40), "40 ms");
        assert_eq!(fmt_duration(59_000), "59 s");
        assert_eq!(fmt_duration(300_000), "5 min");
        assert_eq!(fmt_duration(7_500_000), "2 h 05");
    }

    #[test]
    fn the_metrics_tab_waits_then_draws_its_three_series() {
        let mut app = app_on(ObserveTab::Metrics);
        assert!(draw(&app, 100, 24).contains("Lecture en cours"));
        app.observe.series = Some(Series {
            start_unix: 0,
            step_secs: 1800,
            tokens: vec![10, 2_000, 300],
            cost_usd: vec![0.1, 4.0, 0.5],
            messages: vec![1, 9, 3],
        });
        let out = draw(&app, 100, 24);
        assert!(out.contains("Tokens"), "{out}");
        assert!(out.contains("indicatif"));
        assert!(out.contains("Messages"));
        assert!(out.contains("[métriques]"));
    }

    #[test]
    fn without_monitors_the_alerts_tab_says_how_to_add_one() {
        let out = draw(&app_on(ObserveTab::Alerts), 100, 20);
        assert!(out.contains("Aucune alerte définie"), "{out}");
        assert!(out.contains("[[monitors]]"));
    }

    #[test]
    fn a_standing_alert_has_a_symbol_and_shows_in_the_tab() {
        let mut app = app_on(ObserveTab::Alerts);
        app.observe.monitors = 2;
        app.observe.active = vec![firing()];
        app.observe.history = vec![AlertEntry {
            ts: orchestra_core::now(),
            monitor: "budget".into(),
            raised: true,
            firing: Some(firing()),
        }];
        let out = draw(&app, 110, 20);
        assert!(out.contains("⚠ budget"), "{out}");
        assert!(out.contains("4,20 $"));
        assert!(out.contains("▲"));
        assert!(out.contains("alertes ⚠ 1"), "l'onglet compte les alertes");
    }

    #[test]
    fn quiet_monitors_say_so() {
        let mut app = app_on(ObserveTab::Alerts);
        app.observe.monitors = 2;
        assert!(draw(&app, 100, 20).contains("Rien au-dessus de son seuil"));
    }

    #[test]
    fn the_trace_tab_explains_each_state_it_can_be_in() {
        let mut app = app_on(ObserveTab::Trace);
        assert!(draw(&app, 100, 12).contains("Aucun ticket choisi"));
        app.observe.trace_for = Some(uuid::Uuid::new_v4());
        assert!(draw(&app, 100, 12).contains("Lecture en cours"));
        app.observe.trace = Some(Trace::default());
        assert!(draw(&app, 100, 12).contains("aucune étape"));
    }

    #[test]
    fn a_waterfall_marks_each_step_with_a_symbol() {
        let t0 = orchestra_core::now();
        let span = |kind, label: &str, from: i64, to: Option<i64>, ok| Span {
            kind,
            label: label.into(),
            start: t0 + Duration::seconds(from),
            end: to.map(|s| t0 + Duration::seconds(s)),
            ok,
        };
        let trace = Trace {
            spans: vec![
                span(SpanKind::Agent, "dev", 0, Some(60), Some(true)),
                span(SpanKind::Check, "cargo test", 60, Some(90), Some(false)),
                span(SpanKind::Agent, "fix", 90, None, None),
            ],
        };
        let lines = waterfall(&trace, t0 + Duration::seconds(120), 20);
        assert_eq!(lines.len(), 4);
        assert!(lines[1].contains("✓ agent dev"));
        assert!(lines[2].contains("✗ vérification cargo test"));
        assert!(lines[3].contains("… agent fix"), "une étape en cours ne dit ni oui ni non");
        assert!(lines[0].contains("2 min"));
    }

    #[test]
    fn tiny_terminals_do_not_panic() {
        let mut app = app_on(ObserveTab::Trace);
        app.observe.trace_for = Some(uuid::Uuid::new_v4());
        app.observe.trace = Some(Trace {
            spans: vec![Span {
                kind: SpanKind::Merge,
                label: "dans main".into(),
                start: orchestra_core::now(),
                end: None,
                ok: None,
            }],
        });
        for tab in ObserveTab::ALL {
            app.observe.tab = tab;
            for (w, h) in [(20u16, 6u16), (10, 4), (40, 10), (200, 60)] {
                let _ = draw(&app, w, h);
            }
        }
    }
}
