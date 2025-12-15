use crate::cache_affinity::{hash_string, CacheAffinityManager};
use crate::provider::{load_providers, Provider};
use anyhow::{Context, Result};
use async_compression::tokio::bufread::GzipDecoder;
use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, Response, StatusCode},
};
use bytes::Bytes;
use futures::TryStreamExt;
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tokio_util::io::StreamReader;

#[derive(Clone)]
struct ResolvedProvider {
    kind: String,
    api_url: String,
    api_key: String,
    name: Option<String>,
    level: i32,
}

#[derive(Clone)]
pub struct Router {
    affinity_manager: Arc<CacheAffinityManager>,
    http_client: reqwest::Client,
    // Cached providers with platform-specific configs
    cached_providers: Arc<RwLock<Vec<ResolvedProvider>>>,
}

impl Router {
    pub fn new(
        affinity_manager: Arc<CacheAffinityManager>,
        http_client: reqwest::Client,
    ) -> Result<Self> {
        let providers = match Self::load_and_flatten_providers() {
            Ok(providers) => providers,
            Err(e) => {
                tracing::warn!("Failed to load providers: {}", e);
                Vec::new()
            }
        };

        Ok(Self {
            affinity_manager,
            http_client,
            cached_providers: Arc::new(RwLock::new(providers)),
        })
    }

    /// Reload providers from disk
    pub async fn reload_providers(&self) -> Result<()> {
        tracing::info!("Reloading providers from config file");

        let providers = Self::load_and_flatten_providers()?;
        let count = providers.len();
        let mut cache = self.cached_providers.write().await;
        *cache = providers;

        tracing::info!("✓ Reloaded {} provider endpoints", count);
        Ok(())
    }

    fn load_and_flatten_providers() -> Result<Vec<ResolvedProvider>> {
        let providers = load_providers()?;

        let resolved = Self::flatten_providers(providers);
        let codex_count = resolved.iter().filter(|p| p.kind == "codex").count();
        let claude_count = resolved.iter().filter(|p| p.kind == "claude").count();

        tracing::info!(
            "Loaded {} provider endpoints (codex={}, claude={})",
            resolved.len(),
            codex_count,
            claude_count
        );

        Ok(resolved)
    }

    fn flatten_providers(providers: Vec<Provider>) -> Vec<ResolvedProvider> {
        let mut resolved = Vec::new();

        for provider in providers.into_iter().filter(|p| p.enabled) {
            for kind in ["codex", "claude"] {
                if let Some(config) = provider.get_platform_config(kind) {
                    if !config.api_url.is_empty() && !config.api_key.is_empty() {
                        resolved.push(ResolvedProvider {
                            kind: kind.to_string(),
                            api_url: config.api_url,
                            api_key: config.api_key,
                            name: provider.name.clone(),
                            level: provider.level,
                        });
                    }
                }
            }
        }

        resolved
    }

    /// Route a request to the appropriate provider
    pub async fn route_request(
        &self,
        kind: &str,
        endpoint: &str,
        body: Bytes,
        headers: HeaderMap,
    ) -> Result<Response<Body>> {
        let start_time = Instant::now();

        // Step 1: Extract request info
        let request_json: Value =
            serde_json::from_slice(&body).context("Failed to parse request body as JSON")?;

        let model = request_json["model"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

        let user_id = self.extract_user_id(&headers);
        let affinity_key = CacheAffinityManager::generate_key(&user_id, kind, &model);

        tracing::debug!(
            "Request: kind={}, model={}, user_id={}",
            kind,
            model,
            user_id
        );

        // Step 2: Check cache affinity
        let cached_provider_id = self.affinity_manager.get(&affinity_key).await;

        // Step 3: Get cached providers (no disk I/O!)
        let providers_lock = self.cached_providers.read().await;
        let providers: Vec<ResolvedProvider> = providers_lock
            .iter()
            .filter(|p| p.kind == kind)
            .cloned()
            .collect();
        drop(providers_lock); // Release lock immediately

        if providers.is_empty() {
            anyhow::bail!("No providers available for {} model: {}", kind, model);
        }

        tracing::debug!(
            "Using {} cached providers: {:?}",
            providers.len(),
            providers
                .iter()
                .map(Self::provider_label)
                .collect::<Vec<_>>()
        );

        // Step 4: Try cached provider first if available
        if let Some(ref cached_id) = cached_provider_id {
            if let Some(provider) = providers
                .iter()
                .find(|p| Self::provider_id(p) == *cached_id)
            {
                tracing::debug!(
                    "Trying cached provider: {} (level {})",
                    Self::provider_label(provider),
                    provider.level
                );

                match self.try_provider(provider, endpoint, &body, &headers).await {
                    Ok(response) => {
                        self.affinity_manager
                            .set(&affinity_key, &Self::provider_id(provider))
                            .await;

                        let duration = start_time.elapsed();
                        tracing::info!(
                            "✓ {} {} → {} [cached] {}ms",
                            kind,
                            model,
                            Self::provider_label(provider),
                            duration.as_millis()
                        );

                        return Ok(response);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "✗ Cached provider failed: {} - {}",
                            Self::provider_label(provider),
                            e
                        );
                        self.affinity_manager.invalidate(&affinity_key).await;
                    }
                }
            }
        }

        // Step 5: Try all providers in priority order
        for (idx, provider) in providers.iter().enumerate() {
            // Skip if this is the cached provider we already tried
            if cached_provider_id.as_ref() == Some(&Self::provider_id(provider)) {
                continue;
            }

            tracing::debug!(
                "Trying provider: {} (priority #{} level {})",
                Self::provider_label(provider),
                idx + 1,
                provider.level
            );

            match self.try_provider(provider, endpoint, &body, &headers).await {
                Ok(response) => {
                    self.affinity_manager
                        .set(&affinity_key, &Self::provider_id(provider))
                        .await;

                    let duration = start_time.elapsed();
                    tracing::info!(
                        "✓ {} {} → {} {}ms",
                        kind,
                        model,
                        Self::provider_label(provider),
                        duration.as_millis()
                    );

                    return Ok(response);
                }
                Err(e) => {
                    tracing::warn!(
                        "✗ Provider failed: {} - {}",
                        Self::provider_label(provider),
                        e
                    );
                }
            }
        }

        // Step 6: All providers failed
        anyhow::bail!(
            "All {} providers failed for model: {}",
            providers.len(),
            model
        )
    }

    fn provider_id(provider: &ResolvedProvider) -> String {
        format!("{}::{}", provider.kind, provider.api_url)
    }

    fn provider_label(provider: &ResolvedProvider) -> String {
        if let Some(name) = provider.name.as_ref().filter(|n| !n.is_empty()) {
            format!("{} ({})", name, provider.api_url)
        } else {
            provider.api_url.clone()
        }
    }

    /// Try to forward request to a specific provider
    async fn try_provider(
        &self,
        provider: &ResolvedProvider,
        endpoint: &str,
        body: &Bytes,
        headers: &HeaderMap,
    ) -> Result<Response<Body>> {
        // Construct URL
        let url = format!("{}{}", provider.api_url.trim_end_matches('/'), endpoint);

        // Prepare headers - convert from axum HeaderMap to reqwest HeaderMap
        let mut req_headers = reqwest::header::HeaderMap::new();
        for (key, value) in headers {
            if key == "host" || key == "authorization" {
                continue;
            }

            let lower = key.as_str().to_ascii_lowercase();
            let is_hop_by_hop = matches!(
                lower.as_str(),
                "connection"
                    | "proxy-connection"
                    | "keep-alive"
                    | "transfer-encoding"
                    | "upgrade"
                    | "te"
                    | "trailers"
            );
            if is_hop_by_hop || lower == "content-length" {
                continue;
            }

            // Convert header name and value
            if let Ok(req_name) = reqwest::header::HeaderName::from_bytes(key.as_str().as_bytes()) {
                if let Ok(val) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) {
                    req_headers.insert(req_name, val);
                }
            }
        }

        // Set provider's API key
        req_headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", provider.api_key))?,
        );

        // Ensure Accept header
        if !req_headers.contains_key(reqwest::header::ACCEPT) {
            req_headers.insert(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("application/json"),
            );
        }

        // Forward request
        let response = self
            .http_client
            .post(&url)
            .headers(req_headers)
            .body(body.to_vec())
            .send()
            .await
            .context("Failed to send request to provider")?;

        let status = response.status();

        if !status.is_success() {
            anyhow::bail!("Provider returned error status: {}", status);
        }

        // Check for WAF/firewall blocks (provider returns 200 but with error content)
        if let Some(tengine_error) = response.headers().get("x-tengine-error") {
            anyhow::bail!("Provider blocked by WAF: {:?}", tengine_error);
        }

        // Verify content-type is JSON (providers should return application/json)
        let content_type_value = response.headers().get("content-type").cloned();
        if let Some(content_type) = content_type_value {
            if let Ok(ct_str) = content_type.to_str() {
                if !ct_str.contains("application/json") && !ct_str.contains("text/event-stream") {
                    anyhow::bail!("Provider returned non-JSON content-type: {}", ct_str);
                }
            }
        }

        // Convert reqwest::Response to axum Response
        let axum_status = StatusCode::from_u16(status.as_u16())?;
        let mut axum_response = Response::builder().status(axum_status);

        // Copy headers - convert from reqwest to axum
        let mut has_gzip_encoding = false;
        for (key, value) in response.headers() {
            let key_str = key.as_str();
            let is_hop_by_hop = matches!(
                key_str.to_ascii_lowercase().as_str(),
                "connection"
                    | "proxy-connection"
                    | "keep-alive"
                    | "transfer-encoding"
                    | "upgrade"
                    | "te"
                    | "trailers"
            );

            // Check for content-encoding: gzip
            if key_str.eq_ignore_ascii_case("content-encoding") {
                if let Ok(val_str) = value.to_str() {
                    if val_str.eq_ignore_ascii_case("gzip") {
                        has_gzip_encoding = true;
                        tracing::debug!("Response is gzip-encoded, will decompress");
                        // Skip forwarding content-encoding header since we'll decompress
                        continue;
                    }
                }
            }

            // Skip hop-by-hop headers and let hyper set the correct length for the body we forward.
            if is_hop_by_hop || key_str.eq_ignore_ascii_case("content-length") {
                tracing::debug!("Skipping header: {}", key_str);
                continue;
            }

            if let Ok(val) = HeaderValue::from_bytes(value.as_bytes()) {
                tracing::debug!("Forwarding header: {}: {:?}", key_str, val);
                axum_response = axum_response.header(key_str, val);
            }
        }

        // Stream the response body directly without buffering
        let stream = response.bytes_stream().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, e)
        });

        let body = if has_gzip_encoding {
            // Decompress gzipped response
            tracing::debug!("Decompressing gzipped response");
            let reader = StreamReader::new(stream);
            let decoder = GzipDecoder::new(reader);
            let decompressed_stream = tokio_util::io::ReaderStream::new(decoder).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::Other, e)
            });
            Body::from_stream(decompressed_stream)
        } else {
            // Pass through uncompressed
            Body::from_stream(stream.inspect_ok(|chunk| {
                // Debug: log first few bytes of each chunk
                if !chunk.is_empty() {
                    let preview = &chunk[..chunk.len().min(50)];
                    match std::str::from_utf8(preview) {
                        Ok(s) => tracing::debug!("Response chunk (UTF-8): {:?}...", s),
                        Err(_) => tracing::debug!("Response chunk (bytes): {:02x?}...", &preview[..preview.len().min(20)]),
                    }
                }
            }))
        };

        axum_response.body(body).context("Failed to build response")
    }

    /// Extract user ID from Authorization header (hash of API key)
    fn extract_user_id(&self, headers: &HeaderMap) -> String {
        if let Some(auth) = headers.get("authorization") {
            if let Ok(auth_str) = auth.to_str() {
                let token = auth_str.strip_prefix("Bearer ").unwrap_or(auth_str).trim();
                return hash_string(token);
            }
        }
        "anonymous".to_string()
    }
}

/// Create error response
pub fn error_response(status: StatusCode, message: &str) -> Response<Body> {
    let error_json = serde_json::json!({
        "error": message
    });

    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(error_json.to_string()))
        .unwrap()
}
