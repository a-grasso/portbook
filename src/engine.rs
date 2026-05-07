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

    /// Construct an engine with explicit dependencies. Used by tests
    /// to swap in fake enumerator/inspector. Production code should
    /// use [`Engine::new`].
    pub fn with_deps(
        enumerator: Box<dyn PortEnumerator>,
        inspector: Box<dyn ProcessInspector>,
        prober: Prober,
    ) -> Self {
        Self { enumerator, inspector, prober }
    }

    /// Enumerate listeners and inspect each owning process — fast.
    /// Skips probing entirely. Used to paint a TUI/web skeleton before
    /// slow HTTP probes complete.
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

    /// All current listeners on the host, after portbook's standard filters
    /// (port > 1024, not portbook itself).
    pub fn enumerate(&self) -> anyhow::Result<Vec<Listener>> {
        Ok(self
            .enumerator
            .list()?
            .into_iter()
            .filter(|l| l.port > 1024 && l.port != SELF_PORT)
            .collect())
    }

    /// Probe the given listeners in parallel, yielding each `PortCard`
    /// as its probe completes. Process inspection is fed in pre-done
    /// (callers typically already inspected during the skeleton phase),
    /// so this method only waits on probes — the long pole.
    ///
    /// Use this when the consumer can act on partial results (TUI,
    /// scheduler broadcasting per resolution, `ls` progress meter). For
    /// a final-only result use [`Engine::scan`].
    pub fn scan_streaming_with_procs<'a>(
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

    /// Probe + inspect the given listeners in parallel. Wall-time is the
    /// slowest single probe, not the sum.
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

    /// Convenience: enumerate + scan in one call. Sorted by port.
    pub async fn scan_all(&self) -> anyhow::Result<Vec<PortCard>> {
        let listeners = self.enumerate()?;
        let mut cards = self.scan(listeners).await;
        cards.sort_by_key(|c| c.port);
        Ok(cards)
    }

    /// Run one full poll cycle and yield the producer events: one
    /// optional `Skeleton` (only when at least one port has no cached
    /// card), one `Resolved` per probe completion, then `Done` with
    /// cycle wall time.
    ///
    /// Both the scheduler (broadcast-to-AppState) and the TUI's local
    /// poll loop go through this method. The cache strategy is the
    /// caller's: PID-sensitive in the scheduler (keyed on (port, pid,
    /// command)), port-only in the TUI. See [`CycleCache`].
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

            if map.values().any(|c| c.is_pending()) {
                yield CycleEvent::Skeleton(map.clone());
            }

            let mut stream = std::pin::pin!(self.scan_streaming_with_procs(to_probe));
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

/// Per-cycle cache for [`Engine::run_cycle`]. Implementations choose the
/// cache key (port-only vs port+pid+command) and decide which prior
/// cards may be reused as skeleton seeds vs forced back to "pending".
pub trait CycleCache {
    /// Return a previously-resolved card for `listener`, if available.
    /// Returning `None` makes [`Engine::run_cycle`] emit a pending
    /// placeholder for this listener and probe it this cycle.
    fn lookup(&self, listener: &Listener) -> Option<PortCard>;
    /// Record a freshly-resolved card.
    fn insert(&mut self, card: &PortCard);
    /// Drop entries for listeners no longer present so a vanished port
    /// doesn't linger across cycles.
    fn retain_present(&mut self, listeners: &[Listener]);
}

/// Producer events from [`Engine::run_cycle`]. Consumers map these to
/// their own sink (broadcast snapshot, mpsc, etc.).
pub enum CycleEvent {
    /// Initial frame: cached cards plus pending placeholders. Emitted
    /// only when at least one card is pending.
    Skeleton(HashMap<u16, PortCard>),
    /// One card just finished probing. Boxed because `PortCard` is
    /// large (~300B) and dwarfs the other variants.
    Resolved(Box<PortCard>),
    /// All probes done. `elapsed_ms` is the wall time of the cycle.
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

    /// Empty cache — every listener reads as uncached, every card is
    /// forced through the probe path. Mirrors a fresh-start scheduler.
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

    /// Returns two listeners on ports unlikely to be bound. Probes will
    /// fail fast (ConnectionRefused on loopback is immediate), so the
    /// test stays well under one second.
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

        // Shape: 1 skeleton + N resolved + 1 done, in that order.
        assert!(events.len() >= 3, "got {} events", events.len());
        assert!(
            matches!(events.first(), Some(CycleEvent::Skeleton(_))),
            "first event must be Skeleton"
        );
        assert!(
            matches!(events.last(), Some(CycleEvent::Done { .. })),
            "last event must be Done"
        );
        // Middle events are all Resolved.
        let mid_resolved = events[1..events.len() - 1]
            .iter()
            .filter(|e| matches!(e, CycleEvent::Resolved(_)))
            .count();
        assert_eq!(mid_resolved, 2, "expected 2 Resolved events between skeleton and done");

        // Skeleton carries pending cards for both listeners.
        if let Some(CycleEvent::Skeleton(map)) = events.first() {
            assert_eq!(map.len(), 2);
            assert!(map.values().all(|c| c.is_pending()));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_cycle_skips_skeleton_when_all_cached() {
        // Cache that returns a fully-resolved card for every listener —
        // no pending placeholders, so Skeleton must not be emitted.
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
                    cmdline: None,
                    status: Some(200),
                    probed_url: Some(format!("http://127.0.0.1:{}/", l.port)),
                    probed_at_unix: Some(0),
                    elapsed_ms: Some(1),
                    error_class: None,
                    error_detail: None,
                    attempts: 1,
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

        assert!(
            !events.iter().any(|e| matches!(e, CycleEvent::Skeleton(_))),
            "no Skeleton expected when all cards are cached"
        );
        assert!(
            !events.iter().any(|e| matches!(e, CycleEvent::Resolved(_))),
            "no Resolved expected when all cards are cached (nothing to probe)"
        );
        assert!(matches!(events.last(), Some(CycleEvent::Done { .. })));
    }
}
