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

/// Defensive cap on the SSE accumulation buffer. Local daemon traffic
/// is trusted, but a malformed event with no `\n\n` boundary would
/// otherwise grow the buffer without bound. 1 MiB comfortably exceeds
/// any realistic snapshot payload (port count × ~1 KiB/card).
const SSE_BUF_CAP: usize = 1 << 20;

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
        let mut parser = SseBuf::default();
        while let Some(chunk) = stream.next().await {
            let bytes = match chunk {
                Ok(b) => b,
                Err(_) => break,
            };
            for snap in parser.feed(&bytes) {
                if tx.send(snap).await.is_err() {
                    return;
                }
            }
        }
    });
    true
}

/// Pure SSE accumulator/parser. `feed` appends bytes and returns any
/// fully-parsed `Snapshot`s whose event boundary just landed. Decoding
/// happens once per complete event (boundary at `\n\n`), which means
/// a multi-byte UTF-8 codepoint split across two TCP reads still
/// decodes correctly — `String::from_utf8_lossy` per chunk would have
/// turned the split into `U+FFFD` and dropped the snapshot.
#[derive(Default)]
struct SseBuf {
    /// Raw bytes accumulated until the next `\n\n` event boundary.
    bytes: Vec<u8>,
    /// True when the last feed forced a buffer reset (cap exceeded).
    /// Subsequent bytes are discarded until the next boundary so we
    /// don't try to decode the tail of a corrupted event.
    poisoned: bool,
}

impl SseBuf {
    fn feed(&mut self, chunk: &[u8]) -> Vec<Snapshot> {
        let mut out = Vec::new();

        if self.poisoned {
            // Skip until the first complete event boundary, then
            // resume normal accumulation past it.
            self.bytes.extend_from_slice(chunk);
            if let Some(idx) = find_event_boundary(&self.bytes) {
                self.bytes.drain(..idx + 2);
                self.poisoned = false;
            } else if self.bytes.len() > SSE_BUF_CAP {
                self.bytes.clear();
            }
            return out;
        }

        self.bytes.extend_from_slice(chunk);
        if self.bytes.len() > SSE_BUF_CAP {
            self.bytes.clear();
            self.poisoned = true;
            return out;
        }

        while let Some(idx) = find_event_boundary(&self.bytes) {
            let event: Vec<u8> = self.bytes.drain(..idx + 2).collect();
            // Only decode at event boundaries — UTF-8 sequences are
            // never split across SSE event boundaries (servers always
            // send `\n\n` as ASCII), so the event itself is valid UTF-8
            // even if individual chunks were not.
            let event = match std::str::from_utf8(&event) {
                Ok(s) => s,
                Err(_) => continue,
            };
            for line in event.lines() {
                let Some(json) = line.strip_prefix("data:") else { continue };
                if let Ok(snap) = serde_json::from_str::<Snapshot>(json.trim()) {
                    out.push(snap);
                }
            }
        }
        out
    }
}

fn find_event_boundary(bytes: &[u8]) -> Option<usize> {
    bytes.windows(2).position(|w| w == b"\n\n")
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

#[cfg(test)]
mod sse_buf_tests {
    use super::*;

    fn snap_event(elapsed: Option<u32>) -> String {
        let snap = Snapshot { ports: vec![], scan_elapsed_ms: elapsed };
        format!("data:{}\n\n", serde_json::to_string(&snap).unwrap())
    }

    #[test]
    fn parses_one_event() {
        let mut buf = SseBuf::default();
        let out = buf.feed(snap_event(Some(42)).as_bytes());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].scan_elapsed_ms, Some(42));
    }

    #[test]
    fn parses_two_events_in_one_chunk() {
        let mut buf = SseBuf::default();
        let mut chunk = snap_event(Some(1));
        chunk.push_str(&snap_event(Some(2)));
        let out = buf.feed(chunk.as_bytes());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].scan_elapsed_ms, Some(1));
        assert_eq!(out[1].scan_elapsed_ms, Some(2));
    }

    #[test]
    fn buffers_event_split_across_chunks() {
        let mut buf = SseBuf::default();
        let event = snap_event(Some(7));
        let bytes = event.as_bytes();
        let split = bytes.len() / 2;
        let out1 = buf.feed(&bytes[..split]);
        assert_eq!(out1.len(), 0, "no event boundary yet");
        let out2 = buf.feed(&bytes[split..]);
        assert_eq!(out2.len(), 1);
        assert_eq!(out2[0].scan_elapsed_ms, Some(7));
    }

    #[test]
    fn handles_utf8_codepoint_split_across_chunks() {
        // A multi-byte UTF-8 sequence (here the U+2026 ellipsis "…",
        // 3 bytes: 0xE2 0x80 0xA6) split across feed() calls must still
        // decode correctly when the event boundary lands. The previous
        // String::from_utf8_lossy-per-chunk implementation would have
        // turned the split into U+FFFD and dropped the snapshot.
        let mut card = crate::state::PortCard::pending(
            8080,
            1,
            "x".into(),
            &crate::process::ProcInfo::default(),
        );
        card.title = Some("loading…".into()); // contains U+2026
        let snap = Snapshot { ports: vec![card], scan_elapsed_ms: None };
        let event = format!("data:{}\n\n", serde_json::to_string(&snap).unwrap());
        let bytes = event.as_bytes();

        // Find the byte position of the first 0xE2 (start of "…").
        let split = bytes.iter().position(|b| *b == 0xE2).unwrap() + 1;
        // Sanity-check we actually split mid-codepoint: byte at split
        // index is the 2nd byte of "…" (continuation byte 0x80).
        assert_eq!(bytes[split], 0x80, "split should land mid-codepoint");

        let mut buf = SseBuf::default();
        assert_eq!(buf.feed(&bytes[..split]).len(), 0);
        let out = buf.feed(&bytes[split..]);
        assert_eq!(out.len(), 1, "split-codepoint event must still decode");
        assert_eq!(
            out[0].ports[0].title.as_deref(),
            Some("loading…"),
            "ellipsis must survive the boundary split"
        );
    }

    #[test]
    fn caps_oversized_buffer_and_resyncs_on_next_boundary() {
        let mut buf = SseBuf::default();
        // Feed >1 MiB without a boundary — should poison + clear.
        let junk = vec![b'x'; SSE_BUF_CAP + 16];
        let out = buf.feed(&junk);
        assert_eq!(out.len(), 0);
        assert!(buf.poisoned, "buffer should be poisoned past the cap");
        assert!(buf.bytes.is_empty(), "buffer should be cleared");

        // Tail of the corrupted event arrives — still ignored.
        assert_eq!(buf.feed(b"more junk").len(), 0);
        // Boundary marks the end of the bad event; next event parses.
        assert_eq!(buf.feed(b"\n\n").len(), 0);
        assert!(!buf.poisoned, "next boundary clears the poison");
        let out = buf.feed(snap_event(Some(99)).as_bytes());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].scan_elapsed_ms, Some(99));
    }
}
