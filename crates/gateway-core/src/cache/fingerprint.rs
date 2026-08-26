use serde_json::Value;

/// Inputs to fingerprint a cacheable request.
pub struct FingerprintInputs<'a> {
    /// URL namespace (the `{X}` segment in `/v1/{X}/...`). Scopes the cache
    /// per public route alias.
    pub namespace: &'a str,
    pub endpoint: &'a str,
    pub body: &'a [u8],
    /// Request JSON parsed by the API layer. Supplying it avoids reparsing
    /// large request bodies (notably multimodal requests with base64 data).
    /// `None` means the body was not valid JSON and is hashed verbatim.
    pub json_body: Option<&'a Value>,
    /// Optional cache-key bumper supplied by the caller via header (so
    /// users can force a fresh key without changing the body).
    pub cache_scope: Option<&'a str>,
}

/// Compute a deterministic blake3 hex digest for the complete request body.
/// If the body isn't valid JSON we still hash the raw bytes so the cache layer
/// can be applied uniformly.
pub fn fingerprint(inp: &FingerprintInputs<'_>) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(inp.namespace.as_bytes());
    hasher.update(b"|");
    hasher.update(inp.endpoint.as_bytes());
    hasher.update(b"|");
    if let Some(s) = inp.cache_scope {
        hasher.update(s.as_bytes());
    }
    hasher.update(b"|");

    if let Some(value) = inp.json_body {
        // Hash the complete canonical JSON value. A field allowlist is unsafe
        // for evolving APIs such as OpenAI Responses: omitted fields like
        // `instructions`, `previous_response_id`, or `reasoning` change the
        // generated response and must therefore change the cache key.
        hash_json_value(value, &mut hasher);
    } else {
        // Non-JSON body: hash the raw bytes verbatim.
        hasher.update(inp.body);
    }
    hasher.finalize().to_hex().to_string()
}

fn hash_len(len: usize, hasher: &mut blake3::Hasher) {
    hasher.update(&(len as u64).to_le_bytes());
}

/// Feed an unambiguous, canonical representation directly into BLAKE3.
/// Object keys are sorted and every variable-length value is length-prefixed,
/// so no full canonical JSON string (and no second base64 copy) is allocated.
fn hash_json_value(v: &Value, hasher: &mut blake3::Hasher) {
    match v {
        Value::Null => {
            hasher.update(b"n");
        }
        Value::Bool(b) => {
            hasher.update(if *b { b"b1" } else { b"b0" });
        }
        Value::Number(n) => {
            let encoded = n.to_string();
            hasher.update(b"d");
            hash_len(encoded.len(), hasher);
            hasher.update(encoded.as_bytes());
        }
        Value::String(s) => {
            hasher.update(b"s");
            hash_len(s.len(), hasher);
            hasher.update(s.as_bytes());
        }
        Value::Array(arr) => {
            hasher.update(b"a");
            hash_len(arr.len(), hasher);
            for item in arr {
                hash_json_value(item, hasher);
            }
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            hasher.update(b"o");
            hash_len(keys.len(), hasher);
            for key in keys {
                hash_len(key.len(), hasher);
                hasher.update(key.as_bytes());
                hash_json_value(&map[key], hasher);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_inputs_same_fingerprint() {
        let body = br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}],"temperature":0}"#;
        let json: Value = serde_json::from_slice(body).unwrap();
        let a = fingerprint(&FingerprintInputs {
            namespace: "openai",
            endpoint: "/v1/chat/completions",
            body,
            json_body: Some(&json),
            cache_scope: None,
        });
        let b = fingerprint(&FingerprintInputs {
            namespace: "openai",
            endpoint: "/v1/chat/completions",
            body,
            json_body: Some(&json),
            cache_scope: None,
        });
        assert_eq!(a, b);
    }

    #[test]
    fn object_field_order_does_not_change_fingerprint() {
        let b1 = br#"{"model":"gpt-4o","input":"hi","instructions":"be brief"}"#;
        let b2 = br#"{"instructions":"be brief","input":"hi","model":"gpt-4o"}"#;
        let j1: Value = serde_json::from_slice(b1).unwrap();
        let j2: Value = serde_json::from_slice(b2).unwrap();
        let a = fingerprint(&FingerprintInputs {
            namespace: "openai",
            endpoint: "/v1/responses",
            body: b1,
            json_body: Some(&j1),
            cache_scope: None,
        });
        let b = fingerprint(&FingerprintInputs {
            namespace: "openai",
            endpoint: "/v1/responses",
            body: b2,
            json_body: Some(&j2),
            cache_scope: None,
        });
        assert_eq!(a, b);
    }

    #[test]
    fn strings_with_json_delimiters_do_not_collide() {
        let one_string = br#"["a\",\"b"]"#;
        let two_strings = br#"["a","b"]"#;
        let one_json: Value = serde_json::from_slice(one_string).unwrap();
        let two_json: Value = serde_json::from_slice(two_strings).unwrap();
        let a = fingerprint(&FingerprintInputs {
            namespace: "openai",
            endpoint: "/v1/responses",
            body: one_string,
            json_body: Some(&one_json),
            cache_scope: None,
        });
        let b = fingerprint(&FingerprintInputs {
            namespace: "openai",
            endpoint: "/v1/responses",
            body: two_strings,
            json_body: Some(&two_json),
            cache_scope: None,
        });
        assert_ne!(a, b);
    }

    #[test]
    fn any_request_field_change_busts_fingerprint() {
        let b1 = br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"user":"u1"}"#;
        let b2 = br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"user":"u2"}"#;
        let j1: Value = serde_json::from_slice(b1).unwrap();
        let j2: Value = serde_json::from_slice(b2).unwrap();
        let a = fingerprint(&FingerprintInputs {
            namespace: "openai",
            endpoint: "/v1/chat/completions",
            body: b1,
            json_body: Some(&j1),
            cache_scope: None,
        });
        let b = fingerprint(&FingerprintInputs {
            namespace: "openai",
            endpoint: "/v1/chat/completions",
            body: b2,
            json_body: Some(&j2),
            cache_scope: None,
        });
        assert_ne!(a, b);
    }

    #[test]
    fn temperature_change_busts_fingerprint() {
        let b1 =
            br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"temperature":0}"#;
        let b2 =
            br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"temperature":0.7}"#;
        let j1: Value = serde_json::from_slice(b1).unwrap();
        let j2: Value = serde_json::from_slice(b2).unwrap();
        let a = fingerprint(&FingerprintInputs {
            namespace: "openai",
            endpoint: "/v1/chat/completions",
            body: b1,
            json_body: Some(&j1),
            cache_scope: None,
        });
        let b = fingerprint(&FingerprintInputs {
            namespace: "openai",
            endpoint: "/v1/chat/completions",
            body: b2,
            json_body: Some(&j2),
            cache_scope: None,
        });
        assert_ne!(a, b);
    }

    #[test]
    fn responses_semantic_fields_bust_fingerprint() {
        let base = serde_json::json!({
            "model": "gpt-5",
            "input": "hello",
            "instructions": "be brief",
            "previous_response_id": "resp_1",
            "reasoning": { "effort": "low" },
            "text": { "verbosity": "low" },
            "max_output_tokens": 100,
            "stream": false
        });

        for (field, replacement) in [
            ("instructions", serde_json::json!("be detailed")),
            ("previous_response_id", serde_json::json!("resp_2")),
            ("reasoning", serde_json::json!({ "effort": "high" })),
            ("text", serde_json::json!({ "verbosity": "high" })),
            ("max_output_tokens", serde_json::json!(200)),
            ("stream", serde_json::json!(true)),
        ] {
            let mut changed = base.clone();
            changed[field] = replacement;
            let original_body = serde_json::to_vec(&base).unwrap();
            let changed_body = serde_json::to_vec(&changed).unwrap();

            let original = fingerprint(&FingerprintInputs {
                namespace: "openai",
                endpoint: "/v1/responses",
                body: &original_body,
                json_body: Some(&base),
                cache_scope: None,
            });
            let changed = fingerprint(&FingerprintInputs {
                namespace: "openai",
                endpoint: "/v1/responses",
                body: &changed_body,
                json_body: Some(&changed),
                cache_scope: None,
            });
            assert_ne!(original, changed, "field {field} must affect cache key");
        }
    }
}
