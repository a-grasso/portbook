use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use portbook::cli::{ColorChoice, ExplainOpts, LsOpts, WatchOpts};
use portbook::{AppState, VersionState, bind_addr, build_app, print_completions, scheduler::Scheduler, tracing_filter, version};
use std::net::SocketAddr;
use tracing::info;

#[derive(Parser)]
#[command(name = "portbook", version, about)]
struct Cli {
    /// Increase log verbosity (-v=debug, -vv=trace). Overrides RUST_LOG.
    #[arg(short, long, action = ArgAction::Count, global = true)]
    verbose: u8,

    /// Port the daemon serves on: where `serve` binds, and where the other
    /// subcommands look for it. Lets a dev build run beside an always-on one.
    #[arg(long, global = true, env = "PORTBOOK_PORT", default_value_t = portbook::DEFAULT_PORT)]
    port: u16,

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
    /// Interactive terminal UI (live updates, filter, expand, open in browser).
    Tui,
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
    // Distinct id so it doesn't collide with the global `--port`; still shown
    // as `<PORT>` in help.
    #[arg(id = "explained_port", value_name = "PORT")]
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
        Command::Ls(args) => portbook::cli::run_ls(args.into(), cli.port).await,
        Command::Watch(args) => portbook::cli::run_watch(args.into(), cli.port).await,
        Command::Tui => {
            let code = portbook::cli::run_tui(cli.port).await?;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        Command::Explain(args) => {
            let code = portbook::cli::run_explain(args.into(), cli.port).await?;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        Command::Serve => run_serve(cli.verbose, cli.port).await,
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

async fn run_serve(verbosity: u8, port: u16) -> anyhow::Result<()> {
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
    tokio::spawn(Scheduler::new(state.clone(), port).run());

    let addr: SocketAddr = bind_addr(port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("portbook listening on http://{addr}");

    if std::env::var_os("PORTBOOK_NO_OPEN").is_none() {
        let url = format!("http://{addr}");
        let cmd = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let _ = std::process::Command::new(cmd).arg(&url).spawn();
    }

    axum::serve(listener, build_app(state, version_state, port)).await?;
    Ok(())
}

#[cfg(test)]
mod port_flag_tests {
    use super::*;
    use portbook::DEFAULT_PORT;
    use std::sync::Mutex;

    /// Catches duplicate arg ids and other clap misconfiguration that would
    /// otherwise only panic at runtime - `explain <PORT>` and the global
    /// `--port` are one typo away from colliding.
    #[test]
    fn cli_definition_is_internally_consistent() {
        Cli::command().debug_assert();
    }

    /// Parse with `PORTBOOK_PORT` pinned to `env`, then restore it. The lock
    /// serializes these against each other: cargo runs tests in parallel, and
    /// clap reads the env at parse time, so an unguarded set here would leak
    /// into a concurrent parse. It also pins the ambient value, so a developer
    /// who exports PORTBOOK_PORT doesn't see phantom failures.
    fn parse_with_env(env: Option<&str>, argv: &[&str]) -> u16 {
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        // Poisoning just means another test asserted while holding it; the env
        // is still restored by then, so the guard is safe to reuse.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let prev = std::env::var_os("PORTBOOK_PORT");
        // SAFETY: every writer of PORTBOOK_PORT goes through this lock, and
        // clap's read happens inside it. The previous value is restored below.
        unsafe {
            match env {
                Some(v) => std::env::set_var("PORTBOOK_PORT", v),
                None => std::env::remove_var("PORTBOOK_PORT"),
            }
        }
        let parsed = Cli::parse_from(argv).port;
        unsafe {
            match prev {
                Some(v) => std::env::set_var("PORTBOOK_PORT", v),
                None => std::env::remove_var("PORTBOOK_PORT"),
            }
        }
        parsed
    }

    #[test]
    fn port_defaults_to_the_well_known_port() {
        assert_eq!(parse_with_env(None, &["portbook", "serve"]), DEFAULT_PORT);
    }

    #[test]
    fn port_flag_overrides_the_default() {
        assert_eq!(Cli::parse_from(["portbook", "serve", "--port", "7778"]).port, 7778);
    }

    // Clients need it too: `ls`/`watch`/`tui` have to reach the daemon they mean.
    #[test]
    fn port_flag_reaches_every_client_subcommand() {
        for sub in ["ls", "watch", "tui"] {
            let cli = Cli::parse_from(["portbook", sub, "--port", "7778"]);
            assert_eq!(cli.port, 7778, "--port ignored by `{sub}`");
        }
    }

    #[test]
    fn explain_keeps_its_positional_port_beside_the_daemon_port() {
        let cli = Cli::parse_from(["portbook", "explain", "3000", "--port", "7778"]);
        assert_eq!(cli.port, 7778, "daemon port");
        match cli.command {
            Some(Command::Explain(args)) => assert_eq!(args.port, 3000, "explained port"),
            _ => panic!("expected the explain subcommand"),
        }
    }

    #[test]
    fn port_falls_back_to_the_env_var() {
        assert_eq!(parse_with_env(Some("7779"), &["portbook", "serve"]), 7779);
    }

    #[test]
    fn explicit_flag_beats_the_env_var() {
        let argv = ["portbook", "serve", "--port", "7778"];
        assert_eq!(parse_with_env(Some("7779"), &argv), 7778);
    }
}
