//! `portbook watch` — emit snapshots on an interval. Polls the daemon
//! when one is up; otherwise scans locally each tick. Pure render
//! logic stays in `render::render`.

use super::render::render;
use super::style::Style;
use super::width::term_width;
use super::{ColorChoice, Snapshot, fetch_from_daemon, one_shot_scan};
use crate::cli::LsOpts;
use std::time::Duration;
use tokio::time::interval;

#[derive(Default, Debug, Clone, Copy)]
pub struct WatchOpts {
    pub json: bool,
    pub color: ColorChoice,
    pub interval_secs: u64,
}

pub async fn run_watch(opts: WatchOpts) -> anyhow::Result<()> {
    let interval_secs = opts.interval_secs.max(1);
    let mut tick = interval(Duration::from_secs(interval_secs));
    let style = Style::resolve(opts.color);
    let width = term_width();

    let render_opts = LsOpts {
        json: opts.json,
        all: opts.json, // when streaming JSON, never collapse
        live: false,
        color: opts.color,
    };

    let mut last_signature: Option<String> = None;

    loop {
        tick.tick().await;
        let snapshot = match fetch_from_daemon().await {
            Some(s) => s,
            None => match one_shot_scan().await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("scan failed: {e:#}");
                    continue;
                }
            },
        };

        // In JSON mode, skip identical snapshots so consumers only see
        // real change events. In human mode, always re-render so the
        // user sees liveness.
        if opts.json {
            let sig = snapshot_signature(&snapshot);
            if last_signature.as_ref() == Some(&sig) {
                continue;
            }
            last_signature = Some(sig);
        }

        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        render(&mut out, &snapshot, render_opts, &style, width)?;
    }
}

/// Stable signature of a snapshot for change detection. Two snapshots
/// produce the same signature iff their visible content is identical.
pub(super) fn snapshot_signature(snap: &Snapshot) -> String {
    serde_json::to_string(snap).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{ProbeKind, ProbeResult};
    use crate::process::ProcInfo;
    use crate::state::PortCard;

    fn one_port_snapshot(port: u16, title: &str) -> Snapshot {
        let probe = ProbeResult {
            kind: ProbeKind::Live,
            status: Some(200),
            title: Some(title.into()),
            description: None,
            reason: None,
        };
        let proc = ProcInfo { cwd: None, cmdline: Some("x".into()) };
        Snapshot {
            ports: vec![PortCard::build(port, 1, "x".into(), &proc, &probe)],
        }
    }

    #[test]
    fn signature_is_stable_for_identical_snapshots() {
        let a = one_port_snapshot(8000, "Hello");
        let b = one_port_snapshot(8000, "Hello");
        assert_eq!(snapshot_signature(&a), snapshot_signature(&b));
    }

    #[test]
    fn signature_changes_when_title_changes() {
        let a = one_port_snapshot(8000, "Hello");
        let b = one_port_snapshot(8000, "World");
        assert_ne!(snapshot_signature(&a), snapshot_signature(&b));
    }

    #[test]
    fn signature_changes_when_port_added() {
        let a = Snapshot { ports: vec![] };
        let b = one_port_snapshot(8000, "Hi");
        assert_ne!(snapshot_signature(&a), snapshot_signature(&b));
    }
}
