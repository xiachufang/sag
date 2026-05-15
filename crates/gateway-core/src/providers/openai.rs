use http::header::{HeaderName, HeaderValue, AUTHORIZATION};

use super::AuthInjector;

pub struct OpenAiAuth;

impl AuthInjector for OpenAiAuth {
    fn inject(&self, headers: &mut http::HeaderMap, api_key: &str) {
        if let Ok(v) = HeaderValue::from_str(&format!("Bearer {api_key}")) {
            headers.insert(AUTHORIZATION, v);
        }
        // Some OpenAI-compatible providers expect `api-key` instead.
        if let Ok(v) = HeaderValue::from_str(api_key) {
            headers.insert(HeaderName::from_static("api-key"), v);
        }
    }
}
