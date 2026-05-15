// gateway-api: axum HTTP entrypoint. Full module tree is introduced in
// subsequent milestones.

pub mod auth;
pub mod budget;
pub mod error;
pub mod logging;
pub mod metrics;
pub mod ratelimit;
pub mod reload;
pub mod routes;
pub mod server;
pub mod state;
pub mod tokens;

pub use auth::{AdminPrincipal, GatewayKeyPrincipal};
pub use error::ApiError;
pub use server::{build_router, ServerHandle};
pub use state::AppState;
