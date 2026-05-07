//! Periodic polling layer over `Engine`. Adds inter-cycle caching and
//! state broadcast. Knows nothing about discovery / probing internals —
//! see `ARCHITECTURE.md`.

use crate::discovery::Listener;
use crate::engine::Engine;
use crate::state::{AppState, PortCard};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tracing::{info, warn};

pub struct Scheduler {
    engine: Engine,
    state: AppState,
    cache: HashMap<CacheKey, PortCard>,
}

#[derive(Eq, PartialEq, Hash, Clone)]
struct CacheKey {
    port: u16,
    pid: u32,
    command: String,
}

impl CacheKey {
    fn from(l: &Listener) -> Self {
        Self { port: l.port, pid: l.pid, command: l.command.clone() }
    }
}

impl Scheduler {
    pub fn new(state: AppState) -> Self {
        Self { engine: Engine::new(), state, cache: HashMap::new() }
    }

    pub async fn run(mut self) {
        let mut tick = tokio::time::interval(Duration::from_secs(3));
        loop {
            tick.tick().await;
            if let Err(e) = self.cycle().await {
                warn!("scheduler cycle error: {e:#}");
            }
        }
    }

    async fn cycle(&mut self) -> anyhow::Result<()> {
        let cycle_start = Instant::now();
        let pairs = self.engine.enumerate_with_procs()?;

        // Drop cache entries for listeners that vanished.
        let live_keys: HashSet<CacheKey> =
            pairs.iter().map(|(l, _)| CacheKey::from(l)).collect();
        self.cache.retain(|k, _| live_keys.contains(k));

        // Split into cached (instant) and to-probe (delegated to Engine).
        // Build a skeleton map alongside so we can paint the first frame
        // before slow probes finish — uncached rows render as "probing…".
        let mut new_map: HashMap<u16, PortCard> = HashMap::new();
        let mut skeleton_map: HashMap<u16, PortCard> = HashMap::new();
        let mut to_probe: Vec<Listener> = Vec::new();
        for (l, proc) in pairs {
            let key = CacheKey::from(&l);
            if let Some(card) = self.cache.get(&key) {
                new_map.insert(l.port, card.clone());
                skeleton_map.insert(l.port, card.clone());
            } else {
                skeleton_map.insert(
                    l.port,
                    PortCard::pending(l.port, l.pid, l.command.clone(), &proc),
                );
                to_probe.push(l);
            }
        }
        let new_count = to_probe.len();
        let cached_count = new_map.len();

        // Phase 1: emit skeleton for fast first paint. No-op if nothing
        // is uncached — the cached rows are already the "real" answer.
        if new_count > 0 {
            self.state.replace_skeleton(skeleton_map).await;
        }

        // Phase 2: real probes.
        for card in self.engine.scan(to_probe).await {
            self.cache.insert(
                CacheKey {
                    port: card.port,
                    pid: card.pid,
                    command: card.command.clone(),
                },
                card.clone(),
            );
            new_map.insert(card.port, card);
        }

        let elapsed_ms = cycle_start.elapsed().as_millis().min(u32::MAX as u128) as u32;
        self.state.replace(new_map, Some(elapsed_ms)).await;
        if new_count > 0 {
            info!(
                "scheduler cycle: {} new + {} cached in {:?}",
                new_count,
                cached_count,
                cycle_start.elapsed()
            );
        }
        Ok(())
    }
}
