//! Snapshot stream for the TUI.
//!
//! Tries the running daemon's SSE endpoint first (`/api/stream`). If
//! that fails, falls back to polling the local Engine on a 3s tick.
//! Either way, snapshots flow into the TUI through one mpsc channel.

use crate::BIND_ADDR;
use crate::discovery::Listener;
use crate::engine::{CycleCache, CycleEvent, Engine};
use crate::state::{PortCard, Snapshot};
use futures::StreamExt;
use std::collections::HashMap;
use std::time::Duration;
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
    let mut cache = TuiCache::default();

    if run_one_cycle(&engine, &tx, &mut cache, true).await == CycleOutcome::ChannelClosed {
        return;
    }
    let mut ticker = tokio::time::interval(POLL_INTERVAL);
    ticker.tick().await; // discard immediate first tick
    loop {
        ticker.tick().await;
        if run_one_cycle(&engine, &tx, &mut cache, false).await == CycleOutcome::ChannelClosed {
            break;
        }
    }
}

#[derive(Eq, PartialEq)]
enum CycleOutcome {
    Continue,
    ChannelClosed,
}

/// Port-only cache. Re-probes on subsequent cycles inherit the prior
/// resolved card so the TUI doesn't flash back to "probing…" every
/// 3 seconds. Vanished ports are dropped via `retain_present`.
#[derive(Default)]
struct TuiCache {
    inner: HashMap<u16, PortCard>,
}

impl CycleCache for TuiCache {
    fn lookup(&self, l: &Listener) -> Option<PortCard> {
        self.inner.get(&l.port).cloned()
    }
    fn insert(&mut self, card: &PortCard) {
        self.inner.insert(card.port, card.clone());
    }
    fn retain_present(&mut self, listeners: &[Listener]) {
        let live: std::collections::HashSet<u16> = listeners.iter().map(|l| l.port).collect();
        self.inner.retain(|p, _| live.contains(p));
    }
}

/// One scan cycle, sending snapshots into `tx`. The first call emits
/// a skeleton frame for new ports; subsequent cycles do not (anti-
/// flicker — known ports keep their last resolved state during the
/// re-probe).
async fn run_one_cycle(
    engine: &Engine,
    tx: &Sender<Snapshot>,
    cache: &mut TuiCache,
    with_skeleton: bool,
) -> CycleOutcome {
    let mut map: HashMap<u16, PortCard> = HashMap::new();
    let mut events = std::pin::pin!(engine.run_cycle(cache));
    while let Some(event) = events.next().await {
        match event {
            CycleEvent::Skeleton(skel) => {
                map = skel;
                if with_skeleton && tx.send(build_snap(&map, None)).await.is_err() {
                    return CycleOutcome::ChannelClosed;
                }
            }
            CycleEvent::Resolved(card) => {
                map.insert(card.port, *card);
                if tx.send(build_snap(&map, None)).await.is_err() {
                    return CycleOutcome::ChannelClosed;
                }
            }
            CycleEvent::Done { elapsed_ms } => {
                if tx.send(build_snap(&map, Some(elapsed_ms))).await.is_err() {
                    return CycleOutcome::ChannelClosed;
                }
            }
        }
    }
    CycleOutcome::Continue
}

fn build_snap(map: &HashMap<u16, PortCard>, scan_elapsed_ms: Option<u32>) -> Snapshot {
    let mut ports: Vec<PortCard> = map.values().cloned().collect();
    ports.sort_by_key(|c| c.port);
    Snapshot { ports, scan_elapsed_ms }
}
