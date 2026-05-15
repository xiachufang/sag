use bytes::BytesMut;
use serde_json::Value;

/// Token usage parsed from an upstream response body. None when the
/// response shape is unknown.
#[derive(Debug, Default, Clone, Copy)]
pub struct TokenUsage {
    pub prompt: Option<i64>,
    pub completion: Option<i64>,
    pub cached: Option<i64>,
    pub total: Option<i64>,
}

impl TokenUsage {
    pub fn total_tokens(self) -> Option<i64> {
        if let Some(t) = self.total {
            Some(t)
        } else {
            match (self.prompt, self.completion) {
                (Some(p), Some(c)) => Some(p + c),
                _ => None,
            }
        }
    }
}

/// Try to extract usage from a buffered response body. Handles both the
/// OpenAI non-streaming shape and the Anthropic /messages shape, plus a
/// best-effort scan over the last SSE `usage` event for streaming
/// responses.
pub fn extract_token_usage(body: &BytesMut) -> TokenUsage {
    if body.is_empty() {
        return TokenUsage::default();
    }
    if let Ok(v) = serde_json::from_slice::<Value>(body.as_ref()) {
        return from_value(&v);
    }
    // Streaming: scan for `data: {...}` lines and merge usage from each.
    let mut usage = TokenUsage::default();
    for line in body.as_ref().split(|b| *b == b'\n') {
        let line = trim_ascii(line);
        let payload = match line.strip_prefix(b"data: ") {
            Some(p) => p,
            None => continue,
        };
        if payload == b"[DONE]" {
            continue;
        }
        if let Ok(v) = serde_json::from_slice::<Value>(payload) {
            let u = from_value(&v);
            if u.prompt.is_some() {
                usage.prompt = u.prompt;
            }
            if u.completion.is_some() {
                usage.completion = u.completion;
            }
            if u.cached.is_some() {
                usage.cached = u.cached;
            }
            if u.total.is_some() {
                usage.total = u.total;
            }
        }
    }
    usage
}

fn from_value(v: &Value) -> TokenUsage {
    let mut u = TokenUsage::default();

    if let Some(usage) = v.get("usage").and_then(|x| x.as_object()) {
        u.prompt = usage
            .get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(|x| x.as_i64());
        u.completion = usage
            .get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(|x| x.as_i64());
        u.total = usage.get("total_tokens").and_then(|x| x.as_i64());
        u.cached = usage
            .get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(|x| x.as_i64())
            .or_else(|| {
                usage
                    .get("cache_read_input_tokens")
                    .and_then(|x| x.as_i64())
            });
    }
    u
}

fn trim_ascii(s: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = s.len();
    while start < end && (s[start] == b' ' || s[start] == b'\t' || s[start] == b'\r') {
        start += 1;
    }
    while end > start && (s[end - 1] == b' ' || s[end - 1] == b'\t' || s[end - 1] == b'\r') {
        end -= 1;
    }
    &s[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_chat() {
        let body = br#"{"id":"x","usage":{"prompt_tokens":11,"completion_tokens":5,"total_tokens":16}}"#;
        let mut buf = BytesMut::new();
        buf.extend_from_slice(body);
        let u = extract_token_usage(&buf);
        assert_eq!(u.prompt, Some(11));
        assert_eq!(u.completion, Some(5));
        assert_eq!(u.total_tokens(), Some(16));
    }

    #[test]
    fn parses_anthropic_messages() {
        let body =
            br#"{"id":"x","usage":{"input_tokens":12,"output_tokens":7,"cache_read_input_tokens":3}}"#;
        let mut buf = BytesMut::new();
        buf.extend_from_slice(body);
        let u = extract_token_usage(&buf);
        assert_eq!(u.prompt, Some(12));
        assert_eq!(u.completion, Some(7));
        assert_eq!(u.cached, Some(3));
        assert_eq!(u.total_tokens(), Some(19));
    }
}
