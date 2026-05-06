//! The core "tool" — discovery + inspection + probing of local listeners.
//!
//! Every surface (web UI, CLI, agent endpoints) goes through `Engine`.
//! See `ARCHITECTURE.md` for the layering rules.

use crate::SELF_PORT;
use crate::discovery::{Listener, PortEnumerator};
use crate::probe::Prober;
use crate::process::{ProcInfo, ProcessInspector};
use crate::state::PortCard;

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
