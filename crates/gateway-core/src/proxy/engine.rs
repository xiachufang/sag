use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures::stream::{BoxStream, StreamExt};
use gateway_storage::traits::MetadataStore;
use http::header::{HeaderName, HeaderValue, HOST};
use http::HeaderMap;
use reqwest::Method;

use crate::config::{AppConfig, ProviderConfig};
use crate::error::{GatewayError, Result};
use crate::providers::{build_auth_injector, resolve_credential, AuthInjector};
use crate::security::MasterKey;

/// Headers stripped on the way to the upstream. We deliberately drop
/// hop-by-hop headers and any auth supplied by the client (the gateway
/// adds its own).
const STRIP_REQUEST_HEADERS: &[&str] = &[
    "host",
    "authorization",
    "x-api-key",
    "connection",
    "keep-alive",
    "transfer-encoding",
    "te",
    "trailer",
    "upgrade",
    "proxy-authenticate",
    "proxy-authorization",
    "content-length",
];

/// Headers we never relay back to the client (they belong to the upstream
/// transport, not to our gateway response).
const STRIP_RESPONSE_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "transfer-encoding",
    "te",
    "trailer",
    "upgrade",
    "content-encoding", // reqwest already decompressed for us
    "content-length",
];

pub struct ResolvedProvider {
    pub base_url: String,
    pub api_key: String,
    pub injector: Box<dyn AuthInjector>,
    pub extra_headers: HashMap<String, String>,
}

pub struct ProxyEngine {
    client: reqwest::Client,
    providers: HashMap<String, ResolvedProvider>,
}

#[derive(Clone)]
pub struct ForwardRequest {
    pub provider: String,
    /// Path on the upstream side, e.g. `/v1/chat/completions`.
    pub path: String,
    /// Optional raw query string (without the leading `?`).
    pub query: Option<String>,
    pub method: Method,
    pub headers: HeaderMap,
    pub body: Option<Bytes>,
}

pub struct ForwardResponse {
    pub status: http::StatusCode,
    pub headers: HeaderMap,
    pub upstream_ms: u64,
    pub ttfb_ms: u64,
    pub body: ResponseBody,
}

pub enum ResponseBody {
    /// Body delivered as a chunked stream. Used for SSE and large payloads.
    Stream(BoxStream<'static, std::result::Result<Bytes, std::io::Error>>),
}

impl ProxyEngine {
    pub async fn new(
        config: &AppConfig,
        metadata: Option<Arc<dyn MetadataStore>>,
        master_key: Option<MasterKey>,
        project_id: &str,
    ) -> Result<Self> {
        let client = reqwest::Client::builder()
            .pool_idle_timeout(Duration::from_secs(90))
            .connect_timeout(Duration::from_secs(5))
            .tcp_nodelay(true)
            .build()
            .map_err(|e| GatewayError::Internal(format!("failed to build http client: {e}")))?;

        let mut providers = HashMap::new();
        let env_overrides = HashMap::new();
        for (name, cfg) in &config.providers {
            let resolved = resolve_provider(
                name,
                cfg,
                &env_overrides,
                metadata.as_ref(),
                master_key.as_ref(),
                project_id,
            )
            .await?;
            providers.insert(name.clone(), resolved);
        }

        Ok(Self { client, providers })
    }

    pub fn provider(&self, name: &str) -> Option<&ResolvedProvider> {
        self.providers.get(name)
    }

    pub async fn forward(&self, req: ForwardRequest) -> Result<ForwardResponse> {
        let provider = self
            .providers
            .get(&req.provider)
            .ok_or_else(|| GatewayError::ProviderUnknown(req.provider.clone()))?;

        let url = build_url(&provider.base_url, &req.path, req.query.as_deref())?;

        let mut headers = filter_request_headers(&req.headers);
        provider.injector.inject(&mut headers, &provider.api_key);
        for (k, v) in &provider.extra_headers {
            if let (Ok(name), Ok(val)) =
                (HeaderName::try_from(k.as_str()), HeaderValue::from_str(v))
            {
                headers.insert(name, val);
            }
        }
        // Ensure no client-supplied Host header leaks.
        headers.remove(HOST);

        let started = Instant::now();
        let mut request = self.client.request(req.method, &url).headers(headers);
        if let Some(body) = req.body {
            request = request.body(body);
        }

        let resp = request.send().await.map_err(|e| {
            if e.is_timeout() {
                GatewayError::UpstreamTimeout
            } else {
                GatewayError::Http(e)
            }
        })?;
        let ttfb_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;

        let status = resp.status();
        let upstream_headers = resp.headers().clone();
        let filtered_headers = filter_response_headers(&upstream_headers);
        let stream = resp
            .bytes_stream()
            .map(|item| item.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)))
            .boxed();

        Ok(ForwardResponse {
            status,
            headers: filtered_headers,
            upstream_ms: ttfb_ms,
            ttfb_ms,
            body: ResponseBody::Stream(stream),
        })
    }
}

fn build_url(base: &str, path: &str, query: Option<&str>) -> Result<String> {
    let base = base.trim_end_matches('/');
    let mut path = path.to_string();
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    let mut url = format!("{base}{path}");
    if let Some(q) = query {
        if !q.is_empty() {
            url.push('?');
            url.push_str(q);
        }
    }
    Ok(url)
}

fn filter_request_headers(src: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::with_capacity(src.len());
    for (k, v) in src.iter() {
        let name = k.as_str().to_ascii_lowercase();
        if STRIP_REQUEST_HEADERS.iter().any(|s| *s == name) {
            continue;
        }
        if name.starts_with("x-gateway-") {
            continue;
        }
        out.append(k.clone(), v.clone());
    }
    out
}

fn filter_response_headers(src: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::with_capacity(src.len());
    for (k, v) in src.iter() {
        let name = k.as_str().to_ascii_lowercase();
        if STRIP_RESPONSE_HEADERS.iter().any(|s| *s == name) {
            continue;
        }
        out.append(k.clone(), v.clone());
    }
    out
}

async fn resolve_provider(
    name: &str,
    cfg: &ProviderConfig,
    env_overrides: &HashMap<String, String>,
    metadata: Option<&Arc<dyn MetadataStore>>,
    master: Option<&MasterKey>,
    project_id: &str,
) -> Result<ResolvedProvider> {
    let kind = cfg.kind.as_deref().unwrap_or(name);
    let injector = build_auth_injector(kind)?;
    let api_key = resolve_credential(cfg, env_overrides, metadata, master, project_id).await?;
    Ok(ResolvedProvider {
        base_url: cfg.base_url.clone(),
        api_key,
        injector,
        extra_headers: cfg.headers.clone(),
    })
}

/// Allow tests / shutdown to hold the engine behind an Arc cheaply.
impl ProxyEngine {
    pub fn into_arc(self) -> Arc<Self> {
        Arc::new(self)
    }
}
