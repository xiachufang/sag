use http::header::{HeaderName, HeaderValue};

use super::AuthInjector;

pub struct AnthropicAuth;

impl AuthInjector for AnthropicAuth {
    fn inject(&self, headers: &mut http::HeaderMap, api_key: &str) {
        if let Ok(v) = HeaderValue::from_str(api_key) {
            headers.insert(HeaderName::from_static("x-api-key"), v);
        }
        // Anthropic requires an `anthropic-version` header; default to a
        // recent supported version unless the caller already set one.
        if !headers.contains_key("anthropic-version") {
            headers.insert(
                HeaderName::from_static("anthropic-version"),
                HeaderValue::from_static("2023-06-01"),
            );
        }
    }
}
