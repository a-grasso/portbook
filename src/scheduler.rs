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
        let listeners = self.engine.enumerate()?;

        // Drop cache entries for listeners that vanished.
        let live_keys: HashSet<CacheKey> = listeners.iter().map(CacheKey::from).collect();
        self.cache.retain(|k, _| live_keys.contains(k));

        // Split into cached (instant) and to-probe (delegated to Engine).
        let mut new_map: HashMap<u16, PortCard> = HashMap::new();
        let mut to_probe: Vec<Listener> = Vec::new();
        for l in listeners {
            let key = CacheKey::from(&l);
            if let Some(card) = self.cache.get(&key) {
                new_map.insert(l.port, card.clone());
            } else {
                to_probe.push(l);
            }
        }
        let new_count = to_probe.len();
        let cached_count = new_map.len();

        for card in self.engine.scan(to_probe).await {
            self.cache
                .insert(CacheKey { port: card.port, pid: card.pid, command: card.command.clone() }, card.clone());
            new_map.insert(card.port, card);
        }

        self.state.replace(new_map).await;
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
