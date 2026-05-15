use std::time::Instant;

use crate::error::{GatewayError, Result};
use crate::proxy::engine::{ForwardRequest, ForwardResponse, ProxyEngine};
use crate::proxy::fallback::{AttemptTrace, ProviderChain};
use crate::proxy::retry::{backoff_duration, matches_fallback_trigger, AttemptOutcome};

/// Outcome of running a [`ProviderChain`]. The body stream is fully owned
/// by the response and not yet consumed — the API layer pipes it onward.
pub struct ChainResult {
    pub response: Result<ForwardResponse>,
    pub attempts: Vec<AttemptTrace>,
    pub fallback_used: Option<String>,
}

/// Try each entry of `chain` in order, applying per-entry retry. Falls
/// back to the next entry only when its trigger matches the previous
/// entry's terminal outcome.
pub async fn execute_chain(
    engine: &ProxyEngine,
    chain: &ProviderChain,
    template: ForwardRequest,
) -> ChainResult {
    let mut attempts: Vec<AttemptTrace> = Vec::new();
    let mut last_outcome: Option<AttemptOutcome> = None;
    let mut last_error: Option<GatewayError> = None;
    let mut last_response: Option<ForwardResponse> = None;
    let mut fallback_used: Option<String> = None;

    for (idx, entry) in chain.entries.iter().enumerate() {
        // For non-primary entries, only proceed if previous outcome
        // matches this entry's trigger.
        if idx > 0 {
            let prev = match last_outcome {
                Some(o) => o,
                None => break,
            };
            if !matches_fallback_trigger(prev, &entry.trigger) {
                break;
            }
            fallback_used = Some(entry.provider.clone());
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut req = template.clone();
            req.provider = entry.provider.clone();

            let start = Instant::now();
            let result = engine.forward(req).await;
            let duration_ms = start.elapsed().as_millis().min(u64::MAX as u128) as u64;

            let outcome = match &result {
                Ok(r) => AttemptOutcome::from_status(r.status.as_u16()),
                Err(e) => AttemptOutcome::from_error(e),
            };
            attempts.push(AttemptTrace {
                provider: entry.provider.clone(),
                model: entry.model_override.clone(),
                status: result.as_ref().ok().map(|r| r.status.as_u16()),
                outcome: outcome_label(outcome),
                duration_ms,
            });

            match outcome {
                AttemptOutcome::Success | AttemptOutcome::NonRetryable(_) => {
                    return ChainResult {
                        response: result,
                        attempts,
                        fallback_used,
                    };
                }
                _ => {
                    if attempt >= chain.retry.max_attempts {
                        last_outcome = Some(outcome);
                        match result {
                            Ok(resp) => last_response = Some(resp),
                            Err(e) => last_error = Some(e),
                        }
                        break;
                    }
                    let backoff = backoff_duration(&chain.retry, attempt - 1);
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }

    let response = if let Some(resp) = last_response {
        Ok(resp)
    } else if let Some(err) = last_error {
        Err(err)
    } else {
        Err(GatewayError::Internal("empty provider chain".into()))
    };

    ChainResult {
        response,
        attempts,
        fallback_used,
    }
}

fn outcome_label(o: AttemptOutcome) -> &'static str {
    match o {
        AttemptOutcome::Success => "success",
        AttemptOutcome::RetryableStatus(_) => "retryable_status",
        AttemptOutcome::RetryableNetwork => "retryable_network",
        AttemptOutcome::NonRetryable(_) => "non_retryable",
        AttemptOutcome::Timeout => "timeout",
    }
}
