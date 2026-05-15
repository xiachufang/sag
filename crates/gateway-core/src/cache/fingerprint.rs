use serde_json::Value;

/// Fields that are part of the cache fingerprint. Anything not listed here
/// is deliberately ignored so e.g. the OpenAI `user` tag or stream flag
/// don't fracture the key.
const CACHE_FIELDS: &[&str] = &[
    "model",
    "messages",
    "prompt",
    "input",
    "temperature",
    "top_p",
    "top_k",
    "max_tokens",
    "max_completion_tokens",
    "stop",
    "response_format",
    "tools",
    "tool_choice",
    "system",
];

/// Inputs to fingerprint a cacheable request.
pub struct FingerprintInputs<'a> {
    pub provider: &'a str,
    pub endpoint: &'a str,
    pub body: &'a [u8],
    /// Optional namespace bumper supplied by the caller via header (so
    /// users can force a fresh key without changing the body).
    pub namespace: Option<&'a str>,
}

/// Compute a deterministic blake3 hex digest for the cacheable subset of
/// the body. If the body isn't valid JSON we still hash the raw bytes so
/// the cache layer can be applied uniformly.
pub fn fingerprint(inp: &FingerprintInputs<'_>) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(inp.provider.as_bytes());
    hasher.update(b"|");
    hasher.update(inp.endpoint.as_bytes());
    hasher.update(b"|");
    if let Some(ns) = inp.namespace {
        hasher.update(ns.as_bytes());
    }
    hasher.update(b"|");

    match serde_json::from_slice::<Value>(inp.body) {
        Ok(Value::Object(map)) => {
            let mut entries: Vec<(&str, &Value)> = CACHE_FIELDS
                .iter()
                .filter_map(|k| map.get(*k).map(|v| (*k, v)))
                .collect();
            // CACHE_FIELDS is already in a stable order; iterate in that order.
            for (k, v) in entries.drain(..) {
                hasher.update(k.as_bytes());
                hasher.update(b"=");
                let canonical = canonicalize(v);
                hasher.update(canonical.as_bytes());
                hasher.update(b";");
            }
        }
        _ => {
            // Non-JSON body: hash the raw bytes verbatim.
            hasher.update(inp.body);
        }
    }
    hasher.finalize().to_hex().to_string()
}

/// Canonicalize a JSON value into a stable string form so that field
/// ordering doesn't affect the digest.
fn canonicalize(v: &Value) -> String {
    let mut out = String::new();
    push_canonical(v, &mut out);
    out
}

fn push_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => {
            out.push('"');
            out.push_str(s);
            out.push('"');
        }
        Value::Array(arr) => {
            out.push('[');
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('"');
                out.push_str(k);
                out.push_str("\":");
                push_canonical(&map[*k], out);
            }
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_inputs_same_fingerprint() {
        let body = br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}],"temperature":0}"#;
        let a = fingerprint(&FingerprintInputs {
            provider: "openai",
            endpoint: "/v1/chat/completions",
            body,
            namespace: None,
        });
        let b = fingerprint(&FingerprintInputs {
            provider: "openai",
            endpoint: "/v1/chat/completions",
            body,
            namespace: None,
        });
        assert_eq!(a, b);
    }

    #[test]
    fn ignored_field_does_not_change_fingerprint() {
        let b1 = br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"user":"u1"}"#;
        let b2 = br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"user":"u2"}"#;
        let a = fingerprint(&FingerprintInputs {
            provider: "openai",
            endpoint: "/v1/chat/completions",
            body: b1,
            namespace: None,
        });
        let b = fingerprint(&FingerprintInputs {
            provider: "openai",
            endpoint: "/v1/chat/completions",
            body: b2,
            namespace: None,
        });
        assert_eq!(a, b);
    }

    #[test]
    fn temperature_change_busts_fingerprint() {
        let b1 = br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"temperature":0}"#;
        let b2 = br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"temperature":0.7}"#;
        let a = fingerprint(&FingerprintInputs {
            provider: "openai",
            endpoint: "/v1/chat/completions",
            body: b1,
            namespace: None,
        });
        let b = fingerprint(&FingerprintInputs {
            provider: "openai",
            endpoint: "/v1/chat/completions",
            body: b2,
            namespace: None,
        });
        assert_ne!(a, b);
    }
}
