mod api;
mod discovery;
mod probe;
mod process;
mod project;
mod scheduler;
mod state;

use axum::Router;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use state::AppState;
use std::net::SocketAddr;
use tracing::info;

const BIND_ADDR: &str = "127.0.0.1:7777";

/// Block DNS-rebinding: only accept requests whose Host header matches the
/// loopback address we bind to. A rebound attacker domain would carry its own
/// hostname here and be rejected before reaching any handler.
async fn host_guard(req: Request, next: Next) -> Result<Response, StatusCode> {
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "portbook=info,tower_http=warn".into()),
        )
        .init();

    let state = AppState::new();

    tokio::spawn(scheduler::Scheduler::new(state.clone()).run());

    let app = Router::new()
        .route("/api/ports", get(api::ports))
        .route("/api/stream", get(api::stream))
        .fallback(api::static_handler)
        .with_state(state)
        .layer(middleware::from_fn(host_guard));

    let addr: SocketAddr = BIND_ADDR.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("portbook listening on http://{addr}");

    if std::env::var_os("PORTBOOK_NO_OPEN").is_none() {
        let url = format!("http://{addr}");
        let cmd = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let _ = std::process::Command::new(cmd).arg(&url).spawn();
    }

    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;

    fn test_app() -> Router {
        Router::new()
            .route("/probe", get(|| async { "ok" }))
            .layer(middleware::from_fn(host_guard))
    }

    async fn status_for_host(host: Option<&str>) -> StatusCode {
        let mut req = Request::builder().uri("/probe");
        if let Some(h) = host {
            req = req.header("host", h);
        }
        let req = req.body(Body::empty()).unwrap();
        test_app().oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn allows_loopback_v4() {
        assert_eq!(status_for_host(Some("127.0.0.1:7777")).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn allows_localhost() {
        assert_eq!(status_for_host(Some("localhost:7777")).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn allows_loopback_v6() {
        assert_eq!(status_for_host(Some("[::1]:7777")).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_dns_rebinding() {
        // The whole point: attacker domain that resolved to 127.0.0.1 still
        // carries its own hostname in the Host header.
        assert_eq!(status_for_host(Some("evil.example.com")).await, StatusCode::FORBIDDEN);
        assert_eq!(status_for_host(Some("evil.example.com:7777")).await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_loopback_on_wrong_port() {
        // Defense in depth: don't trust just the hostname half.
        assert_eq!(status_for_host(Some("127.0.0.1:8080")).await, StatusCode::FORBIDDEN);
        assert_eq!(status_for_host(Some("localhost")).await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_missing_host() {
        assert_eq!(status_for_host(None).await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_empty_host() {
        assert_eq!(status_for_host(Some("")).await, StatusCode::FORBIDDEN);
    }
}
