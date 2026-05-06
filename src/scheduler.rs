use crate::discovery::{Listener, PortEnumerator};
use crate::probe::Prober;
use crate::process::{ProcInfo, ProcessInspector};
use crate::state::{AppState, PortCard};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tracing::{debug, warn};

use crate::SELF_PORT;

pub struct Scheduler {
    enumerator: Box<dyn PortEnumerator>,
    inspector: Box<dyn ProcessInspector>,
    prober: Prober,
    state: AppState,
    cache: HashMap<CacheKey, PortCard>,
}

#[derive(Eq, PartialEq, Hash, Clone)]
struct CacheKey { port: u16, pid: u32, command: String }

impl Scheduler {
    pub fn new(state: AppState) -> Self {
        Self {
            enumerator: crate::discovery::default(),
            inspector: crate::process::default(),
            prober: Prober::new(),
            state,
            cache: HashMap::new(),
        }
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
        let listeners = self.enumerator.list()?;
        let filtered: Vec<Listener> = listeners
            .into_iter()
            .filter(|l| l.port > 1024 && l.port != SELF_PORT)
            .collect();
        debug!("listeners: {}", filtered.len());

        let live_keys: HashSet<CacheKey> = filtered.iter().map(|l| CacheKey {
            port: l.port, pid: l.pid, command: l.command.clone(),
        }).collect();
        self.cache.retain(|k, _| live_keys.contains(k));

        let mut new_map: HashMap<u16, PortCard> = HashMap::new();
        for l in filtered {
            let key = CacheKey { port: l.port, pid: l.pid, command: l.command.clone() };
            if let Some(card) = self.cache.get(&key) {
                new_map.insert(l.port, card.clone());
                continue;
            }
            let proc: ProcInfo = if l.pid == 0 { ProcInfo::default() } else { self.inspector.inspect(l.pid) };
            let probe = self.prober.probe(l.port).await;
            let card = PortCard::build(l.port, l.pid, l.command.clone(), &proc, &probe);
            self.cache.insert(key, card.clone());
            new_map.insert(l.port, card);
        }

        self.state.replace(new_map).await;
        Ok(())
    }
}
