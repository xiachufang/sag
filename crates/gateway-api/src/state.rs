use std::sync::Arc;

use arc_swap::ArcSwap;
use gateway_core::config::AppConfig;
use gateway_core::pricing::PricingCatalog;
use gateway_core::proxy::ProxyEngine;
use gateway_core::security::{AdminTokenSigner, MasterKey};
use gateway_storage::traits::StoreBundle;

use crate::budget::BudgetManager;

/// Shared application state injected into every axum handler.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ArcSwap<AppConfig>>,
    pub proxy: Arc<ProxyEngine>,
    pub stores: StoreBundle,
    pub default_project_id: String,
    pub admin_root_token: Option<String>,
    pub master_key: Arc<MasterKey>,
    pub admin_signer: Arc<AdminTokenSigner>,
    pub pricing: Arc<PricingCatalog>,
    pub budgets: Arc<BudgetManager>,
}

impl AppState {
    pub fn config_snapshot(&self) -> Arc<AppConfig> {
        self.config.load_full()
    }
}
