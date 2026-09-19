//! The token ledger: who spent what, and what that is worth.
//!
//! Raw tokens are the truth. The dollar figure is indicative, applied at read
//! time from the price table, so editing the table re-prices the whole history.

use std::collections::BTreeMap;

use anyhow::Result;
use orchestra_core::config::Config;
use orchestra_core::model::{AgentId, TicketId, Tokens, UsageSample};
use orchestra_core::pricing::PriceTable;
use orchestra_core::protocol::{GroupBy, UsageQuery, UsageRow, UsageTotals};

use crate::store::{Recorded, Store, UsageBreakdown};

/// Tokens and price of one scope.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentCost {
    pub tokens: Tokens,
    pub messages: u64,
    pub cost_usd: Option<f64>,
}

#[derive(Clone)]
pub struct UsageLedger {
    pub(crate) store: Store,
    prices: PriceTable,
}

impl UsageLedger {
    pub fn new(store: Store, cfg: &Config) -> Self {
        UsageLedger {
            store,
            prices: cfg.pricing.clone(),
        }
    }

    pub fn prices(&self) -> &PriceTable {
        &self.prices
    }

    /// Record one API response. See [`crate::store::rows`] for why this merges
    /// rather than ignores duplicates.
    pub async fn record(&self, sample: UsageSample) -> Result<Recorded> {
        self.store.record_usage(sample).await
    }

    pub async fn record_batch(&self, samples: Vec<UsageSample>) -> Result<usize> {
        self.store.record_usage_batch(samples).await
    }

    /// Aggregate, then price.
    ///
    /// Each group is split by model in SQL and priced per model here, so a row
    /// covering two models costs the sum of its parts rather than an average of
    /// their rates. That is what makes the rows add up to the total.
    pub async fn rollup(&self, query: UsageQuery) -> Result<(Vec<UsageRow>, UsageTotals)> {
        let breakdown = self.store.usage_breakdown(query.clone()).await?;

        let mut grouped: BTreeMap<BTreeMap<String, String>, Vec<&UsageBreakdown>> = BTreeMap::new();
        for b in &breakdown {
            grouped.entry(b.keys.clone()).or_default().push(b);
        }

        let mut rows: Vec<UsageRow> = grouped
            .into_iter()
            .map(|(keys, parts)| {
                let mut tokens = Tokens::default();
                let mut messages = 0;
                let mut cost = 0.0;
                let mut priced = false;
                let mut estimated = false;
                for p in parts {
                    tokens += p.tokens;
                    messages += p.messages;
                    if let Some(c) = self.prices.cost(&p.model, &p.tokens) {
                        cost += c;
                        priced = true;
                    }
                    estimated |= self.prices.is_estimated(&p.model);
                }
                UsageRow {
                    keys,
                    tokens,
                    cost_usd: priced.then_some(cost),
                    messages,
                    cost_estimated: estimated,
                }
            })
            .collect();

        // Heaviest first, then cut: the ceiling applies to what is displayed,
        // never to what is counted.
        rows.sort_by_key(|r| std::cmp::Reverse(r.tokens.total()));

        let mut totals = UsageTotals::default();
        let mut cost = 0.0;
        let mut priced = false;
        let mut per_model: BTreeMap<&str, Tokens> = BTreeMap::new();
        for b in &breakdown {
            totals.tokens += b.tokens;
            totals.messages += b.messages;
            *per_model.entry(b.model.as_str()).or_default() += b.tokens;
        }
        for (model, tokens) in per_model {
            if let Some(c) = self.prices.cost(model, &tokens) {
                cost += c;
                priced = true;
            }
        }
        totals.cost_usd = priced.then_some(cost);

        rows.truncate(query.limit.max(1) as usize);
        Ok((rows, totals))
    }

    /// Tokens, turns and cost of one agent.
    ///
    /// Priced from the models the agent actually used, not from the model it
    /// was configured with: the configuration holds an alias such as `haiku`,
    /// while the samples hold `claude-haiku-4-5-20251001`. Pricing the alias
    /// misses the table, falls through to the fallback rate, and had the same
    /// agent costing five times more on one screen than on another.
    pub async fn agent_cost(&self, agent_id: AgentId) -> Result<AgentCost> {
        self.cost_of(UsageQuery {
            agent_id: Some(agent_id),
            group_by: vec![GroupBy::Model],
            limit: 200,
            ..Default::default()
        })
        .await
    }

    /// Same, for every agent of a ticket at once.
    pub async fn ticket_cost(&self, ticket_id: TicketId) -> Result<AgentCost> {
        self.cost_of(UsageQuery {
            ticket_id: Some(ticket_id),
            group_by: vec![GroupBy::Model],
            limit: 200,
            ..Default::default()
        })
        .await
    }

    async fn cost_of(&self, query: UsageQuery) -> Result<AgentCost> {
        let breakdown = self.store.usage_breakdown(query).await?;
        let mut out = AgentCost::default();
        let mut cost = 0.0;
        let mut priced = false;
        for part in &breakdown {
            out.tokens += part.tokens;
            out.messages += part.messages;
            if let Some(c) = self.prices.cost(&part.model, &part.tokens) {
                cost += c;
                priced = true;
            }
        }
        out.cost_usd = priced.then_some(cost);
        Ok(out)
    }

    pub async fn ticket_tokens(&self, ticket_id: TicketId) -> Result<Tokens> {
        self.store.ticket_usage(ticket_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::model::UsageSource;
    use orchestra_core::protocol::TimeRange;
    use time::OffsetDateTime;
    use uuid::Uuid;

    fn sample(id: &str, model: &str, out: u64, session: Uuid) -> UsageSample {
        UsageSample {
            message_id: id.into(),
            session_id: session,
            subagent_id: None,
            agent_id: None,
            ticket_id: None,
            project_id: None,
            model: model.into(),
            tokens: Tokens {
                input: 10,
                output: out,
                cache_read: 1000,
                cache_creation: 0,
                thinking: out / 2,
            },
            ts: OffsetDateTime::now_utc(),
            source: UsageSource::Transcript,
        }
    }

    async fn ledger() -> UsageLedger {
        let store = Store::open_memory().unwrap();
        UsageLedger::new(store, &Config::default())
    }

    #[tokio::test]
    async fn a_response_is_counted_once_however_often_it_arrives() {
        let l = ledger().await;
        let s = Uuid::new_v4();
        // The transcript writes one line per content block, the early ones
        // partial; the agent's stdout reports the same response again.
        assert_eq!(
            l.record(sample("msg_1", "claude-opus-5", 5, s))
                .await
                .unwrap(),
            Recorded::New
        );
        assert_eq!(
            l.record(sample("msg_1", "claude-opus-5", 5, s))
                .await
                .unwrap(),
            Recorded::Unchanged
        );
        assert_eq!(
            l.record(sample("msg_1", "claude-opus-5", 787, s))
                .await
                .unwrap(),
            Recorded::Updated
        );
        assert_eq!(
            l.record(sample("msg_1", "claude-opus-5", 5, s))
                .await
                .unwrap(),
            Recorded::Unchanged
        );

        let (_, totals) = l.rollup(UsageQuery::default()).await.unwrap();
        assert_eq!(totals.messages, 1, "une réponse, une ligne");
        assert_eq!(
            totals.tokens.output, 787,
            "le total complet, pas le partiel"
        );
        assert_eq!(totals.tokens.cache_read, 1000, "le cache n'est pas cumulé");
    }

    #[tokio::test]
    async fn separate_responses_add_up() {
        let l = ledger().await;
        let s = Uuid::new_v4();
        l.record(sample("msg_1", "claude-opus-5", 100, s))
            .await
            .unwrap();
        l.record(sample("msg_2", "claude-opus-5", 200, s))
            .await
            .unwrap();
        let (_, totals) = l.rollup(UsageQuery::default()).await.unwrap();
        assert_eq!(totals.messages, 2);
        assert_eq!(totals.tokens.output, 300);
    }

    #[tokio::test]
    async fn cost_follows_the_price_table() {
        let l = ledger().await;
        let s = Uuid::new_v4();
        // 1M output tokens on opus-5 is 25 USD in the default table.
        let mut big = sample("msg_1", "claude-opus-5", 1_000_000, s);
        big.tokens = Tokens {
            input: 0,
            output: 1_000_000,
            cache_read: 0,
            cache_creation: 0,
            thinking: 0,
        };
        l.record(big).await.unwrap();
        let (rows, totals) = l.rollup(UsageQuery::default()).await.unwrap();
        let cost = totals.cost_usd.unwrap();
        assert!((cost - 25.0).abs() < 1e-6, "coût attendu 25, obtenu {cost}");
        assert!(!rows[0].cost_estimated, "opus-5 est dans la grille");
    }

    #[tokio::test]
    async fn an_unknown_model_is_priced_but_flagged() {
        let l = ledger().await;
        let s = Uuid::new_v4();
        l.record(sample("msg_1", "un-modele-inconnu", 100, s))
            .await
            .unwrap();
        let (rows, totals) = l.rollup(UsageQuery::default()).await.unwrap();
        assert!(
            totals.cost_usd.is_some(),
            "le repli donne quand même un ordre de grandeur"
        );
        assert!(rows[0].cost_estimated, "et le signale");
    }

    #[tokio::test]
    async fn grouping_by_model_splits_the_bill() {
        let l = ledger().await;
        let s = Uuid::new_v4();
        l.record(sample("a", "claude-opus-5", 100, s))
            .await
            .unwrap();
        l.record(sample("b", "claude-haiku-4-5", 100, s))
            .await
            .unwrap();
        let q = UsageQuery {
            group_by: vec![GroupBy::Model],
            ..Default::default()
        };
        let (rows, totals) = l.rollup(q).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(totals.messages, 2);
        // Same tokens, cheaper model: the two rows must not cost the same.
        let costs: Vec<f64> = rows.iter().filter_map(|r| r.cost_usd).collect();
        assert_eq!(costs.len(), 2);
        assert!((costs[0] - costs[1]).abs() > 1e-9);
        // And the total is their sum.
        assert!((totals.cost_usd.unwrap() - costs.iter().sum::<f64>()).abs() < 1e-9);
    }

    #[tokio::test]
    async fn the_time_window_filters() {
        let l = ledger().await;
        let s = Uuid::new_v4();
        let mut old = sample("vieux", "claude-opus-5", 100, s);
        old.ts = OffsetDateTime::now_utc() - time::Duration::days(30);
        l.record(old).await.unwrap();
        l.record(sample("recent", "claude-opus-5", 100, s))
            .await
            .unwrap();

        let (_, all) = l.rollup(UsageQuery::default()).await.unwrap();
        assert_eq!(all.messages, 2);

        let week = UsageQuery {
            range: TimeRange::last_days(7),
            ..Default::default()
        };
        let (_, recent) = l.rollup(week).await.unwrap();
        assert_eq!(recent.messages, 1);
    }

    #[tokio::test]
    async fn an_agent_is_priced_by_the_models_it_used() {
        // Regression: pricing by the configured alias ("haiku") missed the
        // table and fell back to the expensive default rate, so the same agent
        // showed two different costs depending on the screen.
        let l = ledger().await;
        let store = l.store.clone();
        let agent_id = Uuid::new_v4();
        let ticket_id = Uuid::new_v4();

        let mut s = sample(
            "msg_1",
            "claude-haiku-4-5-20251001",
            1_000_000,
            Uuid::new_v4(),
        );
        s.tokens = Tokens {
            input: 0,
            output: 1_000_000,
            cache_read: 0,
            cache_creation: 0,
            thinking: 0,
        };
        s.agent_id = Some(agent_id);
        s.ticket_id = Some(ticket_id);
        store.record_usage(s).await.unwrap();

        // One million haiku output tokens is 5 USD, not the 25 of the fallback.
        let cost = l.agent_cost(agent_id).await.unwrap();
        assert_eq!(cost.messages, 1);
        assert_eq!(cost.tokens.output, 1_000_000);
        assert!(
            (cost.cost_usd.unwrap() - 5.0).abs() < 1e-6,
            "{:?}",
            cost.cost_usd
        );

        // The ticket agrees with its agents.
        let ticket = l.ticket_cost(ticket_id).await.unwrap();
        assert_eq!(ticket.cost_usd, cost.cost_usd);

        // And so does the general rollup.
        let (_, totals) = l.rollup(UsageQuery::default()).await.unwrap();
        assert!((totals.cost_usd.unwrap() - 5.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn unmanaged_sessions_can_be_excluded() {
        let l = ledger().await;
        let s = Uuid::new_v4();
        l.record(sample("libre", "claude-opus-5", 100, s))
            .await
            .unwrap();
        let q = UsageQuery {
            include_unmanaged: false,
            ..Default::default()
        };
        let (_, totals) = l.rollup(q).await.unwrap();
        assert_eq!(totals.messages, 0, "aucun agent rattaché à cet échantillon");
    }
}
