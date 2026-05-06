use clap::{Parser, Subcommand};
use portbook::{AppState, BIND_ADDR, VersionState, build_app, scheduler::Scheduler, version};
use std::net::SocketAddr;
use tracing::info;

#[derive(Parser)]
#[command(name = "portbook", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the web UI server (default).
    Ui,
    /// List discovered ports in the terminal.
    Ls,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cmd = cli.command.unwrap_or_else(default_command);
    match cmd {
        Command::Ls => portbook::cli::run_ls().await,
        Command::Ui => run_ui().await,
    }
}

fn default_command() -> Command {
    match std::env::var("PORTBOOK_DEFAULT").as_deref() {
        Ok("ls") => Command::Ls,
        _ => Command::Ui,
    }
}

async fn run_ui() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "portbook=info,tower_http=warn".into()),
        )
        .init();

    let state = AppState::new();
    let version_state = VersionState::new();
    version::spawn_check(version_state.clone());
    tokio::spawn(Scheduler::new(state.clone()).run());

    let addr: SocketAddr = BIND_ADDR.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("portbook listening on http://{addr}");

    if std::env::var_os("PORTBOOK_NO_OPEN").is_none() {
        let url = format!("http://{addr}");
        let cmd = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let _ = std::process::Command::new(cmd).arg(&url).spawn();
    }

    axum::serve(listener, build_app(state, version_state)).await?;
    Ok(())
}
