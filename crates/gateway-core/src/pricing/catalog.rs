use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{GatewayError, Result};

/// Embedded copy of `pricing-catalog.json` shipped with the binary.
const EMBEDDED: &str = include_str!("../../../../pricing-catalog.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingEntry {
    pub provider: String,
    pub model: String,
    pub input_per_1k: f64,
    pub output_per_1k: f64,
    #[serde(default)]
    pub cached_input_per_1k: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct CatalogFile {
    #[serde(default)]
    models: Vec<PricingEntry>,
}

#[derive(Debug, Clone)]
pub struct PricingCatalog {
    by_key: HashMap<(String, String), PricingEntry>,
}

impl PricingCatalog {
    pub fn embedded() -> Self {
        Self::from_str(EMBEDDED).expect("embedded catalog must parse")
    }

    pub fn from_str(json: &str) -> Result<Self> {
        let f: CatalogFile = serde_json::from_str(json)
            .map_err(|e| GatewayError::Internal(format!("pricing catalog parse: {e}")))?;
        let mut by_key = HashMap::new();
        for e in f.models {
            by_key.insert((e.provider.clone(), e.model.clone()), e);
        }
        Ok(Self { by_key })
    }

    pub fn from_path(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| GatewayError::Internal(format!("read catalog {}: {e}", path.display())))?;
        Self::from_str(&text)
    }

    pub fn lookup(&self, provider: &str, model: &str) -> Option<&PricingEntry> {
        self.by_key
            .get(&(provider.to_string(), model.to_string()))
            .or_else(|| {
                // Fallback: strip date suffix (e.g. -20250101) when no exact match.
                let stripped = model.rsplit_once('-').and_then(|(prefix, suffix)| {
                    if suffix.chars().all(|c| c.is_ascii_digit()) {
                        Some(prefix.to_string())
                    } else {
                        None
                    }
                });
                stripped.and_then(|m| self.by_key.get(&(provider.to_string(), m)))
            })
    }
}
