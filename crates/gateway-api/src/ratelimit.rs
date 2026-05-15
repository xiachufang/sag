use std::sync::Arc;

use gateway_core::config::LimitConfig;
use gateway_storage::traits::{ConcurrencyPermit, CounterStore};

use crate::error::ApiError;

const WINDOW_MS: u64 = 60_000;
/// Conservative estimate of tokens charged up-front when the route has a
/// TPM cap and the caller didn't supply `max_tokens`. Reconciled after
/// the request completes.
pub const DEFAULT_TOKEN_PREDEDUCT: i64 = 1024;

/// Held for the lifetime of a request to release the concurrency slot
/// on drop and reconcile the pre-deducted TPM count.
pub struct RatePermit {
    _concurrency: Vec<ConcurrencyPermit>,
    /// (counter_store, tpm_key, pre-deducted amount). On reconciliation
    /// we delta-add `(actual - prededucted)`.
    pub tpm_reservations: Vec<(Arc<dyn CounterStore>, String, i64)>,
}

impl RatePermit {
    pub async fn reconcile_tokens(self, actual_tokens: i64) {
        for (store, key, prededucted) in self.tpm_reservations {
            let delta = actual_tokens - prededucted;
            if delta == 0 {
                continue;
            }
            let _ = store.incr_window(&key, WINDOW_MS, delta).await;
        }
    }
}

/// Apply every configured limit to the request. Returns a permit that
/// must outlive the request (it releases concurrency on drop).
pub async fn check_limits(
    counter: Arc<dyn CounterStore>,
    limits: &[LimitConfig],
    key_id: &str,
    project_id: &str,
) -> Result<RatePermit, ApiError> {
    let mut concurrency_permits = Vec::new();
    let mut tpm_reservations = Vec::new();

    for limit in limits {
        let scope = match limit.target.kind.as_str() {
            "key" => {
                let pattern = limit.target.id.as_deref().unwrap_or("*");
                if pattern != "*" && pattern != key_id {
                    continue;
                }
                format!("key:{key_id}")
            }
            "project" => {
                let pattern = limit.target.id.as_deref().unwrap_or("*");
                if pattern != "*" && pattern != project_id {
                    continue;
                }
                format!("project:{project_id}")
            }
            "global" => "global".to_string(),
            _ => continue,
        };

        if let Some(rpm) = limit.rpm {
            let count = counter
                .incr_window(&format!("rpm:{scope}"), WINDOW_MS, 1)
                .await
                .map_err(ApiError::Storage)?;
            if count as u64 > rpm {
                metrics::counter!("gateway_ratelimit_hit_total", "kind" => "rpm").increment(1);
                return Err(ApiError::Gateway(gateway_core::GatewayError::RateLimited));
            }
        }

        if let Some(tpm) = limit.tpm {
            let tpm_key = format!("tpm:{scope}");
            let count = counter
                .incr_window(&tpm_key, WINDOW_MS, DEFAULT_TOKEN_PREDEDUCT)
                .await
                .map_err(ApiError::Storage)?;
            if count as u64 > tpm {
                // roll back the prededuct so we don't lock the bucket
                let _ = counter
                    .incr_window(&tpm_key, WINDOW_MS, -DEFAULT_TOKEN_PREDEDUCT)
                    .await;
                metrics::counter!("gateway_ratelimit_hit_total", "kind" => "tpm").increment(1);
                return Err(ApiError::Gateway(gateway_core::GatewayError::RateLimited));
            }
            tpm_reservations.push((counter.clone(), tpm_key, DEFAULT_TOKEN_PREDEDUCT));
        }

        if let Some(conc) = limit.concurrency {
            let permit = counter
                .try_acquire_concurrency(&format!("conc:{scope}"), conc)
                .await
                .map_err(ApiError::Storage)?;
            match permit {
                Some(p) => concurrency_permits.push(p),
                None => {
                    metrics::counter!("gateway_ratelimit_hit_total", "kind" => "concurrency")
                        .increment(1);
                    return Err(ApiError::Gateway(gateway_core::GatewayError::RateLimited));
                }
            }
        }
    }

    Ok(RatePermit {
        _concurrency: concurrency_permits,
        tpm_reservations,
    })
}
