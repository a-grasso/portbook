pub mod api;
pub mod cli;
pub mod discovery;
pub mod engine;
pub mod probe;
pub mod process;
pub mod project;
pub mod redact;
pub mod scheduler;
pub mod state;
pub mod version;

use axum::Router;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;

pub use state::AppState;
pub use version::VersionState;

pub const BIND_ADDR: &str = "127.0.0.1:7777";
pub const SELF_PORT: u16 = 7777;

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

/// Convert a `-v` count flag into a `tracing-subscriber` env-filter
/// directive. 0 = info, 1 = debug, 2+ = trace.
pub fn tracing_filter(verbosity: u8) -> &'static str {
    match verbosity {
        0 => "portbook=info,tower_http=warn",
        1 => "portbook=debug,tower_http=info",
        _ => "portbook=trace,tower_http=debug",
    }
}

#[cfg(test)]
mod verbosity_tests {
    use super::tracing_filter;

    #[test]
    fn zero_means_info() {
        assert!(tracing_filter(0).contains("portbook=info"));
    }

    #[test]
    fn one_v_means_debug() {
        assert!(tracing_filter(1).contains("portbook=debug"));
    }

    #[test]
    fn two_or_more_means_trace() {
        assert!(tracing_filter(2).contains("portbook=trace"));
        assert!(tracing_filter(5).contains("portbook=trace"));
    }

    #[test]
    fn higher_v_implies_louder_dependencies() {
        // -vv should also lift our HTTP middleware to a chattier level
        assert!(tracing_filter(0).contains("tower_http=warn"));
        assert!(!tracing_filter(2).contains("tower_http=warn"));
    }
}

pub fn build_app(state: AppState, version: VersionState) -> Router {
    let api = Router::new()
        .route("/api/ports", get(api::ports))
        .route("/api/stream", get(api::stream))
        .with_state(state);
    let version_api = Router::new()
        .route("/api/version", get(api::version))
        .with_state(version);
    api.merge(version_api)
        .fallback(api::static_handler)
        .layer(middleware::from_fn(host_guard))
}
