use thiserror::Error;

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("rate limited")]
    RateLimited,

    #[error("not found")]
    NotFound,

    #[error("invalid request: {0}")]
    BadRequest(String),

    #[error("provider not configured: {0}")]
    ProviderUnknown(String),

    #[error("upstream timeout")]
    UpstreamTimeout,

    #[error("upstream error: status={status}, body={body:?}")]
    UpstreamError { status: u16, body: Option<String> },

    #[error("internal: {0}")]
    Internal(String),

    #[error(transparent)]
    Storage(#[from] gateway_storage::StorageError),

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error(transparent)]
    Serde(#[from] serde_json::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T, E = GatewayError> = std::result::Result<T, E>;

impl GatewayError {
    pub fn status_code(&self) -> u16 {
        match self {
            GatewayError::Unauthorized => 401,
            GatewayError::Forbidden(_) => 403,
            GatewayError::RateLimited => 429,
            GatewayError::NotFound => 404,
            GatewayError::BadRequest(_) => 400,
            GatewayError::ProviderUnknown(_) => 400,
            GatewayError::UpstreamTimeout => 504,
            GatewayError::UpstreamError { status, .. } => *status,
            _ => 500,
        }
    }

    pub fn classification(&self) -> &'static str {
        match self {
            GatewayError::Unauthorized | GatewayError::Forbidden(_) => "auth",
            GatewayError::RateLimited => "rate_limited",
            GatewayError::BadRequest(_) | GatewayError::ProviderUnknown(_) => "bad_request",
            GatewayError::UpstreamTimeout => "timeout",
            GatewayError::UpstreamError { .. } => "upstream_error",
            _ => "gateway_error",
        }
    }
}
