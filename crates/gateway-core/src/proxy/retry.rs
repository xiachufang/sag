use std::time::Duration;

use crate::config::RouteRetryConfig;
use crate::error::GatewayError;

/// Outcome class used by both retry and fallback logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptOutcome {
    Success,
    RetryableStatus(u16),
    RetryableNetwork,
    NonRetryable(u16),
    Timeout,
}

impl AttemptOutcome {
    pub fn from_status(status: u16) -> Self {
        if is_retryable_status(status) {
            AttemptOutcome::RetryableStatus(status)
        } else if (200..400).contains(&status) {
            AttemptOutcome::Success
        } else {
            AttemptOutcome::NonRetryable(status)
        }
    }

    pub fn from_error(err: &GatewayError) -> Self {
        match err {
            GatewayError::UpstreamTimeout => AttemptOutcome::Timeout,
            GatewayError::Http(_) => AttemptOutcome::RetryableNetwork,
            _ => AttemptOutcome::NonRetryable(err.status_code()),
        }
    }

    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            AttemptOutcome::RetryableStatus(_)
                | AttemptOutcome::RetryableNetwork
                | AttemptOutcome::Timeout
        )
    }
}

pub fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

/// Compute exponential backoff with full jitter. Capped at 10s.
pub fn backoff_duration(cfg: &RouteRetryConfig, attempt: u32) -> Duration {
    let base = cfg
        .initial_backoff_ms
        .saturating_mul(1u64 << attempt.min(8));
    let capped = base.min(10_000);
    let jitter = rand::random::<f64>() * capped as f64;
    Duration::from_millis(jitter as u64)
}

/// Determine whether the given attempt outcome maps onto one of the
/// fallback triggers configured for the *next* target. M3 supports a
/// small vocabulary (`upstream_5xx`, `timeout`, `rate_limited`); any
/// missing/empty trigger list defaults to "always".
pub fn matches_fallback_trigger(outcome: AttemptOutcome, triggers: &[String]) -> bool {
    if triggers.is_empty() {
        return matches!(
            outcome,
            AttemptOutcome::RetryableStatus(_)
                | AttemptOutcome::RetryableNetwork
                | AttemptOutcome::Timeout
                | AttemptOutcome::NonRetryable(_)
        );
    }
    triggers.iter().any(|t| {
        let t = t.to_ascii_lowercase();
        match outcome {
            AttemptOutcome::RetryableStatus(s) if (500..600).contains(&s) => {
                matches!(t.as_str(), "upstream_5xx" | "upstream_error")
            }
            AttemptOutcome::NonRetryable(s) if (500..600).contains(&s) => {
                matches!(t.as_str(), "upstream_5xx" | "upstream_error")
            }
            AttemptOutcome::RetryableStatus(429) => {
                matches!(t.as_str(), "rate_limited" | "ratelimited")
            }
            AttemptOutcome::Timeout => matches!(t.as_str(), "timeout"),
            AttemptOutcome::RetryableNetwork => matches!(t.as_str(), "network" | "timeout"),
            _ => false,
        }
    })
}
