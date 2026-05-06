pub mod api;
pub mod discovery;
pub mod probe;
pub mod process;
pub mod project;
pub mod redact;
pub mod scheduler;
pub mod state;

use axum::Router;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;

pub use state::AppState;

pub const BIND_ADDR: &str = "127.0.0.1:7777";

/// Block DNS-rebinding: only accept requests whose Host header matches the
/// loopback address we bind to. A rebound attacker domain would carry its own
/// hostname here and be rejected before reaching any handler.
pub async fn host_guard(req: Request, next: Next) -> Result<Response, StatusCode> {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let allowed = matches!(host, "127.0.0.1:7777" | "localhost:7777" | "[::1]:7777");
    if !allowed {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(next.run(req).await)
}

pub fn build_app(state: AppState) -> Router {
    Router::new()
        .route("/api/ports", get(api::ports))
        .route("/api/stream", get(api::stream))
        .fallback(api::static_handler)
        .with_state(state)
        .layer(middleware::from_fn(host_guard))
}
