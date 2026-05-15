use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::Response;
use bytes::{Bytes, BytesMut};
use futures::stream::{self, StreamExt};
use http_body_util::BodyExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use gateway_core::cache::{
    fingerprint, CacheDirective, CachePolicy, CacheStatus, CachedChunk, CachedResponse,
    FingerprintInputs,
};
use gateway_core::config::{AppConfig, RouteConfig};
use gateway_core::proxy::{execute_chain, ForwardRequest, ProviderChain, ResponseBody};
use gateway_core::GatewayError;

use crate::auth::GatewayKeyPrincipal;
use crate::error::ApiError;
use crate::logging::LogBuilder;
use crate::ratelimit::{check_limits, RatePermit};
use crate::state::AppState;
use crate::tokens::extract_token_usage;

const MAX_LOG_BODY_BYTES: usize = 64 * 1024;
const PROXY_BODY_CHANNEL: usize = 32;
const MAX_CACHEABLE_BODY_BYTES: usize = 2 * 1024 * 1024; // 2 MB cap on cache writes

pub async fn proxy_handler(
    State(state): State<AppState>,
    principal: GatewayKeyPrincipal,
    Path((provider, tail)): Path<(String, String)>,
    headers: HeaderMap,
    method: Method,
    body: Body,
) -> Result<Response, ApiError> {
    let started = Instant::now();
    let config = state.config_snapshot();

    let body_bytes = read_body(body).await?;
    let request_body_for_log = clip_body_for_log(&body_bytes);

    let upstream_path = format!("/{}", tail.trim_start_matches('/'));

    let mut log = LogBuilder::new(
        principal.project_id.clone(),
        Some(provider.clone()),
        Some(upstream_path.clone()),
    );
    log.set_gateway_key(Some(principal.key_id.clone()));
    log.set_request_body(request_body_for_log);
    log.set_model(extract_model(&body_bytes));
    let client_ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    log.set_client(None, client_ua);

    // Budget block check before doing any upstream work.
    if state
        .budgets
        .check_block(&principal.project_id, &principal.key_id)
        .await
    {
        log.set_status("gateway_error", Some(429));
        log.set_timing(started.elapsed().as_millis() as i64, 0, 0);
        log.set_error("budget_exceeded".into());
        log.submit(&state.stores.logs).await;
        return Err(ApiError::Gateway(gateway_core::GatewayError::RateLimited));
    }

    // Rate limit before doing any upstream work.
    let permit = match check_limits(
        state.stores.counter.clone(),
        &config.limits,
        &principal.key_id,
        &principal.project_id,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            log.set_status("rate_limited", Some(429));
            log.set_timing(started.elapsed().as_millis() as i64, 0, 0);
            log.submit(&state.stores.logs).await;
            return Err(e);
        }
    };

    // Resolve the route for this provider. The chain falls back to a
    // primary-only chain when no explicit route is defined.
    let route = find_route(&config, &provider);
    let chain = match &route {
        Some(r) => ProviderChain::from_route(r),
        None => ProviderChain::primary_only(&provider),
    };

    // Parse caching policy from route + request headers.
    let cache_policy = route.and_then(|r| {
        CachePolicy::from_headers(&headers, r.cache.enabled, Duration::from_secs(r.cache.ttl))
    });
    let body_is_cacheable_shape = cache_policy
        .as_ref()
        .map(|p| p.body_is_cacheable(&body_bytes))
        .unwrap_or(false);

    // Compute fingerprint up front so we can use it for both lookup and
    // writeback.
    let fp = fingerprint(&FingerprintInputs {
        provider: &provider,
        endpoint: &upstream_path,
        body: &body_bytes,
        namespace: cache_policy.as_ref().and_then(|p| p.namespace.as_deref()),
    });
    let cache_key = format!("resp:{provider}:{fp}");

    // Cache lookup (unless bypassed/refresh).
    let (cache_status, lookup_result) = match cache_policy.as_ref() {
        Some(p) if matches!(p.directive, CacheDirective::Bypass) => (CacheStatus::Bypass, None),
        Some(p) if matches!(p.directive, CacheDirective::Refresh) => (CacheStatus::Refresh, None),
        Some(_) => match state.stores.kv.get(&cache_key).await {
            Ok(Some(blob)) => match serde_json::from_slice::<CachedResponse>(&blob) {
                Ok(cached) => (CacheStatus::Hit, Some(cached)),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to deserialize cached response");
                    (CacheStatus::Miss, None)
                }
            },
            Ok(None) => (CacheStatus::Miss, None),
            Err(e) => {
                tracing::warn!(error = %e, "cache lookup failed");
                (CacheStatus::Miss, None)
            }
        },
        None => (CacheStatus::Disabled, None),
    };

    if let Some(cached) = lookup_result {
        log.set_status("success", Some(cached.status as i32));
        log.set_timing(started.elapsed().as_millis() as i64, 0, 0);
        log.set_response_body(cached_body_preview(&cached));
        log.set_cached(true);
        // Replay-time savings estimate: re-parse usage from the cached
        // body so the dashboard can show "savings vs upstream".
        let mut tmp = BytesMut::new();
        for c in &cached.chunks {
            tmp.extend_from_slice(&c.data);
        }
        let usage = extract_token_usage(&tmp);
        log.set_token_usage(
            usage.prompt,
            usage.completion,
            usage.cached,
            usage.total_tokens(),
        );
        let mut would_have = None;
        if let Some(model) = log.model().map(str::to_string) {
            if let Some(bd) = gateway_core::pricing::compute_cost(
                &state.pricing,
                &provider,
                &model,
                gateway_core::pricing::TokenUsage {
                    prompt: usage.prompt.unwrap_or(0),
                    completion: usage.completion.unwrap_or(0),
                    cached: usage.cached.unwrap_or(0),
                },
            ) {
                would_have = Some(bd.cost_usd);
            }
        }
        log.set_cost(Some(0.0), would_have);
        log.submit(&state.stores.logs).await;
        metrics::counter!("gateway_cache_response_hit_total").increment(1);
        return Ok(build_cached_response(cached, cache_status, &cache_key));
    }

    if matches!(
        cache_policy.as_ref().map(|p| p.directive),
        Some(CacheDirective::OnlyIfCached)
    ) {
        log.set_status("gateway_error", Some(504));
        log.set_timing(started.elapsed().as_millis() as i64, 0, 0);
        log.set_error("cache miss with X-Gateway-Cache: only".into());
        log.submit(&state.stores.logs).await;
        return Err(ApiError::Gateway(GatewayError::UpstreamTimeout));
    }

    let template = ForwardRequest {
        provider: provider.clone(),
        path: upstream_path,
        query: None,
        method,
        headers,
        body: if body_bytes.is_empty() {
            None
        } else {
            Some(body_bytes)
        },
    };

    let forward_started = Instant::now();
    let result = execute_chain(&state.proxy, &chain, template).await;
    let forward_ms = forward_started.elapsed().as_millis() as i64;

    let attempts_value = serde_json::to_value(&result.attempts).unwrap_or(serde_json::Value::Null);
    log.merge_metadata("attempts", attempts_value);
    log.set_retry(result.attempts.len().saturating_sub(1) as i32);
    if let Some(fb) = result.fallback_used.clone() {
        log.set_fallback(Some(fb));
    }

    let response = match result.response {
        Ok(r) => r,
        Err(e) => {
            log.set_status(e.classification(), Some(e.status_code() as i32));
            log.set_timing(started.elapsed().as_millis() as i64, forward_ms, 0);
            log.set_error(e.to_string());
            log.submit(&state.stores.logs).await;
            return Err(ApiError::Gateway(e));
        }
    };

    let upstream_ms = response.upstream_ms as i64;
    let ttfb_ms = response.ttfb_ms as i64;
    let status = response.status;
    let headers_out = response.headers.clone();
    let ResponseBody::Stream(mut upstream_stream) = response.body;

    let (tx, rx) = mpsc::channel::<std::io::Result<Bytes>>(PROXY_BODY_CHANNEL);

    let log_store = state.stores.logs.clone();
    let kv = state.stores.kv.clone();
    let pricing = state.pricing.clone();
    let budgets = state.budgets.clone();
    let project_id = principal.project_id.clone();
    let key_id = principal.key_id.clone();
    let provider_name = provider.clone();
    let mut response_capture = BytesMut::new();
    let mut response_truncated = false;
    let mut log_holder = Some(log);
    let cache_should_write = matches!(cache_status, CacheStatus::Miss | CacheStatus::Refresh)
        && status.is_success()
        && body_is_cacheable_shape
        && cache_policy.is_some();
    let cache_ttl = cache_policy.as_ref().map(|p| p.ttl);
    let cache_key_for_write = cache_key.clone();
    let headers_for_cache = headers_out.clone();
    let permit_holder: Option<RatePermit> = Some(permit);

    let mut permit_holder = permit_holder;
    tokio::spawn(async move {
        let mut send_err = false;
        let mut cache_chunks: Vec<CachedChunk> = Vec::new();
        let mut cache_total = 0usize;
        let mut cache_oversize = false;

        while let Some(item) = upstream_stream.next().await {
            match item {
                Ok(chunk) => {
                    if response_capture.len() < MAX_LOG_BODY_BYTES {
                        let remaining = MAX_LOG_BODY_BYTES - response_capture.len();
                        let take = remaining.min(chunk.len());
                        response_capture.extend_from_slice(&chunk[..take]);
                        if take < chunk.len() {
                            response_truncated = true;
                        }
                    } else if !chunk.is_empty() {
                        response_truncated = true;
                    }

                    if cache_should_write && !cache_oversize {
                        cache_total = cache_total.saturating_add(chunk.len());
                        if cache_total > MAX_CACHEABLE_BODY_BYTES {
                            cache_oversize = true;
                            cache_chunks.clear();
                        } else {
                            cache_chunks.push(CachedChunk {
                                data: chunk.to_vec(),
                            });
                        }
                    }

                    if !send_err && tx.send(Ok(chunk)).await.is_err() {
                        send_err = true;
                    }
                }
                Err(e) => {
                    if !send_err {
                        let _ = tx.send(Err(e)).await;
                    }
                    if let Some(mut log) = log_holder.take() {
                        log.set_status("upstream_error", Some(status.as_u16() as i32));
                        log.set_timing(started.elapsed().as_millis() as i64, upstream_ms, ttfb_ms);
                        let body_text =
                            captured_body_to_string(&response_capture, response_truncated);
                        log.set_response_body(body_text);
                        log.submit(&log_store).await;
                    }
                    return;
                }
            }
        }

        // Cache write-back after stream finishes.
        if cache_should_write && !cache_oversize {
            let payload = CachedResponse {
                status: status.as_u16(),
                headers: headers_for_cache
                    .iter()
                    .filter_map(|(k, v)| {
                        Some((k.as_str().to_string(), v.to_str().ok()?.to_string()))
                    })
                    .collect(),
                chunks: cache_chunks,
                finished_at_ms: chrono::Utc::now().timestamp_millis(),
            };
            if let Ok(blob) = serde_json::to_vec(&payload) {
                if let Some(ttl) = cache_ttl {
                    if let Err(e) = kv.put(&cache_key_for_write, Bytes::from(blob), ttl).await {
                        tracing::warn!(error = %e, "failed to write cache");
                    } else {
                        metrics::counter!("gateway_cache_response_write_total").increment(1);
                    }
                }
            }
        }

        // Extract token usage from the captured body (best-effort).
        let usage = extract_token_usage(&response_capture);
        if let Some(p) = permit_holder.take() {
            let actual = usage.total_tokens().unwrap_or(0);
            p.reconcile_tokens(actual).await;
        }

        // Pricing + budget bookkeeping.
        let mut cost_value: Option<f64> = None;
        if status.is_success() {
            let model_for_pricing = log_holder
                .as_ref()
                .and_then(|l| l.model().map(str::to_string));
            if let Some(model) = model_for_pricing {
                let pricing_usage = gateway_core::pricing::TokenUsage {
                    prompt: usage.prompt.unwrap_or(0),
                    completion: usage.completion.unwrap_or(0),
                    cached: usage.cached.unwrap_or(0),
                };
                if let Some(bd) = gateway_core::pricing::compute_cost(
                    &pricing,
                    &provider_name,
                    &model,
                    pricing_usage,
                ) {
                    cost_value = Some(bd.cost_usd);
                    budgets.record_cost(&project_id, &key_id, bd.cost_usd).await;
                    metrics::counter!(
                        "gateway_cost_total_usd_micro",
                        "provider" => provider_name.clone(),
                        "model" => model.clone()
                    )
                    .increment((bd.cost_usd * 1_000_000.0) as u64);
                }
            }
        }

        if let Some(mut log) = log_holder.take() {
            let status_class = if status.is_success() {
                "success"
            } else {
                "upstream_error"
            };
            log.set_status(status_class, Some(status.as_u16() as i32));
            log.set_timing(started.elapsed().as_millis() as i64, upstream_ms, ttfb_ms);
            log.set_token_usage(
                usage.prompt,
                usage.completion,
                usage.cached,
                usage.total_tokens(),
            );
            log.set_cost(cost_value, cost_value);
            let body_text = captured_body_to_string(&response_capture, response_truncated);
            log.set_response_body(body_text);
            log.submit(&log_store).await;
        }
    });

    let stream = ReceiverStream::new(rx);
    let body = Body::from_stream(stream);

    let mut builder = Response::builder().status(status);
    for (k, v) in headers_out.iter() {
        builder = builder.header(k, v);
    }
    let mut response = builder
        .body(body)
        .map_err(|e| ApiError::Gateway(GatewayError::Internal(e.to_string())))?;

    insert_cache_headers(response.headers_mut(), cache_status, &cache_key, None);
    if let Ok(hv) = HeaderValue::from_str(&uuid::Uuid::now_v7().to_string()) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("x-gateway-request-id"), hv);
    }
    Ok(response)
}

fn find_route<'a>(config: &'a AppConfig, provider: &str) -> Option<&'a RouteConfig> {
    config
        .routes
        .iter()
        .find(|r| r.primary.provider == provider)
}

fn build_cached_response(
    cached: CachedResponse,
    cache_status: CacheStatus,
    cache_key: &str,
) -> Response {
    let chunks: Vec<std::io::Result<Bytes>> = cached
        .chunks
        .into_iter()
        .map(|c| Ok(Bytes::from(c.data)))
        .collect();
    let body = Body::from_stream(stream::iter(chunks));
    let mut builder =
        Response::builder().status(StatusCode::from_u16(cached.status).unwrap_or(StatusCode::OK));
    for (k, v) in &cached.headers {
        if k.eq_ignore_ascii_case("content-length") || k.eq_ignore_ascii_case("transfer-encoding") {
            continue;
        }
        builder = builder.header(k, v);
    }
    let mut response = builder
        .body(body)
        .unwrap_or_else(|_| Response::new(Body::empty()));
    let age = chrono::Utc::now().timestamp_millis() - cached.finished_at_ms;
    insert_cache_headers(
        response.headers_mut(),
        cache_status,
        cache_key,
        Some(age.max(0) as u64 / 1000),
    );
    response
}

fn insert_cache_headers(
    headers: &mut HeaderMap,
    status: CacheStatus,
    key: &str,
    age_s: Option<u64>,
) {
    if let Ok(v) = HeaderValue::from_str(status.as_header()) {
        headers.insert(HeaderName::from_static("x-gateway-cache-status"), v);
    }
    if let Ok(v) = HeaderValue::from_str(&key[..key.len().min(48)]) {
        headers.insert(HeaderName::from_static("x-gateway-cache-key"), v);
    }
    if let Some(age) = age_s {
        if let Ok(v) = HeaderValue::from_str(&age.to_string()) {
            headers.insert(HeaderName::from_static("x-gateway-cache-age"), v);
        }
    }
}

fn cached_body_preview(cached: &CachedResponse) -> Option<String> {
    let mut out = String::new();
    for c in &cached.chunks {
        if out.len() >= MAX_LOG_BODY_BYTES {
            out.push_str("\n…[truncated]");
            break;
        }
        if let Ok(s) = std::str::from_utf8(&c.data) {
            let remaining = MAX_LOG_BODY_BYTES - out.len();
            out.push_str(&s[..s.len().min(remaining)]);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

async fn read_body(body: Body) -> Result<Bytes, ApiError> {
    let collected = body
        .collect()
        .await
        .map_err(|e| ApiError::BadRequest(format!("failed to read body: {e}")))?;
    Ok(collected.to_bytes())
}

fn clip_body_for_log(body: &Bytes) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    let take = body.len().min(MAX_LOG_BODY_BYTES);
    match std::str::from_utf8(&body[..take]) {
        Ok(s) => {
            if take < body.len() {
                Some(format!("{s}\n…[truncated]"))
            } else {
                Some(s.to_string())
            }
        }
        Err(_) => Some(format!("[binary {} bytes]", body.len())),
    }
}

fn extract_model(body: &Bytes) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.get("model")
        .and_then(|m| m.as_str())
        .map(|s| s.to_string())
}

fn captured_body_to_string(buf: &BytesMut, truncated: bool) -> Option<String> {
    if buf.is_empty() {
        return None;
    }
    let s = match std::str::from_utf8(buf) {
        Ok(s) => s.to_string(),
        Err(_) => format!("[binary {} bytes]", buf.len()),
    };
    if truncated {
        Some(format!("{s}\n…[truncated]"))
    } else {
        Some(s)
    }
}
