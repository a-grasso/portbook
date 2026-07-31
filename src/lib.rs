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
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;

pub use state::AppState;
pub use version::VersionState;

/// Where the daemon lives unless `--port` / `PORTBOOK_PORT` says otherwise.
pub const DEFAULT_PORT: u16 = 7777;

/// The socket the daemon binds. Loopback-only by design: portbook exposes
/// process cmdlines and cwds, which have no business on a LAN interface.
pub fn bind_addr(port: u16) -> String {
    format!("127.0.0.1:{port}")
}

/// Block DNS-rebinding: only accept requests whose Host header matches the
/// loopback address we bind to. A rebound attacker domain would carry its own
/// hostname here and be rejected before reaching any handler.
pub async fn host_guard(
    State(port): State<u16>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let allowed = host == format!("127.0.0.1:{port}")
        || host == format!("localhost:{port}")
        || host == format!("[::1]:{port}");
    if !allowed {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(next.run(req).await)
}

pub fn print_completions<W: std::io::Write>(
    shell: clap_complete::Shell,
    cmd: &mut clap::Command,
    out: &mut W,
) {
    let name = cmd.get_name().to_string();
    clap_complete::generate(shell, cmd, name, out);
}

/// 0 = info, 1 = debug, 2+ = trace.
pub fn tracing_filter(verbosity: u8) -> &'static str {
    match verbosity {
        0 => "portbook=info,tower_http=warn",
        1 => "portbook=debug,tower_http=info",
        _ => "portbook=trace,tower_http=debug",
    }
}

#[cfg(test)]
mod bind_addr_tests {
    use super::{DEFAULT_PORT, bind_addr};

    #[test]
    fn binds_loopback_on_the_default_port() {
        assert_eq!(bind_addr(DEFAULT_PORT), "127.0.0.1:7777");
    }

    #[test]
    fn binds_loopback_on_a_requested_port() {
        assert_eq!(bind_addr(7778), "127.0.0.1:7778");
    }
}

#[cfg(test)]
mod completions_tests {
    use clap::CommandFactory;
    use clap_complete::Shell;

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
        assert!(tracing_filter(0).contains("tower_http=warn"));
        assert!(!tracing_filter(2).contains("tower_http=warn"));
    }
}

/// `port` is the port the daemon is bound to - the Host allowlist is derived
/// from it, so it must match the listener or every request 403s.
pub fn build_app(state: AppState, version: VersionState, port: u16) -> Router {
    let api = Router::new()
        .route("/api/ports", get(api::ports))
        .route("/api/stream", get(api::stream))
        .with_state(state);
    let version_api = Router::new()
        .route("/api/version", get(api::version))
        .with_state(version);
    api.merge(version_api)
        .fallback(api::static_handler)
        .layer(middleware::from_fn_with_state(port, host_guard))
}
