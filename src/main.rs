use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use portbook::cli::{ColorChoice, LsOpts};
use portbook::{AppState, BIND_ADDR, VersionState, build_app, scheduler::Scheduler, tracing_filter, version};
use std::net::SocketAddr;
use tracing::info;

#[derive(Parser)]
#[command(name = "portbook", version, about)]
struct Cli {
    /// Increase log verbosity (-v=debug, -vv=trace). Overrides RUST_LOG.
    #[arg(short, long, action = ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the web UI server (default).
    Ui,
    /// List discovered ports in the terminal.
    Ls(LsArgs),
}

#[derive(Args, Default)]
struct LsArgs {
    /// Show all ports including dead ones (default: collapse dead).
    #[arg(long)]
    all: bool,
    /// Show only live ports.
    #[arg(long, conflicts_with = "all")]
    live: bool,
    /// Color output: auto (default, on when stdout is a tty), always, never.
    #[arg(long, value_enum, default_value_t = CliColor::Auto)]
    color: CliColor,
    /// Emit a single JSON line (machine-readable, no colors).
    #[arg(long)]
    json: bool,
}

#[derive(Default, Debug, Clone, Copy, ValueEnum)]
enum CliColor {
    #[default]
    Auto,
    Always,
    Never,
}

impl From<CliColor> for ColorChoice {
    fn from(c: CliColor) -> Self {
        match c {
            CliColor::Auto => ColorChoice::Auto,
            CliColor::Always => ColorChoice::Always,
            CliColor::Never => ColorChoice::Never,
        }
    }
}

impl From<LsArgs> for LsOpts {
    fn from(a: LsArgs) -> Self {
        LsOpts { all: a.all, live: a.live, color: a.color.into(), json: a.json }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cmd = cli.command.unwrap_or_else(default_command);
    match cmd {
        Command::Ls(args) => portbook::cli::run_ls(args.into()).await,
        Command::Ui => run_ui(cli.verbose).await,
    }
}

fn default_command() -> Command {
    match std::env::var("PORTBOOK_DEFAULT").as_deref() {
        Ok("ls") => Command::Ls(LsArgs::default()),
        _ => Command::Ui,
    }
}

async fn run_ui(verbosity: u8) -> anyhow::Result<()> {
    // -v overrides RUST_LOG; otherwise honor the env var as before.
    let filter = if verbosity > 0 {
        tracing_subscriber::EnvFilter::new(tracing_filter(verbosity))
    } else {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_filter(0).into())
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();

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
