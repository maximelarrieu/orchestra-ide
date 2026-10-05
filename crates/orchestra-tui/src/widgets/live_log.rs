//! The live log of one agent.
//!
//! Events arrive as a stream and are turned into lines a person can follow:
//! what the agent said, which tool it reached for, whether it worked. Raw JSON
//! never appears; neither does the agent's reasoning, only its size.

use std::collections::VecDeque;

use orchestra_core::events::{Event, EventKind};
use orchestra_core::model::Tokens;

/// How many lines one agent keeps in memory. Older ones are paged back from
/// the store when the user scrolls up.
pub const CAPACITY: usize = 5_000;

/// What a line is, which decides how it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// The agent's own words.
    Text,
    /// A tool call in flight.
    ToolRunning,
    ToolOk,
    ToolFailed,
    /// The guard refused a call.
    Blocked,
    /// A message the user sent.
    Steer,
    /// Status changes, spawns, results.
    Notice,
    Thinking,
}

impl LineKind {
    pub const ALL: [LineKind; 8] = [
        LineKind::Text,
        LineKind::ToolRunning,
        LineKind::ToolOk,
        LineKind::ToolFailed,
        LineKind::Blocked,
        LineKind::Steer,
        LineKind::Notice,
        LineKind::Thinking,
    ];
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub kind: LineKind,
    pub text: String,
    /// `HH:MM:SS`, empty for continuation lines.
    pub stamp: String,
    /// Set for tool lines so the result can update the call in place.
    pub tool_use_id: Option<String>,
}

/// The rolling view of one agent.
#[derive(Debug, Default)]
pub struct LiveLog {
    lines: VecDeque<LogLine>,
    pub tokens: Tokens,
    /// Assistant turns seen.
    pub turns: u32,
    /// Size of the reasoning of the current turn, reset when it speaks.
    pub thinking_chars: usize,
    /// True once the run has ended.
    pub finished: bool,
    /// Scroll offset from the bottom; zero means following.
    pub scroll: usize,
}

impl LiveLog {
    pub fn clear(&mut self) {
        self.lines.clear();
        self.tokens = Tokens::default();
        self.turns = 0;
        self.thinking_chars = 0;
        self.finished = false;
        self.scroll = 0;
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn is_following(&self) -> bool {
        self.scroll == 0
    }

    /// Lines to draw for a window `height` tall, oldest first.
    pub fn window(&self, height: usize) -> Vec<&LogLine> {
        let total = self.lines.len();
        let end = total.saturating_sub(self.scroll);
        let start = end.saturating_sub(height);
        self.lines.range(start..end).collect()
    }

    pub fn scroll_up(&mut self, amount: usize, height: usize) {
        let max = self.lines.len().saturating_sub(height);
        self.scroll = (self.scroll + amount).min(max);
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll = self.scroll.saturating_sub(amount);
    }

    pub fn follow(&mut self) {
        self.scroll = 0;
    }

    /// Fold one event in. Returns false when the event had nothing to show.
    pub fn push_event(&mut self, event: &Event) -> bool {
        let stamp = {
            let t = event.ts.time();
            format!("{:02}:{:02}:{:02}", t.hour(), t.minute(), t.second())
        };
        match &event.kind {
            EventKind::AgentText { text } => {
                self.turns += 1;
                self.thinking_chars = 0;
                for (i, line) in text.lines().enumerate() {
                    self.push(LogLine {
                        kind: LineKind::Text,
                        text: line.to_string(),
                        stamp: if i == 0 { stamp.clone() } else { String::new() },
                        tool_use_id: None,
                    });
                }
                true
            }
            EventKind::AgentThinking { chars } => {
                // Reasoning is not shown, only its weight, and it replaces the
                // previous count rather than adding a line each time.
                self.thinking_chars += chars;
                false
            }
            EventKind::ToolStarted {
                tool_use_id,
                tool,
                summary,
            } => {
                self.push(LogLine {
                    kind: LineKind::ToolRunning,
                    text: format!("{tool}  {summary}"),
                    stamp,
                    tool_use_id: Some(tool_use_id.clone()),
                });
                true
            }
            EventKind::ToolFinished {
                tool_use_id,
                ok,
                summary,
            } => {
                // Update the call in place rather than adding a second line.
                if let Some(line) = self
                    .lines
                    .iter_mut()
                    .rev()
                    .find(|l| l.tool_use_id.as_deref() == Some(tool_use_id.as_str()))
                {
                    line.kind = if *ok {
                        LineKind::ToolOk
                    } else {
                        LineKind::ToolFailed
                    };
                    if !ok && !summary.is_empty() {
                        line.text = format!("{}  — {summary}", line.text);
                    }
                    return true;
                }
                self.push(LogLine {
                    kind: if *ok {
                        LineKind::ToolOk
                    } else {
                        LineKind::ToolFailed
                    },
                    text: summary.clone(),
                    stamp,
                    tool_use_id: Some(tool_use_id.clone()),
                });
                true
            }
            EventKind::HookBlocked { tool, reason } => {
                self.push(LogLine {
                    kind: LineKind::Blocked,
                    text: format!("{tool} bloqué : {reason}"),
                    stamp,
                    tool_use_id: None,
                });
                true
            }
            EventKind::AgentSteered { text, hard, .. } => {
                self.push(LogLine {
                    kind: LineKind::Steer,
                    text: format!(
                        "{} {text}",
                        if *hard { "redirection :" } else { "consigne :" }
                    ),
                    stamp,
                    tool_use_id: None,
                });
                true
            }
            EventKind::Usage { sample } => {
                self.tokens += sample.tokens;
                false
            }
            EventKind::AgentSpawned { role, pid, .. } => {
                self.push(LogLine {
                    kind: LineKind::Notice,
                    text: format!("agent {role} démarré (pid {pid})"),
                    stamp,
                    tool_use_id: None,
                });
                true
            }
            EventKind::AgentStatusChanged { status, reason } => {
                if status.is_terminal() {
                    self.finished = true;
                }
                let text = match reason {
                    Some(r) => format!("{} — {}", status.label_fr(), r.label_fr()),
                    None => status.label_fr().to_string(),
                };
                self.push(LogLine {
                    kind: LineKind::Notice,
                    text,
                    stamp,
                    tool_use_id: None,
                });
                true
            }
            EventKind::AgentResult {
                num_turns,
                duration_ms,
                ..
            } => {
                self.push(LogLine {
                    kind: LineKind::Notice,
                    text: format!(
                        "terminé en {num_turns} tour(s), {:.0} s",
                        *duration_ms as f64 / 1000.0
                    ),
                    stamp,
                    tool_use_id: None,
                });
                true
            }
            _ => false,
        }
    }

    fn push(&mut self, line: LogLine) {
        // Following stays following; a reader who scrolled up keeps their place.
        if !self.is_following() {
            self.scroll += 1;
        }
        self.lines.push_back(line);
        while self.lines.len() > CAPACITY {
            self.lines.pop_front();
            if self.scroll > 0 {
                self.scroll -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::events::NewEvent;
    use orchestra_core::model::{AgentStatus, ExitReason, UsageSample, UsageSource};
    use uuid::Uuid;

    fn event(kind: EventKind) -> Event {
        Event::from_new(1, NewEvent::new(kind))
    }

    fn log_with(kinds: Vec<EventKind>) -> LiveLog {
        let mut log = LiveLog::default();
        for k in kinds {
            log.push_event(&event(k));
        }
        log
    }

    #[test]
    fn an_agent_turn_reads_like_a_conversation() {
        let log = log_with(vec![
            EventKind::AgentSpawned {
                role: "backend".into(),
                session_id: Uuid::new_v4(),
                pid: 42,
                cmdline: "claude -p".into(),
            },
            EventKind::AgentText {
                text: "Je lance les tests.".into(),
            },
            EventKind::ToolStarted {
                tool_use_id: "t1".into(),
                tool: "Bash".into(),
                summary: "cargo test".into(),
            },
            EventKind::ToolFinished {
                tool_use_id: "t1".into(),
                ok: true,
                summary: "12 passed".into(),
            },
        ]);
        let lines: Vec<&LogLine> = log.window(10);
        assert_eq!(lines.len(), 3, "l'outil tient sur une seule ligne");
        assert_eq!(lines[1].kind, LineKind::Text);
        assert_eq!(lines[2].kind, LineKind::ToolOk);
        assert!(lines[2].text.contains("cargo test"));
        assert_eq!(log.turns, 1);
    }

    #[test]
    fn a_failed_tool_keeps_its_reason_on_the_same_line() {
        let log = log_with(vec![
            EventKind::ToolStarted {
                tool_use_id: "t1".into(),
                tool: "Bash".into(),
                summary: "cargo build".into(),
            },
            EventKind::ToolFinished {
                tool_use_id: "t1".into(),
                ok: false,
                summary: "error[E0308]".into(),
            },
        ]);
        let lines = log.window(10);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].kind, LineKind::ToolFailed);
        assert!(lines[0].text.contains("cargo build"));
        assert!(lines[0].text.contains("E0308"));
    }

    #[test]
    fn reasoning_is_counted_never_shown() {
        let mut log = log_with(vec![EventKind::AgentThinking { chars: 1200 }]);
        assert!(log.is_empty(), "aucune ligne pour la réflexion");
        assert_eq!(log.thinking_chars, 1200);
        log.push_event(&event(EventKind::AgentThinking { chars: 300 }));
        assert_eq!(log.thinking_chars, 1500, "la réflexion s'accumule");
        log.push_event(&event(EventKind::AgentText {
            text: "voilà".into(),
        }));
        assert_eq!(log.thinking_chars, 0, "et repart à zéro quand il parle");
    }

    #[test]
    fn a_refusal_by_the_guard_stands_out() {
        let log = log_with(vec![EventKind::HookBlocked {
            tool: "Bash".into(),
            reason: "hors du worktree".into(),
        }]);
        assert_eq!(log.window(10)[0].kind, LineKind::Blocked);
        assert!(log.window(10)[0].text.contains("hors du worktree"));
    }

    #[test]
    fn steering_appears_in_the_log() {
        let log = log_with(vec![
            EventKind::AgentSteered {
                text: "ajoute un test".into(),
                by: "user".into(),
                hard: false,
            },
            EventKind::AgentSteered {
                text: "stop, autre approche".into(),
                by: "user".into(),
                hard: true,
            },
        ]);
        let lines = log.window(10);
        assert!(lines[0].text.starts_with("consigne :"));
        assert!(lines[1].text.starts_with("redirection :"));
    }

    #[test]
    fn tokens_accumulate_without_adding_lines() {
        let mut log = LiveLog::default();
        for output in [10u64, 20] {
            log.push_event(&event(EventKind::Usage {
                sample: Box::new(UsageSample {
                    message_id: "m".into(),
                    session_id: Uuid::new_v4(),
                    subagent_id: None,
                    agent_id: None,
                    ticket_id: None,
                    project_id: None,
                    model: "claude-opus-5".into(),
                    tokens: Tokens {
                        input: 1,
                        output,
                        cache_read: 5,
                        cache_creation: 0,
                        thinking: 0,
                    },
                    ts: orchestra_core::now(),
                    source: UsageSource::Stream,
                }),
            }));
        }
        assert!(log.is_empty());
        assert_eq!(log.tokens.output, 30);
        assert_eq!(log.tokens.cache_read, 10);
    }

    #[test]
    fn the_end_of_a_run_is_recorded() {
        let log = log_with(vec![
            EventKind::AgentResult {
                subtype: "success".into(),
                num_turns: 4,
                duration_ms: 12_300,
                total_cost_usd: Some(0.4),
                text: "fini".into(),
            },
            EventKind::AgentStatusChanged {
                status: AgentStatus::Done,
                reason: Some(ExitReason::Success),
            },
        ]);
        assert!(log.finished);
        let lines = log.window(10);
        assert!(lines[0].text.contains("4 tour"));
        assert!(lines[0].text.contains("12 s"));
        assert!(lines[1].text.contains("succès"));
    }

    #[test]
    fn multi_line_text_keeps_one_timestamp() {
        let log = log_with(vec![EventKind::AgentText {
            text: "première ligne\nseconde ligne".into(),
        }]);
        let lines = log.window(10);
        assert_eq!(lines.len(), 2);
        assert!(!lines[0].stamp.is_empty());
        assert!(lines[1].stamp.is_empty(), "pas d'horodatage répété");
    }

    #[test]
    fn scrolling_holds_its_place_while_the_agent_keeps_talking() {
        let mut log = LiveLog::default();
        for i in 0..50 {
            log.push_event(&event(EventKind::AgentText {
                text: format!("ligne {i}"),
            }));
        }
        assert!(log.is_following());
        assert_eq!(log.window(5).last().unwrap().text, "ligne 49");

        log.scroll_up(10, 5);
        assert!(!log.is_following());
        let before = log.window(5).last().unwrap().text.clone();

        // New output must not drag the reader back down.
        log.push_event(&event(EventKind::AgentText {
            text: "ligne 50".into(),
        }));
        assert_eq!(log.window(5).last().unwrap().text, before);

        log.follow();
        assert_eq!(log.window(5).last().unwrap().text, "ligne 50");
    }

    #[test]
    fn scrolling_stops_at_both_ends() {
        let mut log = LiveLog::default();
        for i in 0..10 {
            log.push_event(&event(EventKind::AgentText {
                text: format!("{i}"),
            }));
        }
        log.scroll_up(1000, 5);
        assert_eq!(log.scroll, 5, "on ne remonte pas au-delà du début");
        log.scroll_down(1000);
        assert_eq!(log.scroll, 0);
    }

    #[test]
    fn the_log_is_bounded() {
        let mut log = LiveLog::default();
        for i in 0..(CAPACITY + 200) {
            log.push_event(&event(EventKind::AgentText {
                text: format!("{i}"),
            }));
        }
        assert_eq!(log.len(), CAPACITY);
        assert_eq!(log.window(1)[0].text, (CAPACITY + 199).to_string());
    }
}
