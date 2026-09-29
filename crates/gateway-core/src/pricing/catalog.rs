use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{GatewayError, Result};

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

#[derive(Debug, Clone, Default)]
pub struct PricingCatalog {
    by_key: HashMap<(String, String), PricingEntry>,
    // Entries are displayed by API ID; canonical names remain preferred lookup keys.
    pub(super) openrouter: HashMap<String, PricingEntry>,
    pub(super) openrouter_by_slug: HashMap<String, String>,
    pub(super) openrouter_by_slug_model: HashMap<String, String>,
    pub(super) openrouter_by_id_model: HashMap<String, String>,
    pub(super) openrouter_slugs: HashMap<String, String>,
}

impl PricingCatalog {
    pub fn from_str(json: &str) -> Result<Self> {
        let f: CatalogFile = serde_json::from_str(json)
            .map_err(|e| GatewayError::Internal(format!("pricing catalog parse: {e}")))?;
        let mut by_key = HashMap::new();
        for e in f.models {
            by_key.insert((e.provider.clone(), e.model.clone()), e);
        }
        Ok(Self {
            by_key,
            ..Self::default()
        })
    }

    pub fn from_path(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| GatewayError::Internal(format!("read catalog {}: {e}", path.display())))?;
        Self::from_str(&text)
    }

    /// A missing overrides file is fine; malformed or unreadable files are not.
    pub fn from_optional_path(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_str(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(GatewayError::Internal(format!(
                "read catalog {}: {e}",
                path.display()
            ))),
        }
    }

    /// Return a copy of this catalog with `overrides` layered on top —
    /// an override for the same (provider, model) replaces the base entry,
    /// and overrides for unknown models are added.
    pub fn with_overrides<I>(&self, overrides: I) -> Self
    where
        I: IntoIterator<Item = PricingEntry>,
    {
        let mut catalog = self.clone();
        for e in overrides {
            catalog
                .by_key
                .insert((e.provider.clone(), e.model.clone()), e);
        }
        catalog
    }

    pub fn entries(&self) -> impl Iterator<Item = &PricingEntry> {
        self.by_key
            .values()
            .chain(self.openrouter.values().filter(|e| {
                !self
                    .by_key
                    .contains_key(&(e.provider.clone(), e.model.clone()))
                    && !self.openrouter_slugs.get(&e.model).is_some_and(|slug| {
                        self.by_key
                            .contains_key(&(e.provider.clone(), slug.clone()))
                    })
            }))
    }

    pub fn source(&self, provider: &str, model: &str) -> &'static str {
        if self
            .by_key
            .contains_key(&(provider.to_string(), model.to_string()))
        {
            "catalog"
        } else {
            "openrouter"
        }
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
            .or_else(|| self.by_key.get(&("*".to_string(), model.to_string())))
            .or_else(|| {
                // Canonical slug, canonical last segment, API ID, then ID last segment.
                let entry = self
                    .openrouter_by_slug
                    .get(model)
                    .or_else(|| self.openrouter_by_slug_model.get(model))
                    .and_then(|id| self.openrouter.get(id))
                    .or_else(|| self.openrouter.get(model))
                    .or_else(|| {
                        self.openrouter_by_id_model
                            .get(model)
                            .and_then(|id| self.openrouter.get(id))
                    })?;
                let slug = self.openrouter_slugs.get(&entry.model)?;
                // Continue honoring existing canonical-slug overrides after changing display IDs.
                for scope in [provider, "*"] {
                    for name in [&entry.model, slug] {
                        if let Some(price) = self.by_key.get(&(scope.to_string(), name.clone())) {
                            return Some(price);
                        }
                    }
                }
                Some(entry)
            })
    }
}
