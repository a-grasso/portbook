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

/// Generate a shell completion script for `cmd` and write it to `out`.
/// Thin wrapper over clap_complete::generate so we can unit-test the
/// surface without spinning up the full binary.
pub fn print_completions<W: std::io::Write>(
    shell: clap_complete::Shell,
    cmd: &mut clap::Command,
    out: &mut W,
) {
    let name = cmd.get_name().to_string();
    clap_complete::generate(shell, cmd, name, out);
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
mod completions_tests {
    use clap::CommandFactory;
    use clap_complete::Shell;

    // A trivial Cli stand-in for completion tests so we don't depend on
    // the binary's main.rs Cli struct from a lib test.
    #[derive(clap::Parser)]
    #[command(name = "portbook")]
    struct DummyCli {
        #[command(subcommand)]
        _command: Option<DummyCmd>,
    }
    #[derive(clap::Subcommand)]
    enum DummyCmd { Ls, Serve }

    #[test]
    fn print_completions_emits_non_empty_bash_script() {
        let mut buf: Vec<u8> = Vec::new();
        super::print_completions(Shell::Bash, &mut DummyCli::command(), &mut buf);
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("portbook"), "completion script should mention the binary name");
        assert!(out.len() > 100, "completion script should be substantial");
    }

    #[test]
    fn print_completions_works_for_zsh() {
        let mut buf: Vec<u8> = Vec::new();
        super::print_completions(Shell::Zsh, &mut DummyCli::command(), &mut buf);
        assert!(!buf.is_empty());
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
