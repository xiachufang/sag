use std::sync::Arc;

use arc_swap::ArcSwap;
use chrono::{Datelike, TimeZone, Utc};
use dashmap::DashSet;
use gateway_core::config::{AppConfig, BudgetConfig, BudgetThreshold};
use gateway_storage::traits::{CounterStore, MetadataStore};
use serde::Serialize;

#[derive(Clone)]
pub struct BudgetManager {
    counter: Arc<dyn CounterStore>,
    #[allow(dead_code)]
    metadata: Arc<dyn MetadataStore>,
    config: Arc<ArcSwap<AppConfig>>,
    notified: Arc<DashSet<(String, i64, u32)>>, // (budget_id, period_start, threshold_pct)
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize)]
pub struct BudgetUsage {
    pub budget_id: String,
    pub name: String,
    pub period: String,
    pub period_start: i64,
    pub amount_usd: f64,
    pub used_usd: f64,
    pub pct: f64,
    pub blocked: bool,
}

impl BudgetManager {
    pub fn new(
        counter: Arc<dyn CounterStore>,
        metadata: Arc<dyn MetadataStore>,
        config: Arc<ArcSwap<AppConfig>>,
    ) -> Self {
        Self {
            counter,
            metadata,
            config,
            notified: Arc::new(DashSet::new()),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("budget http client"),
        }
    }

    fn matching_budgets(&self, project_id: &str, key_id: &str) -> Vec<BudgetConfig> {
        self.config
            .load()
            .budgets
            .iter()
            .filter(|b| {
                let tgt = &b.target;
                let project_ok = tgt
                    .project_id
                    .as_deref()
                    .map(|p| p == project_id)
                    .unwrap_or(true);
                let key_ok = tgt
                    .gateway_key_id
                    .as_deref()
                    .map(|k| k == key_id)
                    .unwrap_or(true);
                project_ok && key_ok
            })
            .cloned()
            .collect()
    }

    /// Check whether a request should be blocked because some matching
    /// budget has crossed a `block` threshold.
    pub async fn check_block(&self, project_id: &str, key_id: &str) -> bool {
        for b in self.matching_budgets(project_id, key_id) {
            let period_start = period_start_for(&b.period);
            let used = self
                .counter
                .read_budget(&b.name, period_start)
                .await
                .unwrap_or(0.0);
            let pct = if b.amount_usd > 0.0 { used / b.amount_usd } else { 0.0 };
            for t in &b.thresholds {
                if pct >= t.at && t.action == "block" {
                    return true;
                }
            }
        }
        false
    }

    /// Record a cost against all matching budgets, possibly firing
    /// notification webhooks for threshold crossings.
    pub async fn record_cost(&self, project_id: &str, key_id: &str, cost_usd: f64) {
        if cost_usd <= 0.0 {
            return;
        }
        for b in self.matching_budgets(project_id, key_id) {
            let period_start = period_start_for(&b.period);
            let used = self
                .counter
                .incr_budget(&b.name, period_start, cost_usd)
                .await
                .unwrap_or(0.0);
            let pct = if b.amount_usd > 0.0 { used / b.amount_usd } else { 0.0 };
            metrics::histogram!("gateway_budget_pct", "budget" => b.name.clone())
                .record(pct);
            for t in &b.thresholds {
                if pct >= t.at {
                    let bucket = (t.at * 100.0) as u32;
                    let dedupe_key = (b.name.clone(), period_start, bucket);
                    if self.notified.insert(dedupe_key) {
                        self.fire_threshold(&b, t, used, pct).await;
                    }
                }
            }
        }
    }

    async fn fire_threshold(
        &self,
        budget: &BudgetConfig,
        threshold: &BudgetThreshold,
        used: f64,
        pct: f64,
    ) {
        tracing::info!(
            budget = %budget.name,
            action = %threshold.action,
            pct = pct,
            used = used,
            "budget threshold crossed"
        );
        metrics::counter!(
            "gateway_budget_threshold_total",
            "budget" => budget.name.clone(),
            "action" => threshold.action.clone()
        )
        .increment(1);
        if let Some(url) = &threshold.webhook {
            let payload = serde_json::json!({
                "budget": budget.name,
                "action": threshold.action,
                "threshold": threshold.at,
                "amount_usd": budget.amount_usd,
                "used_usd": used,
                "pct": pct,
            });
            let _ = self
                .http
                .post(url)
                .json(&payload)
                .send()
                .await
                .map_err(|e| tracing::warn!(error = %e, "budget webhook failed"));
        }
    }

    pub async fn current_usage(&self, project_id: Option<&str>) -> Vec<BudgetUsage> {
        let mut out = Vec::new();
        for b in self.config.load().budgets.iter() {
            if let Some(p) = project_id {
                if let Some(tp) = &b.target.project_id {
                    if tp != p {
                        continue;
                    }
                }
            }
            let period_start = period_start_for(&b.period);
            let used = self
                .counter
                .read_budget(&b.name, period_start)
                .await
                .unwrap_or(0.0);
            let pct = if b.amount_usd > 0.0 { used / b.amount_usd } else { 0.0 };
            let blocked = b
                .thresholds
                .iter()
                .any(|t| t.action == "block" && pct >= t.at);
            out.push(BudgetUsage {
                budget_id: b.name.clone(),
                name: b.name.clone(),
                period: b.period.clone(),
                period_start,
                amount_usd: b.amount_usd,
                used_usd: used,
                pct,
                blocked,
            });
        }
        out
    }
}

/// Compute the unix-ms timestamp at the start of the current `period`
/// (UTC). Unknown periods default to monthly so the budget never
/// "resets" unexpectedly.
pub fn period_start_for(period: &str) -> i64 {
    let now = Utc::now();
    let dt = match period {
        "daily" => Utc
            .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
            .unwrap(),
        "weekly" => {
            let monday = now.date_naive() - chrono::Duration::days(now.weekday().num_days_from_monday() as i64);
            Utc.with_ymd_and_hms(monday.year(), monday.month(), monday.day(), 0, 0, 0)
                .unwrap()
        }
        _ => Utc
            .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
            .unwrap(),
    };
    dt.timestamp_millis()
}
