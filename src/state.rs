use crate::probe::{ProbeError, ProbeKind, ProbeResult};
use crate::process::ProcInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortCard {
    pub port: u16,
    pub pid: u32,
    pub command: String,
    pub url: String,
    pub kind: ProbeKind,
    pub reason: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub project_root: Option<String>,
    pub project_name: Option<String>,
    pub cwd: Option<String>,
    pub cmdline: Option<String>,
    pub status: Option<u16>,
    pub probed_url: Option<String>,
    pub probed_at_unix: Option<u64>,
    pub elapsed_ms: Option<u32>,
    pub error_class: Option<ProbeError>,
    pub error_detail: Option<String>,
    pub attempts: u8,
}

impl PortCard {
    pub fn build(port: u16, pid: u32, command: String, proc: &ProcInfo, probe: &ProbeResult) -> Self {
        let project_root = proc.cwd.as_deref().and_then(crate::project::detect_root);
        let project_name = project_root.as_deref().map(crate::project::folder_name);
        Self {
            port,
            pid,
            command,
            url: format!("http://localhost:{port}"),
            kind: probe.kind,
            reason: probe.reason.clone(),
            title: probe.title.clone(),
            description: probe.description.clone(),
            project_root,
            project_name,
            cwd: proc.cwd.clone(),
            cmdline: proc.cmdline.as_deref().map(crate::redact::redact_cmdline),
            status: probe.status,
            probed_url: Some(probe.probed_url.clone()),
            probed_at_unix: Some(probe.probed_at_unix),
            elapsed_ms: Some(probe.elapsed_ms),
            error_class: probe.error_class,
            error_detail: probe.error_detail.clone(),
            attempts: probe.attempts,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<RwLock<HashMap<u16, PortCard>>>,
    tx: broadcast::Sender<Snapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub ports: Vec<PortCard>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(16);
        Self { inner: Arc::new(RwLock::new(HashMap::new())), tx }
    }

    pub async fn snapshot(&self) -> Snapshot {
        let map = self.inner.read().await;
        let mut ports: Vec<PortCard> = map.values().cloned().collect();
        ports.sort_by_key(|c| c.port);
        Snapshot { ports }
    }

    pub async fn replace(&self, new_map: HashMap<u16, PortCard>) {
        {
            let mut w = self.inner.write().await;
            *w = new_map;
        }
        let snap = self.snapshot().await;
        let _ = self.tx.send(snap);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Snapshot> {
        self.tx.subscribe()
    }
}
