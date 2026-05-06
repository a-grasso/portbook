mod api;
mod discovery;
mod probe;
mod process;
mod project;
mod scheduler;
mod state;

use axum::Router;
use axum::routing::get;
use state::AppState;
use std::net::SocketAddr;
use tracing::info;

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
        .with_state(state);

    let addr: SocketAddr = "127.0.0.1:7777".parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("portbook listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
