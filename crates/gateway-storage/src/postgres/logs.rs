use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sqlx::{PgPool, Postgres, QueryBuilder, Row};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::error::{Result, StorageError};
use crate::models::*;
use crate::traits::LogStore;

const BATCH_MAX_ROWS: usize = 500;
const BATCH_FLUSH_INTERVAL: Duration = Duration::from_millis(100);
const CHANNEL_CAPACITY: usize = 8192;

enum Cmd {
    Record(Box<RequestLogRecord>),
    Flush(oneshot::Sender<()>),
}

pub struct PostgresLogStore {
    pool: PgPool,
    tx: mpsc::Sender<Cmd>,
    _flush_lock: Arc<Mutex<()>>, // unused but kept to mirror SQLite layout
}

impl PostgresLogStore {
    pub fn new(pool: PgPool) -> Self {
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let worker_pool = pool.clone();
        tokio::spawn(async move {
            batch_worker(worker_pool, rx).await;
        });
        Self {
            pool,
            tx,
            _flush_lock: Arc::new(Mutex::new(())),
        }
    }
}

#[async_trait]
impl LogStore for PostgresLogStore {
    async fn append(&self, rec: RequestLogRecord) -> Result<()> {
        match self.tx.try_send(Cmd::Record(Box::new(rec))) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                metrics::counter!("gateway_log_drop_total", "reason" => "full").increment(1);
                Ok(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(StorageError::Unavailable("log worker stopped".into()))
            }
        }
    }

    async fn query(&self, q: LogQuery) -> Result<Page<RequestLogRow>> {
        let limit = if q.limit == 0 { 50 } else { q.limit.min(500) };
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
            r#"
            SELECT id, project_id, gateway_key_id, provider, model, endpoint,
                   request_ts, duration_ms, status, http_status, cached, retry_count,
                   prompt_tokens, completion_tokens, cost_usd
              FROM request_logs WHERE 1=1
            "#,
        );
        if let Some(v) = &q.project_id {
            qb.push(" AND project_id = ").push_bind(v.clone());
        }
        if let Some(v) = &q.gateway_key_id {
            qb.push(" AND gateway_key_id = ").push_bind(v.clone());
        }
        if let Some(v) = &q.provider {
            qb.push(" AND provider = ").push_bind(v.clone());
        }
        if let Some(v) = &q.model {
            qb.push(" AND model = ").push_bind(v.clone());
        }
        if let Some(v) = &q.status {
            qb.push(" AND status = ").push_bind(v.clone());
        }
        if let Some(v) = q.from_ts {
            qb.push(" AND request_ts >= ").push_bind(v);
        }
        if let Some(v) = q.to_ts {
            qb.push(" AND request_ts <= ").push_bind(v);
        }
        qb.push(" ORDER BY request_ts DESC, id DESC LIMIT ")
            .push_bind((limit + 1) as i64);

        let rows = qb.build().fetch_all(&self.pool).await?;
        let has_more = rows.len() > limit as usize;
        let items: Vec<RequestLogRow> = rows
            .into_iter()
            .take(limit as usize)
            .map(|r| RequestLogRow {
                id: r.get("id"),
                project_id: r.get("project_id"),
                gateway_key_id: r.get("gateway_key_id"),
                provider: r.get("provider"),
                model: r.get("model"),
                endpoint: r.get("endpoint"),
                request_ts: r.get("request_ts"),
                duration_ms: r.get("duration_ms"),
                status: r.get("status"),
                http_status: r.get::<Option<i32>, _>("http_status"),
                cached: r.get::<bool, _>("cached"),
                retry_count: r.get::<i32, _>("retry_count"),
                prompt_tokens: r.get("prompt_tokens"),
                completion_tokens: r.get("completion_tokens"),
                cost_usd: r.get("cost_usd"),
            })
            .collect();
        let next_cursor = if has_more {
            items.last().map(|r| format!("{}:{}", r.request_ts, r.id))
        } else {
            None
        };
        Ok(Page { items, next_cursor })
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<RequestLogDetail>> {
        let row = sqlx::query("SELECT * FROM request_logs WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(r) = row else { return Ok(None) };
        let metadata: Option<String> = r.try_get("metadata").ok();
        let metadata = match metadata {
            Some(s) if !s.is_empty() => Some(serde_json::from_str(&s)?),
            _ => None,
        };
        let record = RequestLogRecord {
            id: r.get("id"),
            project_id: r.get("project_id"),
            gateway_key_id: r.get("gateway_key_id"),
            provider: r.get("provider"),
            model: r.get("model"),
            endpoint: r.get("endpoint"),
            request_ts: r.get("request_ts"),
            duration_ms: r.get("duration_ms"),
            upstream_ms: r.get("upstream_ms"),
            ttfb_ms: r.get("ttfb_ms"),
            status: r.get("status"),
            http_status: r.get::<Option<i32>, _>("http_status"),
            cached: r.get::<bool, _>("cached"),
            retry_count: r.get::<i32, _>("retry_count"),
            fallback_used: r.get("fallback_used"),
            prompt_tokens: r.get("prompt_tokens"),
            completion_tokens: r.get("completion_tokens"),
            cached_tokens: r.get("cached_tokens"),
            total_tokens: r.get("total_tokens"),
            cost_usd: r.get("cost_usd"),
            would_have_cost_usd: r.get("would_have_cost_usd"),
            metadata,
            client_ip: r.get("client_ip"),
            user_agent: r.get("user_agent"),
            error_message: r.get("error_message"),
            request_body: r.get("request_body"),
            response_body: r.get("response_body"),
        };
        Ok(Some(RequestLogDetail { record }))
    }

    async fn aggregate(&self, q: AggregateQuery) -> Result<AggregateResult> {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("SELECT ");
        let mut select_exprs: Vec<&str> = Vec::new();
        let mut group_exprs: Vec<&str> = Vec::new();
        for dim in &q.group_by {
            let (s, g) = match dim {
                AggregateDimension::Provider => ("provider AS dim_provider", "provider"),
                AggregateDimension::Model => ("model AS dim_model", "model"),
                AggregateDimension::GatewayKey => {
                    ("gateway_key_id AS dim_gateway_key", "gateway_key_id")
                }
                AggregateDimension::Day => (
                    "to_char(to_timestamp(request_ts/1000), 'YYYY-MM-DD') AS dim_day",
                    "to_char(to_timestamp(request_ts/1000), 'YYYY-MM-DD')",
                ),
                AggregateDimension::Hour => (
                    "to_char(to_timestamp(request_ts/1000), 'YYYY-MM-DD\"T\"HH24') AS dim_hour",
                    "to_char(to_timestamp(request_ts/1000), 'YYYY-MM-DD\"T\"HH24')",
                ),
            };
            select_exprs.push(s);
            group_exprs.push(g);
        }
        if !select_exprs.is_empty() {
            qb.push(select_exprs.join(", "));
            qb.push(", ");
        }
        qb.push(
            "COUNT(*)::BIGINT AS requests, \
             COALESCE(SUM(prompt_tokens), 0)::BIGINT AS prompt_tokens, \
             COALESCE(SUM(completion_tokens), 0)::BIGINT AS completion_tokens, \
             COALESCE(SUM(cost_usd), 0)::DOUBLE PRECISION AS cost_usd, \
             COALESCE(SUM(would_have_cost_usd - cost_usd), 0)::DOUBLE PRECISION AS cached_savings \
             FROM request_logs WHERE 1=1",
        );
        if let Some(v) = &q.project_id {
            qb.push(" AND project_id = ").push_bind(v.clone());
        }
        if let Some(v) = q.from_ts {
            qb.push(" AND request_ts >= ").push_bind(v);
        }
        if let Some(v) = q.to_ts {
            qb.push(" AND request_ts <= ").push_bind(v);
        }
        if !group_exprs.is_empty() {
            qb.push(" GROUP BY ").push(group_exprs.join(", "));
            qb.push(" ORDER BY cost_usd DESC LIMIT 500");
        }

        let rows = qb.build().fetch_all(&self.pool).await?;
        let mut groups = Vec::with_capacity(rows.len());
        let mut total_cost = 0.0;
        for r in rows {
            let mut key = serde_json::Map::new();
            for dim in &q.group_by {
                match dim {
                    AggregateDimension::Provider => {
                        key.insert(
                            "provider".into(),
                            r.try_get::<Option<String>, _>("dim_provider")
                                .ok()
                                .flatten()
                                .map(serde_json::Value::String)
                                .unwrap_or(serde_json::Value::Null),
                        );
                    }
                    AggregateDimension::Model => {
                        key.insert(
                            "model".into(),
                            r.try_get::<Option<String>, _>("dim_model")
                                .ok()
                                .flatten()
                                .map(serde_json::Value::String)
                                .unwrap_or(serde_json::Value::Null),
                        );
                    }
                    AggregateDimension::GatewayKey => {
                        key.insert(
                            "gateway_key_id".into(),
                            r.try_get::<Option<String>, _>("dim_gateway_key")
                                .ok()
                                .flatten()
                                .map(serde_json::Value::String)
                                .unwrap_or(serde_json::Value::Null),
                        );
                    }
                    AggregateDimension::Day => {
                        key.insert(
                            "day".into(),
                            r.try_get::<String, _>("dim_day")
                                .map(serde_json::Value::String)
                                .unwrap_or(serde_json::Value::Null),
                        );
                    }
                    AggregateDimension::Hour => {
                        key.insert(
                            "hour".into(),
                            r.try_get::<String, _>("dim_hour")
                                .map(serde_json::Value::String)
                                .unwrap_or(serde_json::Value::Null),
                        );
                    }
                }
            }
            let cost: f64 = r.get("cost_usd");
            total_cost += cost;
            groups.push(AggregateGroup {
                key: serde_json::Value::Object(key),
                requests: r.get("requests"),
                prompt_tokens: r.get("prompt_tokens"),
                completion_tokens: r.get("completion_tokens"),
                cost_usd: cost,
                cached_savings_usd: r.get("cached_savings"),
            });
        }
        Ok(AggregateResult {
            total_cost_usd: total_cost,
            groups,
        })
    }

    async fn purge_older_than(&self, ts: Timestamp) -> Result<u64> {
        let res = sqlx::query("DELETE FROM request_logs WHERE request_ts < $1")
            .bind(ts)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }

    async fn flush(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Cmd::Flush(tx))
            .await
            .map_err(|_| StorageError::Unavailable("log worker stopped".into()))?;
        let _ = rx.await;
        Ok(())
    }
}

async fn batch_worker(pool: PgPool, mut rx: mpsc::Receiver<Cmd>) {
    let mut buf: Vec<RequestLogRecord> = Vec::with_capacity(BATCH_MAX_ROWS);
    let mut ticker = tokio::time::interval(BATCH_FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            biased;
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    Cmd::Record(r) => {
                        buf.push(*r);
                        if buf.len() >= BATCH_MAX_ROWS {
                            flush_batch(&pool, &mut buf).await;
                        }
                    }
                    Cmd::Flush(s) => {
                        if !buf.is_empty() { flush_batch(&pool, &mut buf).await; }
                        let _ = s.send(());
                    }
                }
            }
            _ = ticker.tick() => {
                if !buf.is_empty() { flush_batch(&pool, &mut buf).await; }
            }
        }
    }
    if !buf.is_empty() {
        flush_batch(&pool, &mut buf).await;
    }
}

async fn flush_batch(pool: &PgPool, buf: &mut Vec<RequestLogRecord>) {
    if buf.is_empty() {
        return;
    }
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
        r#"
        INSERT INTO request_logs
            (id, project_id, gateway_key_id, provider, model, endpoint,
             request_ts, duration_ms, upstream_ms, ttfb_ms,
             status, http_status, cached, retry_count, fallback_used,
             prompt_tokens, completion_tokens, cached_tokens, total_tokens,
             cost_usd, would_have_cost_usd, metadata,
             client_ip, user_agent, error_message,
             request_body, response_body)
        "#,
    );
    qb.push_values(buf.drain(..), |mut b, rec| {
        let metadata_str = rec
            .metadata
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "null".into()));
        b.push_bind(rec.id)
            .push_bind(rec.project_id)
            .push_bind(rec.gateway_key_id)
            .push_bind(rec.provider)
            .push_bind(rec.model)
            .push_bind(rec.endpoint)
            .push_bind(rec.request_ts)
            .push_bind(rec.duration_ms)
            .push_bind(rec.upstream_ms)
            .push_bind(rec.ttfb_ms)
            .push_bind(rec.status)
            .push_bind(rec.http_status)
            .push_bind(rec.cached)
            .push_bind(rec.retry_count)
            .push_bind(rec.fallback_used)
            .push_bind(rec.prompt_tokens)
            .push_bind(rec.completion_tokens)
            .push_bind(rec.cached_tokens)
            .push_bind(rec.total_tokens)
            .push_bind(rec.cost_usd)
            .push_bind(rec.would_have_cost_usd)
            .push_bind(metadata_str)
            .push_bind(rec.client_ip)
            .push_bind(rec.user_agent)
            .push_bind(rec.error_message)
            .push_bind(rec.request_body)
            .push_bind(rec.response_body);
    });
    if let Err(e) = qb.build().execute(pool).await {
        metrics::counter!("gateway_log_write_error_total").increment(1);
        tracing::error!(error = %e, "failed to flush log batch");
    } else {
        metrics::counter!("gateway_log_write_total").increment(1);
    }
}
