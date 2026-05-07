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

pub struct Engine {
    enumerator: Box<dyn PortEnumerator>,
    inspector: Box<dyn ProcessInspector>,
    prober: Prober,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            enumerator: crate::discovery::default(),
            inspector: crate::process::default(),
            prober: Prober::new(),
        }
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
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
