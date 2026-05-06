//! Terminal view over the Engine. View-only — no discovery / probe logic
//! here, see `ARCHITECTURE.md`.

mod render;
mod style;
mod width;

use crate::BIND_ADDR;
use crate::engine::Engine;
use crate::state::Snapshot;
use render::render;
use style::Style;
use width::term_width;

pub use style::ColorChoice;

#[derive(Default, Debug, Clone, Copy)]
pub struct LsOpts {
    pub all: bool,
    pub live: bool,
    pub color: ColorChoice,
    pub json: bool,
}

pub async fn run_ls(opts: LsOpts) -> anyhow::Result<()> {
    let snapshot = match fetch_from_daemon().await {
        Some(s) => s,
        None => one_shot_scan().await?,
    };
    let style = Style::resolve(opts.color);
    let width = term_width();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    render(&mut out, &snapshot, opts, &style, width)?;
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
    let ports = Engine::new().scan_all().await?;
    Ok(Snapshot { ports })
}
