//! Snapshot stream for the TUI.
//!
//! Tries the running daemon's SSE endpoint first (`/api/stream`). If
//! that fails, falls back to polling the local Engine on a 3s tick.
//! Either way, snapshots flow into the TUI through one mpsc channel.

use crate::BIND_ADDR;
use crate::engine::Engine;
use crate::state::{PortCard, Snapshot};
use futures::StreamExt;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::Sender;

const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// Cheap "is the daemon up?" check used to label the source in the
/// footer. Probing /api/ports is enough — if it answers, SSE will too.
pub async fn daemon_alive() -> bool {
    let url = format!("http://{BIND_ADDR}/api/ports");
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(300))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    matches!(client.get(&url).send().await, Ok(r) if r.status().is_success())
}

pub fn spawn(tx: Sender<Snapshot>) {
    tokio::spawn(async move {
        if try_sse(tx.clone()).await {
            return;
        }
        poll_loop(tx).await;
    });
}

/// Connect to /api/stream and forward decoded snapshots. Returns true if
/// the connection was established and a forwarder task was spawned, false
/// if the daemon isn't reachable.
async fn try_sse(tx: Sender<Snapshot>) -> bool {
    let url = format!("http://{BIND_ADDR}/api/stream");
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    let resp = match client.get(&url).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return false,
    };

    tokio::spawn(async move {
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        while let Some(chunk) = stream.next().await {
            let bytes = match chunk {
                Ok(b) => b,
                Err(_) => break,
            };
            buf.push_str(&String::from_utf8_lossy(&bytes));
            // SSE event boundary is a blank line. Each event is one or
            // more `data:` lines; we only care about the JSON payload.
            while let Some(idx) = buf.find("\n\n") {
                let event: String = buf.drain(..idx + 2).collect();
                for line in event.lines() {
                    let json = match line.strip_prefix("data:") {
                        Some(j) => j.trim(),
                        None => continue,
                    };
                    if let Ok(snap) = serde_json::from_str::<Snapshot>(json)
                        && tx.send(snap).await.is_err()
                    {
                        return;
                    }
                }
            }
        }
    });
    true
}

async fn poll_loop(tx: Sender<Snapshot>) {
    let engine = Engine::new();
    // `last_complete` holds the previous cycle's resolved cards so
    // re-probes on subsequent cycles render the prior view (live/error/
    // dead, complete with title and probe metadata) instead of flashing
    // back to "probing…" every 3 seconds. Only first-time-seen ports
    // ever appear as pending placeholders.
    let mut last_complete: std::collections::HashMap<u16, PortCard> =
        std::collections::HashMap::new();

    if scan_cycle(&engine, &tx, &mut last_complete, true).await.is_err() {
        return;
    }
    let mut ticker = tokio::time::interval(POLL_INTERVAL);
    ticker.tick().await; // discard immediate first tick
    loop {
        ticker.tick().await;
        if scan_cycle(&engine, &tx, &mut last_complete, false)
            .await
            .is_err()
        {
            break;
        }
    }
}

/// One scan cycle. Emits a skeleton snapshot (only when uncached ports
/// exist — typically just first-paint or when a brand-new listener
/// appears), an incremental snapshot per probe completion, then a
/// final snapshot carrying `scan_elapsed_ms` so the TUI footer can
/// stop saying "probing…".
///
/// On cycles after the first, known ports show their last resolved
/// state during re-probe — only newly-discovered ports render as
/// pending. This matches the daemon's caching behavior.
async fn scan_cycle(
    engine: &Engine,
    tx: &Sender<Snapshot>,
    last_complete: &mut std::collections::HashMap<u16, PortCard>,
    with_skeleton: bool,
) -> Result<(), ()> {
    let start = Instant::now();
    let pairs = match engine.enumerate_with_procs() {
        Ok(p) => p,
        Err(_) => return Ok(()),
    };

    // Seed the working map: known ports inherit their last resolved
    // result; unknown ports get a pending placeholder.
    let mut map: std::collections::HashMap<u16, PortCard> = pairs
        .iter()
        .map(|(l, proc)| {
            let card = match last_complete.get(&l.port) {
                Some(prev) => prev.clone(),
                None => PortCard::pending(l.port, l.pid, l.command.clone(), proc),
            };
            (l.port, card)
        })
        .collect();

    let any_pending = map.values().any(|c| c.is_pending());
    if with_skeleton && any_pending {
        let snap = build_snap(&map, None);
        if tx.send(snap).await.is_err() {
            return Err(());
        }
    }

    // Stream probe results, broadcasting after each.
    let mut stream = std::pin::pin!(engine.scan_streaming_with_procs(pairs));
    while let Some(card) = stream.next().await {
        map.insert(card.port, card);
        // Intermediate snapshots stay skeleton-flagged (no timing) so
        // the TUI footer keeps saying "probing…" until the cycle ends.
        if tx.send(build_snap(&map, None)).await.is_err() {
            return Err(());
        }
    }

    // Remember the resolved cards for the next cycle. Vanished ports
    // are naturally forgotten because we rebuild fresh from `pairs`.
    *last_complete = map.clone();

    let elapsed_ms = start.elapsed().as_millis().min(u32::MAX as u128) as u32;
    if tx.send(build_snap(&map, Some(elapsed_ms))).await.is_err() {
        return Err(());
    }
    Ok(())
}

fn build_snap(
    map: &std::collections::HashMap<u16, PortCard>,
    scan_elapsed_ms: Option<u32>,
) -> Snapshot {
    let mut ports: Vec<PortCard> = map.values().cloned().collect();
    ports.sort_by_key(|c| c.port);
    Snapshot { ports, scan_elapsed_ms }
}
