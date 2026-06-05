use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use gateway_storage::models::LogQuery;

use crate::auth::AdminPrincipal;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ListLogsQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub gateway_key_id: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub from: Option<i64>,
    #[serde(default)]
    pub to: Option<i64>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LogsResponse {
    pub items: Vec<serde_json::Value>,
    pub next_cursor: Option<String>,
}

pub async fn list_logs(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
    Query(q): Query<ListLogsQuery>,
) -> Result<Json<LogsResponse>, ApiError> {
    let project_id = q.project_id.or(Some(state.default_project_id.clone()));
    let lq = LogQuery {
        project_id,
        gateway_key_id: q.gateway_key_id,
        namespace: q.namespace,
        model: q.model,
        status: q.status,
        from_ts: q.from,
        to_ts: q.to,
        limit: q.limit.unwrap_or(50),
        cursor: q.cursor,
    };
    let page = state.stores.logs.query(lq).await?;
    let items = page
        .items
        .into_iter()
        .map(|r| {
            let preview = r.request_body.as_deref().and_then(request_preview);
            let mut v = serde_json::to_value(&r).unwrap_or(serde_json::Value::Null);
            if let (serde_json::Value::Object(map), Some(p)) = (&mut v, preview) {
                map.insert("request_preview".into(), serde_json::Value::String(p));
            }
            v
        })
        .collect();
    Ok(Json(LogsResponse {
        items,
        next_cursor: page.next_cursor,
    }))
}

// Per-message sample budget: keep this many chars from the head and tail of
// each message's content, and cap the assembled preview overall.
const PREVIEW_HEAD_CHARS: usize = 24;
const PREVIEW_TAIL_CHARS: usize = 24;
const PREVIEW_MAX_CHARS: usize = 400;

/// Build a one-line sample of a chat request body for the list view: for each
/// entry in `messages`, keep the head and tail of its content joined with an
/// ellipsis. Non-chat bodies (or ones truncated past valid JSON) fall back to
/// a head/tail sample of the raw body.
fn request_preview(body: &str) -> Option<String> {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let mut out = String::new();
    if let Some(msgs) = parsed
        .as_ref()
        .and_then(|v| v.get("messages"))
        .and_then(|m| m.as_array())
    {
        for m in msgs {
            let Some(text) = m.get("content").and_then(content_text) else {
                continue;
            };
            let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("?");
            if !out.is_empty() {
                out.push_str(" | ");
            }
            out.push_str(role);
            out.push_str(": ");
            out.push_str(&clip_middle(
                &normalize_ws(&text),
                PREVIEW_HEAD_CHARS,
                PREVIEW_TAIL_CHARS,
            ));
            if out.chars().count() >= PREVIEW_MAX_CHARS {
                break;
            }
        }
    }
    if out.is_empty() {
        out = clip_middle(&normalize_ws(body), 60, 60);
    }
    // Overall cap so one request with many messages can't bloat the payload.
    if out.chars().count() > PREVIEW_MAX_CHARS {
        out = out.chars().take(PREVIEW_MAX_CHARS).collect::<String>() + "…";
    }
    (!out.is_empty()).then_some(out)
}

/// Pull plain text out of a message `content`: either a bare string or the
/// OpenAI/Anthropic block-array shape (`[{type:"text", text:"…"}, …]`).
fn content_text(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(parts) => {
            let mut out = String::new();
            for p in parts {
                if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    out.push_str(t);
                }
            }
            (!out.is_empty()).then_some(out)
        }
        _ => None,
    }
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Keep the first `head` and last `tail` chars, joined by an ellipsis.
/// Char-based so multi-byte text (e.g. CJK) clips safely.
fn clip_middle(s: &str, head: usize, tail: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= head + tail + 1 {
        return s.to_string();
    }
    let h: String = chars[..head].iter().collect();
    let t: String = chars[chars.len() - tail..].iter().collect();
    format!("{h}…{t}")
}

#[cfg(test)]
mod preview_tests {
    use super::*;

    #[test]
    fn samples_each_message_head_and_tail() {
        let long = "a".repeat(30) + &"b".repeat(30);
        let body = format!(
            r#"{{"messages":[{{"role":"system","content":"be brief"}},{{"role":"user","content":"{long}"}}]}}"#
        );
        let p = request_preview(&body).unwrap();
        assert!(p.starts_with("system: be brief | user: "));
        assert!(p.contains('…'));
        assert!(p.contains(&"a".repeat(24)));
        assert!(p.ends_with(&"b".repeat(24)));
    }

    #[test]
    fn handles_block_array_content_and_cjk() {
        let body = r#"{"messages":[{"role":"user","content":[{"type":"text","text":"你好世界,这是一段足够长的中文文本,用来验证按字符截断不会在多字节边界崩溃,中间再多垫一些字符让总长超过头尾预算之和,结尾在这里"}]}]}"#;
        let p = request_preview(body).unwrap();
        assert!(p.starts_with("user: 你好世界"));
        assert!(p.contains('…'));
        assert!(p.ends_with("结尾在这里"));
    }

    #[test]
    fn falls_back_to_raw_clip_for_non_chat_bodies() {
        let p =
            request_preview(r#"{"input":"embed me","model":"text-embedding-3-small"}"#).unwrap();
        assert!(p.contains("embed me"));
        // Truncated/non-JSON bodies also fall back rather than erroring.
        assert!(request_preview("not json at all").is_some());
    }
}

pub async fn get_log(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let detail = state
        .stores
        .logs
        .get_by_id(&id)
        .await?
        .ok_or(ApiError::Gateway(gateway_core::GatewayError::NotFound))?;
    Ok(Json(serde_json::to_value(&detail.record).unwrap()))
}
