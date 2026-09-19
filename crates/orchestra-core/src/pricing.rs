//! Indicative USD pricing.
//!
//! The user is on a Claude subscription: no dollar amount is actually billed
//! per request. Raw tokens are the truth; this turns them into a number that
//! makes runs comparable. Prices live in `config.toml` and are applied at query
//! time, so editing the table re-prices the whole history.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::Tokens;

/// USD per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub cache_write: f64,
}

const PER_MILLION: f64 = 1_000_000.0;

impl ModelPrice {
    /// Cost of one sample. Thinking tokens are already inside `output`, so they
    /// are never added again.
    pub fn cost(&self, t: &Tokens) -> f64 {
        (t.input as f64 * self.input
            + t.output as f64 * self.output
            + t.cache_read as f64 * self.cache_read
            + t.cache_creation as f64 * self.cache_write)
            / PER_MILLION
    }
}

/// Model prefix to price. Lookup is longest-prefix, so `claude-sonnet-5` covers
/// `claude-sonnet-5-20260514` without listing every dated snapshot.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PriceTable {
    #[serde(flatten, default)]
    pub models: BTreeMap<String, ModelPrice>,
    /// Applied to models absent from the table, so an unknown model still shows
    /// a number rather than a blank.
    #[serde(default)]
    pub fallback: Option<ModelPrice>,
}

impl PriceTable {
    /// Longest matching prefix wins; `fallback` otherwise.
    pub fn lookup(&self, model: &str) -> Option<&ModelPrice> {
        self.models
            .iter()
            .filter(|(prefix, _)| model.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(_, price)| price)
            .or(self.fallback.as_ref())
    }

    /// `None` when the model is unknown and no fallback is configured.
    pub fn cost(&self, model: &str, tokens: &Tokens) -> Option<f64> {
        self.lookup(model).map(|p| p.cost(tokens))
    }

    /// True when the price came from `fallback` rather than a real entry.
    pub fn is_estimated(&self, model: &str) -> bool {
        !self.models.keys().any(|p| model.starts_with(p.as_str()))
    }

    /// Table shipped in `assets/config.example.toml`. Public list prices as of
    /// 2026-09; the user is expected to edit them.
    ///
    /// `cache_write` is the **one-hour** rate, twice the input price, not the
    /// five-minute rate of 1.25 times. That is not a guess: a real run was
    /// compared against the cost Claude Code reports for itself, and the
    /// five-minute rate under-estimated it by a third. Claude Code caches for
    /// an hour by default, which is what the transcripts show.
    pub fn defaults() -> Self {
        let mut models = BTreeMap::new();
        let mut add = |name: &str, input, output, cache_read, cache_write| {
            models.insert(
                name.to_string(),
                ModelPrice {
                    input,
                    output,
                    cache_read,
                    cache_write,
                },
            );
        };
        //  model             input  output  cache_read  cache_write (1 h)
        add("claude-fable-5", 10.0, 50.0, 1.0, 20.0);
        add("claude-mythos-5", 10.0, 50.0, 1.0, 20.0);
        add("claude-opus-5", 5.0, 25.0, 0.5, 10.0);
        add("claude-opus-4", 5.0, 25.0, 0.5, 10.0);
        add("claude-sonnet-5", 2.0, 10.0, 0.2, 4.0);
        add("claude-sonnet-4", 3.0, 15.0, 0.3, 6.0);
        add("claude-haiku-4", 1.0, 5.0, 0.1, 2.0);
        PriceTable {
            models,
            fallback: Some(ModelPrice {
                input: 5.0,
                output: 25.0,
                cache_read: 0.5,
                cache_write: 10.0,
            }),
        }
    }
}

/// `12.3k`, `1.23M`: token counts in a column of a terminal table.
pub fn fmt_tokens(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1_000.0),
        _ => format!("{:.2}M", n as f64 / 1_000_000.0),
    }
}

/// Always prefixed with the almost-equal sign: the number is indicative, never
/// a bill.
pub fn fmt_usd(v: f64) -> String {
    if v >= 100.0 {
        format!("≈${v:.0}")
    } else if v >= 1.0 {
        format!("≈${v:.2}")
    } else {
        format!("≈${v:.3}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: u64, output: u64, cache_read: u64, cache_creation: u64) -> Tokens {
        Tokens {
            input,
            output,
            cache_read,
            cache_creation,
            thinking: output / 2,
        }
    }

    #[test]
    fn longest_prefix_wins() {
        let t = PriceTable::defaults();
        // A dated snapshot resolves to its family.
        let p = t.lookup("claude-sonnet-5-20260514").unwrap();
        assert_eq!(p.input, 2.0);
        // sonnet-5 and sonnet-4 must not be confused.
        assert_eq!(t.lookup("claude-sonnet-4-6").unwrap().input, 3.0);
        assert_eq!(t.lookup("claude-opus-5").unwrap().output, 25.0);
    }

    #[test]
    fn unknown_model_falls_back_and_is_flagged() {
        let t = PriceTable::defaults();
        assert!(t.lookup("gpt-9").is_some());
        assert!(t.is_estimated("gpt-9"));
        assert!(!t.is_estimated("claude-opus-5"));

        let bare = PriceTable {
            models: BTreeMap::new(),
            fallback: None,
        };
        assert!(bare.cost("claude-opus-5", &Tokens::default()).is_none());
    }

    #[test]
    fn cost_does_not_double_bill_thinking() {
        let price = ModelPrice {
            input: 10.0,
            output: 50.0,
            cache_read: 1.0,
            cache_write: 12.5,
        };
        // 1M input, 1M output, no cache: 10 + 50, whatever the thinking share.
        let a = price.cost(&Tokens {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 0,
            cache_creation: 0,
            thinking: 0,
        });
        let b = price.cost(&Tokens {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 0,
            cache_creation: 0,
            thinking: 900_000,
        });
        assert!((a - 60.0).abs() < 1e-9);
        assert!((a - b).abs() < 1e-9);
    }

    #[test]
    fn the_table_matches_what_claude_code_reports() {
        // Measured, not assumed: one real `claude -p` call on Haiku 4.5 that
        // Claude Code priced at $0.0177983 for itself.
        let t = PriceTable::defaults();
        let observed = Tokens {
            input: 10,
            output: 43,
            cache_read: 14_053,
            cache_creation: 8_084,
            thinking: 0,
        };
        let ours = t.cost("claude-haiku-4-5-20251001", &observed).unwrap();
        let reported = 0.0177983;
        assert!(
            (ours - reported).abs() < 1e-6,
            "estimation {ours:.6} contre coût rapporté {reported:.6}"
        );
    }

    #[test]
    fn cache_is_cheaper_than_fresh_input() {
        let t = PriceTable::defaults();
        let fresh = t.cost("claude-opus-5", &tokens(100_000, 0, 0, 0)).unwrap();
        let cached = t.cost("claude-opus-5", &tokens(0, 0, 100_000, 0)).unwrap();
        assert!(cached < fresh);
    }

    #[test]
    fn empty_tokens_cost_nothing() {
        let t = PriceTable::defaults();
        assert_eq!(t.cost("claude-opus-5", &Tokens::default()).unwrap(), 0.0);
    }

    #[test]
    fn formatting_is_compact() {
        assert_eq!(fmt_tokens(0), "0");
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(12_345), "12.3k");
        assert_eq!(fmt_tokens(1_234_567), "1.23M");
        assert_eq!(fmt_usd(0.0123), "≈$0.012");
        assert_eq!(fmt_usd(3.456), "≈$3.46");
        assert_eq!(fmt_usd(1234.5), "≈$1234");
    }

    #[test]
    fn table_round_trips_through_toml() {
        let t = PriceTable::defaults();
        let s = toml::to_string(&t).unwrap();
        let back: PriceTable = toml::from_str(&s).unwrap();
        assert_eq!(back, t);
    }
}
