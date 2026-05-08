//! The core "tool" — discovery + inspection + probing of local listeners.
//!
//! Every surface (web UI, CLI, agent endpoints) goes through `Engine`.
//! See `ARCHITECTURE.md` for the layering rules.

use crate::SELF_PORT;
use crate::discovery::{Listener, PortEnumerator};
use crate::probe::Prober;
use crate::process::{ProcInfo, ProcessInspector};
use crate::state::PortCard;
use futures::StreamExt;
use futures::stream::Stream;
use std::collections::HashMap;
use std::time::Instant;
use tracing::warn;

pub struct Engine {
    enumerator: Box<dyn PortEnumerator>,
    inspector: Box<dyn ProcessInspector>,
    prober: Prober,
}

impl Engine {
    pub fn new() -> Self {
        Self::with_deps(
            crate::discovery::default(),
            crate::process::default(),
            Prober::new(),
        )
    }

    /// Construct with explicit dependencies (test seam for fake enumerator/inspector).
    pub fn with_deps(
        enumerator: Box<dyn PortEnumerator>,
        inspector: Box<dyn ProcessInspector>,
        prober: Prober,
    ) -> Self {
        Self { enumerator, inspector, prober }
    }

    /// Enumerate listeners and inspect each owning process; skips probing entirely.
    pub fn enumerate_with_procs(&self) -> anyhow::Result<Vec<(Listener, ProcInfo)>> {
        let listeners = self.enumerate()?;
        let inspector = self.inspector.as_ref();
        Ok(listeners
            .into_iter()
            .map(|l| {
                let proc = if l.pid == 0 {
                    ProcInfo::default()
                } else {
                    inspector.inspect(l.pid)
                };
                (l, proc)
            })
            .collect())
    }

    /// Listeners after standard filters (port > 1024, not portbook itself).
    pub fn enumerate(&self) -> anyhow::Result<Vec<Listener>> {
        Ok(self
            .enumerator
            .list()?
            .into_iter()
            .filter(|l| l.port > 1024 && l.port != SELF_PORT)
            .collect())
    }

    /// Probe listeners in parallel, yielding each `PortCard` as its probe completes.
    /// Process inspection is supplied pre-done so this only waits on probes.
    /// Use [`Engine::scan`] when the consumer wants a final-only result.
    pub fn scan_stream<'a>(
        &'a self,
        pairs: Vec<(Listener, ProcInfo)>,
    ) -> impl Stream<Item = PortCard> + 'a {
        let prober = &self.prober;
        futures::stream::iter(pairs)
            .map(move |(l, proc)| async move {
                let probe = prober.probe(l.port).await;
                PortCard::build(l.port, l.pid, l.command.clone(), &proc, &probe)
            })
            .buffer_unordered(64)
    }

    /// Probe + inspect listeners in parallel; wall-time is the slowest probe, not the sum.
    pub async fn scan(&self, listeners: Vec<Listener>) -> Vec<PortCard> {
        let inspector = self.inspector.as_ref();
        let prober = &self.prober;
        futures::future::join_all(listeners.into_iter().map(|l| async move {
            let proc = if l.pid == 0 {
                ProcInfo::default()
            } else {
                inspector.inspect(l.pid)
            };
            let probe = prober.probe(l.port).await;
            PortCard::build(l.port, l.pid, l.command.clone(), &proc, &probe)
        }))
        .await
    }

    /// Enumerate + scan in one call, sorted by port.
    pub async fn scan_all(&self) -> anyhow::Result<Vec<PortCard>> {
        let listeners = self.enumerate()?;
        let mut cards = self.scan(listeners).await;
        cards.sort_by_key(|c| c.port);
        Ok(cards)
    }

    /// Run one poll cycle: yields one `Skeleton`, one `Resolved` per probe completion,
    /// then `Done` with cycle wall time. Cache strategy is the caller's choice
    /// (see [`CycleCache`]).
    pub fn run_cycle<'a, C: CycleCache + 'a>(
        &'a self,
        cache: &'a mut C,
    ) -> impl Stream<Item = CycleEvent> + 'a {
        async_stream::stream! {
            let started = Instant::now();
            let pairs = match self.enumerate_with_procs() {
                Ok(p) => p,
                Err(e) => {
                    warn!("engine cycle: enumerate failed: {e:#}");
                    yield CycleEvent::Done {
                        elapsed_ms: elapsed_ms_capped(started),
                    };
                    return;
                }
            };

            let listeners: Vec<Listener> = pairs.iter().map(|(l, _)| l.clone()).collect();
            cache.retain_present(&listeners);

            let mut map: HashMap<u16, PortCard> = HashMap::with_capacity(pairs.len());
            let mut to_probe: Vec<(Listener, ProcInfo)> = Vec::new();
            for (l, proc) in pairs {
                match cache.lookup(&l) {
                    Some(card) => {
                        map.insert(l.port, card);
                    }
                    None => {
                        map.insert(
                            l.port,
                            PortCard::pending(l.port, l.pid, l.command.clone(), &proc),
                        );
                        to_probe.push((l, proc));
                    }
                }
            }

            // Always emit, even when nothing needs re-probing, so consumers that
            // rebuild working state per cycle see the engine's starting map.
            // Whether to surface it to the user is the consumer's call.
            yield CycleEvent::Skeleton(map.clone());

            let mut stream = std::pin::pin!(self.scan_stream(to_probe));
            while let Some(card) = stream.next().await {
                cache.insert(&card);
                yield CycleEvent::Resolved(Box::new(card));
            }

            yield CycleEvent::Done {
                elapsed_ms: elapsed_ms_capped(started),
            };
        }
    }
}

fn elapsed_ms_capped(started: Instant) -> u32 {
    started.elapsed().as_millis().min(u32::MAX as u128) as u32
}

/// Per-cycle cache for [`Engine::run_cycle`]. Implementations choose the cache key
/// (port-only vs port+pid+command) and which entries to retain across cycles.
pub trait CycleCache {
    /// `None` makes [`Engine::run_cycle`] emit a pending placeholder and probe this cycle.
    fn lookup(&self, listener: &Listener) -> Option<PortCard>;
    fn insert(&mut self, card: &PortCard);
    /// Drop entries for vanished listeners so they don't linger across cycles.
    fn retain_present(&mut self, listeners: &[Listener]);
}

pub enum CycleEvent {
    /// Cached cards + pending placeholders. Always emitted at cycle start, even when
    /// nothing needs probing. Consumers that gate UI on "anything pending" inspect the map.
    Skeleton(HashMap<u16, PortCard>),
    /// Boxed because `PortCard` (~300B) dwarfs the other variants.
    Resolved(Box<PortCard>),
    Done { elapsed_ms: u32 },
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod cycle_tests {
    use super::*;
    use crate::process::ProcessInspector;
    use std::sync::Mutex;

    struct FakeEnum(Vec<Listener>);
    impl PortEnumerator for FakeEnum {
        fn list(&self) -> anyhow::Result<Vec<Listener>> {
            Ok(self.0.clone())
        }
    }

    struct FakeProcs;
    impl ProcessInspector for FakeProcs {
        fn inspect(&self, _pid: u32) -> ProcInfo {
            ProcInfo::default()
        }
    }

    #[derive(Default)]
    struct NoCache {
        inserted: Mutex<Vec<u16>>,
    }
    impl CycleCache for NoCache {
        fn lookup(&self, _l: &Listener) -> Option<PortCard> { None }
        fn insert(&mut self, card: &PortCard) {
            self.inserted.lock().unwrap().push(card.port);
        }
        fn retain_present(&mut self, _ls: &[Listener]) {}
    }

    /// Two listeners on ports unlikely to be bound; ConnectionRefused on loopback
    /// is immediate, so probes resolve well under one second.
    fn two_unbound_listeners() -> Vec<Listener> {
        vec![
            Listener { port: 50991, pid: 1, command: "fakeA".into() },
            Listener { port: 50992, pid: 2, command: "fakeB".into() },
        ]
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_cycle_emits_skeleton_then_resolved_then_done() {
        let engine = Engine::with_deps(
            Box::new(FakeEnum(two_unbound_listeners())),
            Box::new(FakeProcs),
            Prober::new(),
        );
        let mut cache = NoCache::default();
        let stream = engine.run_cycle(&mut cache);
        let events: Vec<CycleEvent> = futures::StreamExt::collect(Box::pin(stream)).await;

        assert!(events.len() >= 3, "got {} events", events.len());
        assert!(
            matches!(events.first(), Some(CycleEvent::Skeleton(_))),
            "first event must be Skeleton"
        );
        assert!(
            matches!(events.last(), Some(CycleEvent::Done { .. })),
            "last event must be Done"
        );
        let mid_resolved = events[1..events.len() - 1]
            .iter()
            .filter(|e| matches!(e, CycleEvent::Resolved(_)))
            .count();
        assert_eq!(mid_resolved, 2, "expected 2 Resolved events between skeleton and done");

        if let Some(CycleEvent::Skeleton(map)) = events.first() {
            assert_eq!(map.len(), 2);
            assert!(map.values().all(|c| c.is_pending()));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_cycle_emits_skeleton_with_cached_map_when_nothing_to_probe() {
        struct FullCache;
        impl CycleCache for FullCache {
            fn lookup(&self, l: &Listener) -> Option<PortCard> {
                Some(PortCard {
                    port: l.port,
                    pid: l.pid,
                    command: l.command.clone(),
                    url: format!("http://localhost:{}", l.port),
                    kind: crate::probe::ProbeKind::Live,
                    reason: None,
                    title: Some("cached".into()),
                    description: None,
                    project_root: None,
                    project_name: None,
                    cwd: None,
                    cwd_short: None,
                    cmdline: None,
                    status: Some(200),
                    probed_url: Some(format!("http://127.0.0.1:{}/", l.port)),
                    probed_at_unix: Some(0),
                    elapsed_ms: Some(1),
                    error_class: None,
                    error_detail: None,
                    attempts: 1,
                    pending: false,
                })
            }
            fn insert(&mut self, _c: &PortCard) {}
            fn retain_present(&mut self, _ls: &[Listener]) {}
        }

        let engine = Engine::with_deps(
            Box::new(FakeEnum(two_unbound_listeners())),
            Box::new(FakeProcs),
            Prober::new(),
        );
        let mut cache = FullCache;
        let stream = engine.run_cycle(&mut cache);
        let events: Vec<CycleEvent> = futures::StreamExt::collect(Box::pin(stream)).await;

        // Regression: Skeleton must fire even with an all-cached cycle so the TUI
        // can rebuild working state. Bug seen previously where re-probe path lost cards.
        let skeletons: Vec<&HashMap<u16, PortCard>> = events
            .iter()
            .filter_map(|e| if let CycleEvent::Skeleton(m) = e { Some(m) } else { None })
            .collect();
        assert_eq!(skeletons.len(), 1, "exactly one Skeleton expected");
        assert_eq!(skeletons[0].len(), 2, "skeleton must carry both cached cards");
        assert!(
            skeletons[0].values().all(|c| !c.is_pending()),
            "cached cards stay non-pending in the all-cached skeleton"
        );
        assert!(
            !events.iter().any(|e| matches!(e, CycleEvent::Resolved(_))),
            "no Resolved expected when all cards are cached (nothing to probe)"
        );
        assert!(matches!(events.last(), Some(CycleEvent::Done { .. })));
    }
}
