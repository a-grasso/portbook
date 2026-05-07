//! Periodic polling layer over `Engine`. Adds inter-cycle caching and
//! state broadcast. Knows nothing about discovery / probing internals —
//! see `ARCHITECTURE.md`.

use crate::discovery::Listener;
use crate::engine::{CycleCache, CycleEvent, Engine};
use crate::state::{AppState, PortCard};
use futures::StreamExt;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tracing::info;

pub struct Scheduler {
    engine: Engine,
    state: AppState,
    cache: SchedulerCache,
}

/// PID-and-command-keyed cache. A reload that changes PID forces a
/// re-probe (correctness over noise) — see [`CacheKey`].
#[derive(Default)]
struct SchedulerCache {
    inner: HashMap<CacheKey, PortCard>,
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

impl CycleCache for SchedulerCache {
    fn lookup(&self, l: &Listener) -> Option<PortCard> {
        self.inner.get(&CacheKey::from(l)).cloned()
    }

    fn insert(&mut self, card: &PortCard) {
        let key = CacheKey {
            port: card.port,
            pid: card.pid,
            command: card.command.clone(),
        };
        self.inner.insert(key, card.clone());
    }

    fn retain_present(&mut self, listeners: &[Listener]) {
        let live: HashSet<CacheKey> = listeners.iter().map(CacheKey::from).collect();
        self.inner.retain(|k, _| live.contains(k));
    }
}

impl Scheduler {
    pub fn new(state: AppState) -> Self {
        Self {
            engine: Engine::new(),
            state,
            cache: SchedulerCache::default(),
        }
    }

    pub async fn run(mut self) {
        let mut tick = tokio::time::interval(Duration::from_secs(3));
        loop {
            tick.tick().await;
            self.cycle().await;
        }
    }

    async fn cycle(&mut self) {
        let mut new_count: usize = 0;
        let mut total: usize = 0;
        let mut events = std::pin::pin!(self.engine.run_cycle(&mut self.cache));
        while let Some(event) = events.next().await {
            match event {
                CycleEvent::Skeleton(map) => {
                    new_count = map.values().filter(|c| c.is_pending()).count();
                    total = map.len();
                    self.state.replace_skeleton(map).await;
                }
                CycleEvent::Resolved(card) => {
                    self.state.update_one(*card).await;
                }
                CycleEvent::Done { elapsed_ms } => {
                    self.state.mark_done(elapsed_ms).await;
                    if new_count > 0 {
                        info!(
                            "scheduler cycle: {} new + {} cached in {}ms",
                            new_count,
                            total.saturating_sub(new_count),
                            elapsed_ms
                        );
                    }
                }
            }
        }
    }
}
