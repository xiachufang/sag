use crate::pricing::catalog::{PricingCatalog, PricingEntry};

#[derive(Debug, Clone, Copy, Default)]
pub struct TokenUsage {
    pub prompt: i64,
    pub completion: i64,
    pub cached: i64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CostBreakdown {
    /// Actual amount billed for this request. `0` for cache hits served
    /// entirely by the gateway.
    pub cost_usd: f64,
    /// What the user would have paid without the gateway's cache. Equal
    /// to `cost_usd` for non-cached responses, and `>0` for cache hits.
    pub would_have_cost_usd: f64,
}

/// Apply pricing to a usage record. Returns `None` when no matching
/// pricing row is found — the caller should record `cost_usd = NULL` so
/// downstream aggregation skips this row instead of guessing.
pub fn compute_cost(
    catalog: &PricingCatalog,
    provider: &str,
    model: &str,
    usage: TokenUsage,
) -> Option<CostBreakdown> {
    let entry = catalog.lookup(provider, model)?;
    Some(apply(entry, usage))
}

fn apply(entry: &PricingEntry, u: TokenUsage) -> CostBreakdown {
    let cached_input_rate = entry.cached_input_per_1k.unwrap_or(entry.input_per_1k);
    let billable_prompt = (u.prompt - u.cached).max(0) as f64;
    let cached_prompt = u.cached.max(0) as f64;
    let completion = u.completion.max(0) as f64;

    let cost = billable_prompt * entry.input_per_1k / 1000.0
        + cached_prompt * cached_input_rate / 1000.0
        + completion * entry.output_per_1k / 1000.0;
    CostBreakdown {
        cost_usd: round6(cost),
        would_have_cost_usd: round6(cost),
    }
}

fn round6(v: f64) -> f64 {
    (v * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_for_gpt_4o_mini() {
        let cat = PricingCatalog::embedded();
        let bd = compute_cost(
            &cat,
            "openai",
            "gpt-4o-mini",
            TokenUsage {
                prompt: 1000,
                completion: 500,
                cached: 0,
            },
        )
        .unwrap();
        // 1000 input * 0.00015/1k + 500 output * 0.0006/1k
        let expected = 0.00015 + 0.0003;
        assert!((bd.cost_usd - expected).abs() < 1e-9);
    }

    #[test]
    fn cached_input_rate_applied() {
        let cat = PricingCatalog::embedded();
        let bd = compute_cost(
            &cat,
            "anthropic",
            "claude-sonnet-4-6",
            TokenUsage {
                prompt: 1000,
                completion: 0,
                cached: 800,
            },
        )
        .unwrap();
        // 200 fresh prompt * 0.003/1k + 800 cached * 0.0003/1k
        let expected = 200.0 * 0.003 / 1000.0 + 800.0 * 0.0003 / 1000.0;
        assert!((bd.cost_usd - expected).abs() < 1e-9);
    }
}
