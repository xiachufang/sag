use std::collections::HashMap;
use std::sync::Arc;

use gateway_core::config::{AppConfig, GatewayKeyConfig};
use gateway_core::security::{derive_hash, parse_gateway_key, MasterKey};
use gateway_storage::models::{NewGatewayKey, KEY_ORIGIN_CONFIG};
use gateway_storage::traits::MetadataStore;

/// Apply the `gateway_keys` block from the config to the metadata store:
/// upsert each declared key, then prune any previously-seeded keys whose
/// id is no longer present. Safe to call at boot and on every reload.
pub async fn apply(
    metadata: &Arc<dyn MetadataStore>,
    master_key: &MasterKey,
    default_project_id: &str,
    keys: &[GatewayKeyConfig],
) -> Result<(), String> {
    // First pass: validate + group ids by project so the prune step at the
    // end can delete config keys whose id was dropped from the YAML.
    let mut by_project: HashMap<String, Vec<String>> = HashMap::new();
    let mut to_upsert: Vec<(GatewayKeyConfig, String, String)> = Vec::new();
    for k in keys {
        let plaintext = k.resolve_secret().map_err(|e| {
            format!("gateway_keys[id={}].secret could not be resolved: {e}", k.id)
        })?;
        let prefix = parse_gateway_key(&plaintext).ok_or_else(|| {
            format!(
                "gateway_keys[id={}].secret must start with 'sk-gw-live-' or 'sk-gw-test-'",
                k.id
            )
        })?;
        let project_id = k
            .project_id
            .clone()
            .unwrap_or_else(|| default_project_id.to_string());
        by_project
            .entry(project_id.clone())
            .or_default()
            .push(k.id.clone());
        to_upsert.push((k.clone(), plaintext, prefix.to_string()));
        // `project_id` already moved into the by_project entry above.
    }

    for (k, plaintext, prefix) in to_upsert {
        let project_id = k
            .project_id
            .clone()
            .unwrap_or_else(|| default_project_id.to_string());
        let hash = derive_hash(master_key, &plaintext);
        let last4: String = plaintext.chars().rev().take(4).collect::<Vec<_>>()
            .into_iter().rev().collect();
        metadata
            .upsert_seeded_key(NewGatewayKey {
                id: k.id.clone(),
                project_id,
                name: k.name.clone(),
                prefix,
                hash,
                last4,
                scopes: k.scopes.clone(),
                expires_at: None,
                origin: KEY_ORIGIN_CONFIG.to_string(),
            })
            .await
            .map_err(|e| format!("upsert gateway_keys[id={}] failed: {e}", k.id))?;
    }

    // Prune per project: anything still flagged origin='config' that wasn't
    // in the current YAML gets deleted. If a project has no keys declared
    // anymore, we still need to prune previously-seeded keys for it — but
    // the only way we'd know about such a project here is if it appeared
    // in a previous config; we don't track that. Operators removing all
    // keys from a project can be handled by listing them in keep_ids: [].
    // For now we only prune projects mentioned in the current config; if
    // we ever need full sweep, expose a projects-of-interest list.
    for (project_id, ids) in by_project {
        metadata
            .prune_seeded_keys_not_in(&project_id, &ids)
            .await
            .map_err(|e| format!("prune seeded keys for project '{project_id}' failed: {e}"))?;
    }
    // Always prune the default project too so removing the last seeded key
    // from the YAML actually deletes it.
    if !keys
        .iter()
        .any(|k| k.project_id.as_deref().unwrap_or(default_project_id) == default_project_id)
    {
        metadata
            .prune_seeded_keys_not_in(default_project_id, &[])
            .await
            .map_err(|e| format!("prune seeded keys for default project failed: {e}"))?;
    }
    Ok(())
}

/// Convenience wrapper used by the reload watcher.
pub async fn apply_from_config(
    metadata: &Arc<dyn MetadataStore>,
    master_key: &MasterKey,
    default_project_id: &str,
    config: &AppConfig,
) {
    if let Err(e) = apply(metadata, master_key, default_project_id, &config.gateway_keys).await {
        tracing::warn!(error = %e, "config-seeded gateway keys not applied");
        metrics::counter!("gateway_seeded_keys_error_total").increment(1);
    } else {
        metrics::counter!("gateway_seeded_keys_apply_total").increment(1);
    }
}
