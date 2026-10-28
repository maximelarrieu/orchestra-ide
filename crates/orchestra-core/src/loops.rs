//! Spotting an agent that repeats itself. Pure: it sees the tool calls the
//! stream already carries, and says when the same one has come back too often
//! in a row. What to do about it is the supervisor's (rule 15).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct RepeatDetector {
    threshold: u32,
    last: Option<u64>,
    count: u32,
}

impl RepeatDetector {
    /// `threshold` identical calls in a row make a loop. `0` turns it off.
    pub fn new(threshold: u32) -> Self {
        RepeatDetector { threshold, last: None, count: 0 }
    }

    /// Feed one tool call. `Some(n)` exactly once per streak, when it reaches
    /// the threshold; a different call starts the count over.
    pub fn observe(&mut self, tool: &str, input: &Value) -> Option<u32> {
        if self.threshold == 0 {
            return None;
        }
        let mut h = DefaultHasher::new();
        tool.hash(&mut h);
        input.to_string().hash(&mut h);
        let key = h.finish();
        if self.last == Some(key) {
            self.count += 1;
        } else {
            self.last = Some(key);
            self.count = 1;
        }
        (self.count == self.threshold).then_some(self.count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_same_call_in_a_row_trips_once_at_the_threshold() {
        let mut d = RepeatDetector::new(3);
        let call = json!({"command": "cargo test"});
        assert_eq!(d.observe("Bash", &call), None);
        assert_eq!(d.observe("Bash", &call), None);
        assert_eq!(d.observe("Bash", &call), Some(3));
        assert_eq!(d.observe("Bash", &call), None, "une fois par série");
    }

    #[test]
    fn a_different_call_breaks_the_streak() {
        let mut d = RepeatDetector::new(3);
        let a = json!({"command": "cargo test"});
        d.observe("Bash", &a);
        d.observe("Bash", &a);
        assert_eq!(d.observe("Read", &json!({"file_path": "x"})), None);
        assert_eq!(d.observe("Bash", &a), None);
        assert_eq!(d.observe("Bash", &a), None);
        assert_eq!(d.observe("Bash", &a), Some(3));
    }

    #[test]
    fn zero_is_off() {
        let mut d = RepeatDetector::new(0);
        for _ in 0..50 {
            assert_eq!(d.observe("Bash", &json!({})), None);
        }
    }
}
