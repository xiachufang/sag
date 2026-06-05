use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use gateway_core::pricing::{PricingCatalog, PricingEntry};
use gateway_storage::models::PricingRow;

use crate::auth::AdminPrincipal;
use crate::error::ApiError;
use crate::state::AppState;

/// Merge admin-set override rows on top of the file catalog. Also used at
/// boot (gateway-bin) to build the initial effective catalog.
pub fn merged_catalog(base: &PricingCatalog, rows: Vec<PricingRow>) -> PricingCatalog {
    base.with_overrides(rows.into_iter().map(row_to_entry))
}

fn row_to_entry(r: PricingRow) -> PricingEntry {
    PricingEntry {
        provider: r.provider,
        model: r.model,
        input_per_1k: r.input_per_1k,
        output_per_1k: r.output_per_1k,
        cached_input_per_1k: r.cached_input_per_1k,
    }
}

/// Rebuild the effective catalog from base + stored overrides and swap it in.
async fn refresh_pricing(state: &AppState) -> Result<(), ApiError> {
    let rows = state.stores.metadata.list_pricing().await?;
    let merged = merged_catalog(&state.pricing_base, rows);
    state.pricing.store(Arc::new(merged));
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct PricingItem {
    pub provider: String,
    pub model: String,
    pub input_per_1k: f64,
    pub output_per_1k: f64,
    pub cached_input_per_1k: Option<f64>,
    /// "override" when an admin-set price shadows (or adds to) the catalog.
    pub source: &'static str,
}

pub async fn list_pricing(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
) -> Result<Json<Vec<PricingItem>>, ApiError> {
    let overrides = state.stores.metadata.list_pricing().await?;
    let override_keys: HashSet<(String, String)> = overrides
        .iter()
        .map(|r| (r.provider.clone(), r.model.clone()))
        .collect();
    let merged = merged_catalog(&state.pricing_base, overrides);
    let mut items: Vec<PricingItem> = merged
        .entries()
        .map(|e| PricingItem {
            provider: e.provider.clone(),
            model: e.model.clone(),
            input_per_1k: e.input_per_1k,
            output_per_1k: e.output_per_1k,
            cached_input_per_1k: e.cached_input_per_1k,
            source: if override_keys.contains(&(e.provider.clone(), e.model.clone())) {
                "override"
            } else {
                "catalog"
            },
        })
        .collect();
    items.sort_by(|a, b| (&a.provider, &a.model).cmp(&(&b.provider, &b.model)));
    Ok(Json(items))
}

#[derive(Debug, Deserialize)]
pub struct UpsertPricingBody {
    pub provider: String,
    pub model: String,
    pub input_per_1k: f64,
    pub output_per_1k: f64,
    #[serde(default)]
    pub cached_input_per_1k: Option<f64>,
}

pub async fn upsert_pricing(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
    Json(body): Json<UpsertPricingBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let provider = body.provider.trim().to_string();
    let model = body.model.trim().to_string();
    if provider.is_empty() || model.is_empty() {
        return Err(ApiError::BadRequest(
            "provider and model are required".into(),
        ));
    }
    let valid = |v: f64| v.is_finite() && v >= 0.0;
    if !valid(body.input_per_1k)
        || !valid(body.output_per_1k)
        || !body.cached_input_per_1k.is_none_or(valid)
    {
        return Err(ApiError::BadRequest(
            "prices must be non-negative numbers".into(),
        ));
    }
    state
        .stores
        .metadata
        .upsert_pricing(PricingRow {
            provider,
            model,
            input_per_1k: body.input_per_1k,
            output_per_1k: body.output_per_1k,
            cached_input_per_1k: body.cached_input_per_1k,
        })
        .await?;
    refresh_pricing(&state).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn delete_pricing(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
    Path((provider, model)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .stores
        .metadata
        .delete_pricing(&provider, &model)
        .await?;
    refresh_pricing(&state).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Debug, Default, Deserialize)]
pub struct RecomputeBody {
    /// Optional UTC epoch-ms window; omitted bounds are unbounded.
    #[serde(default)]
    pub from: Option<i64>,
    #[serde(default)]
    pub to: Option<i64>,
}

/// Re-derive historical `cost_usd` / `would_have_cost_usd` from stored token
/// counts using the *current* effective pricing (catalog + overrides). Rows
/// whose (provider, model) has no price are left untouched and reported back.
pub async fn recompute_costs(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
    body: Option<Json<RecomputeBody>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    // Land any buffered log writes first so they are included in the sweep.
    state.stores.logs.flush().await?;

    let catalog = state.pricing.load();
    let keys = state.stores.logs.distinct_cost_keys(b.from, b.to).await?;
    let mut updated_rows = 0u64;
    let mut models_matched = 0u32;
    let mut models_without_price: Vec<String> = Vec::new();
    for (provider, model) in keys {
        match catalog.lookup(&provider, &model) {
            Some(e) => {
                let cached_rate = e.cached_input_per_1k.unwrap_or(e.input_per_1k);
                updated_rows += state
                    .stores
                    .logs
                    .recompute_costs(
                        &provider,
                        &model,
                        e.input_per_1k,
                        cached_rate,
                        e.output_per_1k,
                        b.from,
                        b.to,
                    )
                    .await?;
                models_matched += 1;
            }
            None => models_without_price.push(format!("{provider}/{model}")),
        }
    }
    Ok(Json(serde_json::json!({
        "updated_rows": updated_rows,
        "models_matched": models_matched,
        "models_without_price": models_without_price,
    })))
}
