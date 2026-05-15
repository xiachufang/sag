use chrono::Utc;
use gateway_storage::models::RequestLogRecord;
use gateway_storage::traits::LogStore;
use std::sync::Arc;
use uuid::Uuid;

/// Helper used by the proxy handler to construct + append a request log
/// record. Centralized so future milestones can add cost/token fields in
/// one place.
pub struct LogBuilder {
    rec: RequestLogRecord,
}

impl LogBuilder {
    pub fn new(project_id: String, namespace: Option<String>, endpoint: Option<String>) -> Self {
        Self {
            rec: RequestLogRecord {
                id: Uuid::now_v7().to_string(),
                project_id,
                gateway_key_id: None,
                namespace,
                model: None,
                endpoint,
                request_ts: Utc::now().timestamp_millis(),
                duration_ms: None,
                upstream_ms: None,
                ttfb_ms: None,
                status: "success".into(),
                http_status: None,
                cached: false,
                retry_count: 0,
                fallback_used: None,
                prompt_tokens: None,
                completion_tokens: None,
                cached_tokens: None,
                total_tokens: None,
                cost_usd: None,
                would_have_cost_usd: None,
                metadata: None,
                client_ip: None,
                user_agent: None,
                error_message: None,
                request_body: None,
                response_body: None,
            },
        }
    }

    pub fn id(&self) -> &str {
        &self.rec.id
    }

    pub fn set_model(&mut self, model: Option<String>) {
        self.rec.model = model;
    }

    pub fn set_gateway_key(&mut self, key_id: Option<String>) {
        self.rec.gateway_key_id = key_id;
    }

    pub fn set_status(&mut self, status: &str, http_status: Option<i32>) {
        self.rec.status = status.into();
        self.rec.http_status = http_status;
    }

    pub fn set_timing(&mut self, duration_ms: i64, upstream_ms: i64, ttfb_ms: i64) {
        self.rec.duration_ms = Some(duration_ms);
        self.rec.upstream_ms = Some(upstream_ms);
        self.rec.ttfb_ms = Some(ttfb_ms);
    }

    pub fn set_client(&mut self, ip: Option<String>, user_agent: Option<String>) {
        self.rec.client_ip = ip;
        self.rec.user_agent = user_agent;
    }

    pub fn set_request_body(&mut self, body: Option<String>) {
        self.rec.request_body = body;
    }

    pub fn set_response_body(&mut self, body: Option<String>) {
        self.rec.response_body = body;
    }

    pub fn set_error(&mut self, msg: String) {
        self.rec.error_message = Some(msg);
    }

    pub fn set_cached(&mut self, cached: bool) {
        self.rec.cached = cached;
    }

    pub fn set_retry(&mut self, retry_count: i32) {
        self.rec.retry_count = retry_count;
    }

    pub fn set_fallback(&mut self, provider: Option<String>) {
        self.rec.fallback_used = provider;
    }

    pub fn set_token_usage(
        &mut self,
        prompt: Option<i64>,
        completion: Option<i64>,
        cached: Option<i64>,
        total: Option<i64>,
    ) {
        self.rec.prompt_tokens = prompt;
        self.rec.completion_tokens = completion;
        self.rec.cached_tokens = cached;
        self.rec.total_tokens = total;
    }

    pub fn set_cost(&mut self, cost_usd: Option<f64>, would_have_cost_usd: Option<f64>) {
        self.rec.cost_usd = cost_usd;
        self.rec.would_have_cost_usd = would_have_cost_usd;
    }

    pub fn model(&self) -> Option<&str> {
        self.rec.model.as_deref()
    }

    pub fn merge_metadata(&mut self, key: &str, value: serde_json::Value) {
        let entry = self
            .rec
            .metadata
            .get_or_insert_with(|| serde_json::json!({}));
        if let serde_json::Value::Object(map) = entry {
            map.insert(key.to_string(), value);
        } else {
            *entry = serde_json::json!({ key: value });
        }
    }

    pub async fn submit(self, store: &Arc<dyn LogStore>) {
        if let Err(e) = store.append(self.rec).await {
            tracing::warn!(error = %e, "failed to append request log");
        }
    }
}
