// gateway-core: business logic (proxy, cache, retry, fallback, ratelimit,
// pricing, budget, providers). Modules are introduced milestone by milestone.

pub mod cache;
pub mod config;
pub mod error;
pub mod pricing;
pub mod providers;
pub mod proxy;
pub mod security;

pub use error::{GatewayError, Result};
