use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use portbook::cli::{ColorChoice, ExplainOpts, LsOpts, WatchOpts};
use portbook::{AppState, BIND_ADDR, VersionState, build_app, print_completions, scheduler::Scheduler, tracing_filter, version};
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
    /// Run the daemon: web UI + JSON API on http://127.0.0.1:7777 (default).
    #[command(alias = "ui")]
    Serve,
    /// List discovered ports in the terminal.
    Ls(LsArgs),
    /// Stream snapshots on an interval (good for piping to jq).
    Watch(WatchArgs),
    /// Explain how a single port was classified (paste-ready diagnostic block).
    Explain(ExplainArgs),
    /// Generate shell completion script (e.g. `portbook completions zsh`).
    Completions {
        /// Target shell.
        shell: Shell,
    },
}

#[derive(Args, Default)]
struct WatchArgs {
    /// Emit one JSON line per change (skips identical snapshots).
    #[arg(long)]
    json: bool,
    /// Color output: auto (default), always, never. Ignored in --json mode.
    #[arg(long, value_enum, default_value_t = CliColor::Auto)]
    color: CliColor,
    /// Polling interval in seconds (min 1).
    #[arg(long, default_value_t = 3)]
    interval: u64,
}

impl From<WatchArgs> for WatchOpts {
    fn from(a: WatchArgs) -> Self {
        WatchOpts { json: a.json, color: a.color.into(), interval_secs: a.interval }
    }
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

#[derive(Args)]
struct ExplainArgs {
    /// Port number to explain.
    port: u16,
    /// Emit a single JSON object instead of a paste-ready text block.
    #[arg(long)]
    json: bool,
}

impl From<ExplainArgs> for ExplainOpts {
    fn from(a: ExplainArgs) -> Self {
        ExplainOpts { port: a.port, json: a.json }
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
        Command::Watch(args) => portbook::cli::run_watch(args.into()).await,
        Command::Explain(args) => {
            let code = portbook::cli::run_explain(args.into()).await?;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        Command::Serve => run_serve(cli.verbose).await,
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            print_completions(shell, &mut cmd, &mut std::io::stdout());
            Ok(())
        }
    }
}

fn default_command() -> Command {
    match std::env::var("PORTBOOK_DEFAULT").as_deref() {
        Ok("ls") => Command::Ls(LsArgs::default()),
        _ => Command::Serve,
    }
}

async fn run_serve(verbosity: u8) -> anyhow::Result<()> {
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
