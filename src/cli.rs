//! Terminal view over the Engine. View-only — no discovery / probe logic
//! here, see `ARCHITECTURE.md`.

mod explain;
mod render;
mod style;
mod tui;
mod watch;
mod width;

use crate::BIND_ADDR;
use crate::engine::Engine;
use crate::state::Snapshot;
use render::render;
use style::Style;
use width::term_width;

pub use explain::{ExplainOpts, run_explain, EXIT_PORT_NOT_FOUND};
pub use style::ColorChoice;
pub use tui::{run_tui, EXIT_NOT_A_TTY};
pub use watch::{WatchOpts, run_watch};

#[derive(Default, Debug, Clone, Copy)]
pub struct LsOpts {
    pub all: bool,
    pub live: bool,
    pub color: ColorChoice,
    pub json: bool,
}

pub async fn run_ls(opts: LsOpts) -> anyhow::Result<()> {
    let snapshot = match fetch_from_daemon().await {
        Some(s) => s,
        // Progress meter is noise for `--json` (machine consumers) and
        // when stderr isn't a tty (piped/redirected). `one_shot_scan_with_progress`
        // double-checks the tty itself, but the explicit gate keeps it
        // off entirely for JSON callers.
        None => one_shot_scan_with_progress(!opts.json).await?,
    };
    let style = Style::resolve(opts.color);
    let width = term_width();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    render(&mut out, &snapshot, opts, &style, width)?;
    Ok(())
}

pub(super) async fn fetch_from_daemon() -> Option<Snapshot> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .ok()?;
    let url = format!("http://{BIND_ADDR}/api/ports");

    // First request: bail fast if daemon isn't there.
    let snap = fetch_once(&client, &url).await?;
    if !is_skeleton(&snap) {
        return Some(snap);
    }
    // Daemon is mid-cycle — its current snapshot is a skeleton (probes
    // in flight). Poll briefly for the resolved version instead of
    // printing pending rows or redoing the scan locally. The daemon
    // will emit a full snapshot within probe-wall-time; we cap at 6s.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    let mut last = snap;
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        match fetch_once(&client, &url).await {
            Some(s) if !is_skeleton(&s) => return Some(s),
            Some(s) => last = s,
            None => break,
        }
    }
    // Still a skeleton after the budget — return None so the caller
    // falls back to a local one-shot scan instead of showing
    // misleading "probing…" rows.
    let _ = last;
    None
}

async fn fetch_once(client: &reqwest::Client, url: &str) -> Option<Snapshot> {
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<Snapshot>().await.ok()
}

fn is_skeleton(snap: &Snapshot) -> bool {
    snap.scan_elapsed_ms.is_none() && snap.ports.iter().any(|c| c.is_pending())
}

pub(super) async fn one_shot_scan() -> anyhow::Result<Snapshot> {
    one_shot_scan_with_progress(false).await
}

/// Local one-shot scan. When `show_progress` is true and stderr is a
/// tty, prints a single-line `probing… N/M (Xs)` indicator that
/// updates as probes resolve and is cleared before stdout output.
pub(super) async fn one_shot_scan_with_progress(show_progress: bool) -> anyhow::Result<Snapshot> {
    use futures::StreamExt;
    use std::io::IsTerminal;

    let start = std::time::Instant::now();
    let engine = Engine::new();
    let pairs = engine.enumerate_with_procs()?;
    let total = pairs.len();

    let progress_on = show_progress && std::io::stderr().is_terminal();
    let mut completed = 0usize;
    let mut cards: std::collections::HashMap<u16, crate::state::PortCard> = std::collections::HashMap::new();

    if progress_on && total > 0 {
        eprint!("\rprobing… 0/{total}");
        let _ = std::io::Write::flush(&mut std::io::stderr());
    }

    let mut stream = std::pin::pin!(engine.scan_streaming_with_procs(pairs));
    while let Some(card) = stream.next().await {
        completed += 1;
        if progress_on {
            eprint!(
                "\rprobing… {completed}/{total} ({:.1}s)\x1b[K",
                start.elapsed().as_secs_f32()
            );
            let _ = std::io::Write::flush(&mut std::io::stderr());
        }
        cards.insert(card.port, card);
    }

    if progress_on {
        // Clear the progress line so the next stdout write starts clean.
        eprint!("\r\x1b[K");
        let _ = std::io::Write::flush(&mut std::io::stderr());
    }

    let mut ports: Vec<_> = cards.into_values().collect();
    ports.sort_by_key(|c| c.port);
    let elapsed_ms = start.elapsed().as_millis().min(u32::MAX as u128) as u32;
    Ok(Snapshot {
        ports,
        scan_elapsed_ms: Some(elapsed_ms),
    })
}
