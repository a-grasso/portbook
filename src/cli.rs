use crate::probe::ProbeKind;
use crate::state::{PortCard, Snapshot};
use crate::{BIND_ADDR, SELF_PORT};

/// Run the `ls` command: print a table of port cards. Tries the running daemon
/// first, falls back to a fresh one-shot scan if no daemon is reachable.
pub async fn run_ls() -> anyhow::Result<()> {
    let snapshot = match fetch_from_daemon().await {
        Some(s) => s,
        None => one_shot_scan().await?,
    };
    print_snapshot(&snapshot);
    Ok(())
}

async fn fetch_from_daemon() -> Option<Snapshot> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .ok()?;
    let url = format!("http://{BIND_ADDR}/api/ports");
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<Snapshot>().await.ok()
}

async fn one_shot_scan() -> anyhow::Result<Snapshot> {
    let enumerator = crate::discovery::default();
    let inspector = crate::process::default();
    let prober = crate::probe::Prober::new();

    let listeners: Vec<_> = enumerator
        .list()?
        .into_iter()
        .filter(|l| l.port > 1024 && l.port != SELF_PORT)
        .collect();

    let mut ports: Vec<PortCard> = Vec::with_capacity(listeners.len());
    for l in listeners {
        let proc = if l.pid == 0 {
            crate::process::ProcInfo::default()
        } else {
            inspector.inspect(l.pid)
        };
        let probe = prober.probe(l.port).await;
        ports.push(PortCard::build(l.port, l.pid, l.command.clone(), &proc, &probe));
    }
    ports.sort_by_key(|c| c.port);
    Ok(Snapshot { ports })
}

fn print_snapshot(snap: &Snapshot) {
    if snap.ports.is_empty() {
        println!("No listening ports detected.");
        return;
    }

    let live = snap.ports.iter().filter(|c| c.kind == ProbeKind::Live).count();
    println!("{} live · {} total\n", live, snap.ports.len());

    // columns: PORT  KIND  PROJECT  TITLE  CMD
    let rows: Vec<[String; 5]> = snap
        .ports
        .iter()
        .map(|c| {
            [
                format!(":{}", c.port),
                kind_label(c).to_string(),
                c.project_name.clone().unwrap_or_default(),
                c.title.clone().unwrap_or_default(),
                c.cmdline.clone().unwrap_or_else(|| c.command.clone()),
            ]
        })
        .collect();

    let headers = ["PORT", "KIND", "PROJECT", "TITLE", "CMD"];
    let mut widths = headers.map(|h| h.len());
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            // last column doesn't need padding — let it overflow
            if i < widths.len() - 1 {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
    }

    print_row(&headers.map(String::from), &widths);
    println!();
    for row in &rows {
        print_row(row, &widths);
    }
}

fn kind_label(c: &PortCard) -> &str {
    match c.kind {
        ProbeKind::Live => "live",
        ProbeKind::Error => c.reason.as_deref().unwrap_or("error"),
        ProbeKind::Dead => c.reason.as_deref().unwrap_or("dead"),
    }
}

fn print_row(row: &[String; 5], widths: &[usize; 5]) {
    let last = row.len() - 1;
    for (i, cell) in row.iter().enumerate() {
        if i == last {
            println!("{}", truncate(cell, 80));
        } else {
            print!("{:<width$}  ", cell, width = widths[i]);
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        let mut out: String = chars.into_iter().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
