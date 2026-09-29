use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use super::{PricingCatalog, PricingEntry};
use crate::error::{GatewayError, Result};

const MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<Value>,
}

#[derive(Deserialize)]
struct Model {
    canonical_slug: String,
    pricing: Price,
}

#[derive(Deserialize)]
struct Price {
    prompt: String,
    completion: String,
    input_cache_read: Option<String>,
}

fn per_1k(value: &str) -> Option<f64> {
    let price = value.parse::<f64>().ok()? * 1000.0;
    (price.is_finite() && price >= 0.0).then_some(price)
}

impl PricingCatalog {
    /// Fetch public base prices once at startup, without requiring an API key.
    pub async fn from_openrouter() -> Result<Self> {
        Self::fetch_openrouter(MODELS_URL).await
    }

    async fn fetch_openrouter(url: &str) -> Result<Self> {
        let json = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Self::parse_openrouter(&json)
    }

    fn parse_openrouter(json: &str) -> Result<Self> {
        let mut response: ModelsResponse = serde_json::from_str(json)?;
        let mut catalog = Self::default();
        // Different API IDs can share a canonical slug (e.g. batch variants).
        // Prefer standard prices over variant prices; IDs are not lookup keys.
        response.data.sort_by_key(|value| {
            value
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id.contains(':'))
        });
        for value in response.data {
            let Ok(model) = serde_json::from_value::<Model>(value) else {
                continue;
            };
            let slug = model.canonical_slug;
            if slug.is_empty() || slug.ends_with('/') {
                continue;
            }
            let (Some(input), Some(output)) = (
                per_1k(&model.pricing.prompt),
                per_1k(&model.pricing.completion),
            ) else {
                continue;
            };
            let cached = match model.pricing.input_cache_read {
                Some(value) => match per_1k(&value) {
                    Some(price) => Some(price),
                    None => continue,
                },
                None => None,
            };
            catalog
                .openrouter
                .entry(slug.clone())
                .or_insert(PricingEntry {
                    provider: "*".into(),
                    model: slug,
                    input_per_1k: input,
                    output_per_1k: output,
                    cached_input_per_1k: cached,
                });
        }
        if catalog.openrouter.is_empty() {
            return Err(GatewayError::Internal(
                "OpenRouter returned no usable model prices".into(),
            ));
        }
        let mut by_model = HashMap::new();
        let mut ambiguous = HashSet::new();
        for slug in catalog.openrouter.keys() {
            let name = slug.rsplit('/').next().unwrap().to_string();
            if by_model.insert(name.clone(), slug.clone()).is_some() {
                ambiguous.insert(name);
            }
        }
        // Ambiguous short names require a full canonical slug or a local price.
        by_model.retain(|name, _| !ambiguous.contains(name));
        catalog.openrouter_by_model = by_model;
        Ok(catalog)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pricing::{compute_cost, TokenUsage};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const MODELS: &str = r#"{"data":[
        {"id":"vendor/alias:batch","canonical_slug":"vendor/model-20260101","pricing":{"prompt":"0.000001","completion":"0.000005"}},
        {"id":"vendor/alias","canonical_slug":"vendor/model-20260101","pricing":{"prompt":"0.000002","completion":"0.00001","input_cache_read":"0.0000002"}},
        {"canonical_slug":"vendor/free","pricing":{"prompt":"0","completion":"0"}}
    ]}"#;

    #[test]
    fn canonical_slug_then_last_segment_with_per_token_conversion() {
        let catalog = PricingCatalog::parse_openrouter(MODELS).unwrap();
        for model in ["vendor/model-20260101", "model-20260101"] {
            let price = catalog.lookup("custom-upstream", model).unwrap();
            assert_eq!(price.model, "vendor/model-20260101");
            assert!((price.input_per_1k - 0.002).abs() < 1e-12);
            assert!((price.output_per_1k - 0.01).abs() < 1e-12);
            assert!((price.cached_input_per_1k.unwrap() - 0.0002).abs() < 1e-12);
            let cost = compute_cost(
                &catalog,
                "custom-upstream",
                model,
                TokenUsage {
                    prompt: 1000,
                    completion: 500,
                    cached: 800,
                },
            )
            .unwrap();
            assert!((cost.cost_usd - 0.00556).abs() < 1e-12);
        }
        assert!(catalog.lookup("vendor", "alias").is_none());
        assert!(catalog.lookup("vendor", "vendor/alias").is_none());
        assert_eq!(catalog.lookup("any", "free").unwrap().input_per_1k, 0.0);
        assert_eq!(
            catalog.lookup("any", "free").unwrap().cached_input_per_1k,
            None
        );
    }

    #[test]
    fn full_slug_wins_over_short_name_and_ambiguous_names_are_not_guessed() {
        let catalog = PricingCatalog::parse_openrouter(
            r#"{"data":[
            {"canonical_slug":"model","pricing":{"prompt":"1","completion":"1"}},
            {"canonical_slug":"a/model","pricing":{"prompt":"2","completion":"2"}},
            {"canonical_slug":"a/shared","pricing":{"prompt":"3","completion":"3"}},
            {"canonical_slug":"b/shared","pricing":{"prompt":"4","completion":"4"}}
        ]}"#,
        )
        .unwrap();
        assert_eq!(catalog.lookup("any", "model").unwrap().input_per_1k, 1000.0);
        assert_eq!(
            catalog.lookup("any", "a/model").unwrap().input_per_1k,
            2000.0
        );
        assert!(catalog.lookup("any", "shared").is_none());
        assert!(catalog.lookup("any", "b/shared").is_some());
    }

    #[test]
    fn file_and_admin_overrides_win_and_revert_to_base() {
        let remote = PricingCatalog::parse_openrouter(MODELS).unwrap();
        let file = PricingCatalog::from_str(r#"{"models":[
            {"provider":"*","model":"vendor/model-20260101","input_per_1k":0.02,"output_per_1k":0.03},
            {"provider":"custom","model":"free","input_per_1k":0.04,"output_per_1k":0.05}
        ]}"#).unwrap();
        let base = remote.with_overrides(file.entries().cloned());
        assert_eq!(base.entries().count(), 3);
        for model in ["vendor/model-20260101", "model-20260101"] {
            assert_eq!(base.lookup("any", model).unwrap().input_per_1k, 0.02);
        }
        assert_eq!(base.lookup("custom", "free").unwrap().input_per_1k, 0.04);
        assert_eq!(base.lookup("other", "free").unwrap().input_per_1k, 0.0);
        let admin = base.with_overrides([PricingEntry {
            provider: "*".into(),
            model: "vendor/model-20260101".into(),
            input_per_1k: 0.0,
            output_per_1k: 0.0,
            cached_input_per_1k: Some(0.0),
        }]);
        assert_eq!(
            admin.lookup("any", "model-20260101").unwrap().input_per_1k,
            0.0
        );
        let reverted = base.with_overrides([]);
        assert_eq!(
            reverted
                .lookup("any", "model-20260101")
                .unwrap()
                .input_per_1k,
            0.02
        );
        assert_eq!(base.source("*", "vendor/model-20260101"), "catalog");
        assert_eq!(base.source("*", "vendor/free"), "openrouter");
    }

    #[test]
    fn invalid_rows_are_skipped_without_losing_valid_prices() {
        let catalog = PricingCatalog::parse_openrouter(r#"{"data":[
            {"canonical_slug":"a/negative","pricing":{"prompt":"-1","completion":"0"}},
            {"canonical_slug":"a/nan","pricing":{"prompt":"NaN","completion":"0"}},
            {"canonical_slug":"a/overflow","pricing":{"prompt":"1e308","completion":"0"}},
            {"canonical_slug":"a/bad-cache","pricing":{"prompt":"0","completion":"0","input_cache_read":"bad"}},
            {"id":"missing-slug","pricing":{"prompt":"0","completion":"0"}},
            {"canonical_slug":"a/missing-pricing"},
            {"canonical_slug":"a/good","pricing":{"prompt":"0","completion":"0"}}
        ]}"#).unwrap();
        assert_eq!(catalog.entries().count(), 1);
        assert!(catalog.lookup("any", "good").is_some());
        assert!(PricingCatalog::parse_openrouter(r#"{"data":[]}"#).is_err());
        assert!(PricingCatalog::parse_openrouter(r#"{"error":"unavailable"}"#).is_err());
    }

    async fn mock_endpoint(status: &str, body: &str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/models", listener.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            socket.read(&mut buffer).await.unwrap();
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        url
    }

    #[tokio::test]
    async fn fetches_prices_and_rejects_http_errors_or_invalid_json() {
        let url = mock_endpoint("200 OK", MODELS).await;
        assert!(PricingCatalog::fetch_openrouter(&url)
            .await
            .unwrap()
            .lookup("any", "free")
            .is_some());
        let url = mock_endpoint("503 Service Unavailable", MODELS).await;
        assert!(PricingCatalog::fetch_openrouter(&url).await.is_err());
        let url = mock_endpoint("200 OK", "not json").await;
        assert!(PricingCatalog::fetch_openrouter(&url).await.is_err());
    }
}
