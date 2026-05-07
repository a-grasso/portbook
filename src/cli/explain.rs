//! `portbook explain <port>` — paste-ready diagnostic for a single port.
//!
//! Prints discovery row + probe record + redacted cmdline. Used by users
//! reporting "why is port X not Live?" — copy the output into an issue.

use super::{fetch_from_daemon, one_shot_scan};
use crate::state::{PortCard, Snapshot};

pub struct ExplainOpts {
    pub port: u16,
    pub json: bool,
}

/// Exit code returned when the requested port isn't currently listening.
/// Distinct from clap's `2` (misuse) and the generic `1` (runtime error).
pub const EXIT_PORT_NOT_FOUND: i32 = 3;

pub async fn run_explain(opts: ExplainOpts) -> anyhow::Result<i32> {
    let snapshot = match fetch_from_daemon().await {
        Some(s) => s,
        None => one_shot_scan().await?,
    };
    let card = match snapshot.ports.iter().find(|c| c.port == opts.port) {
        Some(c) => c,
        None => {
            if opts.json {
                let payload = serde_json::json!({
                    "port": opts.port,
                    "found": false,
                    "available_ports": snapshot.ports.iter().map(|c| c.port).collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string(&payload)?);
            } else {
                eprintln!("port {} is not currently listening", opts.port);
                if !snapshot.ports.is_empty() {
                    let listed: Vec<String> =
                        snapshot.ports.iter().map(|c| c.port.to_string()).collect();
                    eprintln!("known ports: {}", listed.join(", "));
                }
            }
            return Ok(EXIT_PORT_NOT_FOUND);
        }
    };

    if opts.json {
        println!("{}", serde_json::to_string(card)?);
    } else {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        write_text(&mut out, card, &snapshot)?;
    }
    Ok(0)
}

fn write_text<W: std::io::Write>(out: &mut W, c: &PortCard, snap: &Snapshot) -> std::io::Result<()> {
    writeln!(out, "portbook explain :{}", c.port)?;
    writeln!(out, "─────────────────────────────────────────")?;
    writeln!(out, "port             : {}", c.port)?;
    writeln!(out, "pid              : {}", c.pid)?;
    writeln!(out, "command          : {}", c.command)?;
    writeln!(
        out,
        "cmdline          : {}",
        c.cmdline.as_deref().unwrap_or("(unknown)")
    )?;
    writeln!(
        out,
        "cwd              : {}",
        c.cwd.as_deref().unwrap_or("(unknown)")
    )?;
    writeln!(
        out,
        "project          : {}",
        c.project_name.as_deref().unwrap_or("(none detected)")
    )?;
    writeln!(out)?;
    writeln!(out, "kind             : {}", kind_str(c))?;
    writeln!(
        out,
        "reason           : {}",
        c.reason.as_deref().unwrap_or("(none)")
    )?;
    writeln!(
        out,
        "http status      : {}",
        c.status.map(|s| s.to_string()).unwrap_or_else(|| "—".into())
    )?;
    writeln!(
        out,
        "title            : {}",
        c.title.as_deref().unwrap_or("(none)")
    )?;
    writeln!(
        out,
        "description      : {}",
        c.description.as_deref().unwrap_or("(none)")
    )?;
    writeln!(out)?;
    writeln!(out, "probe diagnostics")?;
    writeln!(
        out,
        "  url            : {}",
        c.probed_url.as_deref().unwrap_or("(unknown)")
    )?;
    writeln!(
        out,
        "  elapsed (ms)   : {}",
        c.elapsed_ms.map(|n| n.to_string()).unwrap_or_else(|| "—".into())
    )?;
    writeln!(
        out,
        "  attempts       : {}",
        c.attempts
    )?;
    writeln!(
        out,
        "  error class    : {}",
        c.error_class
            .map(|e| serde_json::to_value(e).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default())
            .unwrap_or_else(|| "—".into())
    )?;
    writeln!(
        out,
        "  error detail   : {}",
        c.error_detail.as_deref().unwrap_or("—")
    )?;
    writeln!(
        out,
        "  probed at      : {}",
        c.probed_at_unix
            .map(|s| format!("unix {s}"))
            .unwrap_or_else(|| "—".into())
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "portbook         : v{} ({} on {})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    )?;
    writeln!(out, "snapshot ports   : {}", snap.ports.len())?;
    Ok(())
}

fn kind_str(c: &PortCard) -> &'static str {
    match c.kind {
        crate::probe::ProbeKind::Live => "live",
        crate::probe::ProbeKind::Error => "error",
        crate::probe::ProbeKind::Dead => "dead",
    }
}
