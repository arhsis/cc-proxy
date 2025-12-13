use crate::router::{error_response, Router};
use axum::{
    body::Body,
    extract::{Request, State},
    http::{Response, StatusCode},
    routing::post,
    Router as AxumRouter,
};
use std::sync::Arc;
use tower_http::trace::TraceLayer;

const MAX_BODY_BYTES: usize = 5 * 1024 * 1024; // ~5MB guard against oversized payloads

#[derive(Clone)]
pub struct AppState {
    pub router: Arc<Router>,
}

pub fn create_app(router: Arc<Router>) -> AxumRouter {
    let state = AppState { router };

    AxumRouter::new()
        .route("/v1/messages", post(handle_claude))
        .route("/responses", post(handle_codex))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Handle Claude Code requests
async fn handle_claude(
    State(state): State<AppState>,
    request: Request,
) -> Result<Response<Body>, Response<Body>> {
    handle_request(state, request, "claude", "/v1/messages").await
}

/// Handle Codex requests
async fn handle_codex(
    State(state): State<AppState>,
    request: Request,
) -> Result<Response<Body>, Response<Body>> {
    handle_request(state, request, "codex", "/responses").await
}

/// Generic request handler
async fn handle_request(
    state: AppState,
    request: Request,
    kind: &str,
    endpoint: &str,
) -> Result<Response<Body>, Response<Body>> {
    // Extract headers
    let headers = request.headers().clone();

    // Read body
    let body = match axum::body::to_bytes(request.into_body(), MAX_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!("Request body rejected: {}", e);
            return Err(error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Request body too large",
            ));
        }
    };

    // Route request
    match state
        .router
        .route_request(kind, endpoint, body, headers)
        .await
    {
        Ok(response) => Ok(response),
        Err(e) => {
            tracing::error!("Request routing failed: {}", e);
            Err(error_response(
                StatusCode::BAD_GATEWAY,
                &format!("All providers failed: {}", e),
            ))
        }
    }
}

pub async fn run_server(router: Arc<Router>, bind_addr: &str) -> anyhow::Result<()> {
    let app = create_app(router);

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to bind to {}: {}", bind_addr, e))?;

    tracing::info!("🚀 cc-proxy listening on {}", bind_addr);
    tracing::info!("   POST /v1/messages (Claude Code)");
    tracing::info!("   POST /responses (Codex)");

    axum::serve(listener, app)
        .await
        .map_err(|e| anyhow::anyhow!("Server error: {}", e))?;

    Ok(())
}
