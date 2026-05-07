//! `portbook tui` — interactive terminal view over the same Engine that
//! powers `ls` and the web UI. Connects to a running daemon's SSE stream
//! when available, otherwise polls the local Engine on a 3s tick.
//!
//! Layout decision: expand-in-place rows (k9s / `gh` style) — works at
//! any terminal width. See `app::App` for state, `ui::render` for the
//! pure render function. No engine logic lives here.

mod app;
mod source;
mod ui;

use app::App;
use crossterm::event::{Event, EventStream, KeyEventKind};
use crossterm::{execute, terminal};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, IsTerminal, Write};
use std::time::Duration;

/// Returned by `portbook tui` when stdout isn't a tty so the caller
/// can distinguish "wrong environment" from a generic runtime failure
/// (which exits 1 via `anyhow`). Documented in the README exit-codes
/// table. Coexists with `EXIT_PORT_NOT_FOUND = 3` from `explain`.
pub const EXIT_NOT_A_TTY: i32 = 4;

pub async fn run_tui() -> anyhow::Result<i32> {
    if !io::stdout().is_terminal() {
        eprintln!(
            "portbook tui requires an interactive terminal.\n\
             Try `portbook ls` for piping or scripting."
        );
        return Ok(EXIT_NOT_A_TTY);
    }

    let source_label = if source::daemon_alive().await {
        "daemon"
    } else {
        "polling"
    };
    let mut app = App::new(source_label);

    // `watch` channel: producer never blocks on a slow renderer, and
    // the TUI only ever cares about the latest snapshot. Initial value
    // is an empty snapshot so the first frame draws before any source
    // event arrives.
    let initial = crate::state::Snapshot { ports: Vec::new(), scan_elapsed_ms: None };
    let (snap_tx, mut snap_rx) = tokio::sync::watch::channel(initial);
    source::spawn(snap_tx);

    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;

    let mut events = EventStream::new();
    let res = run_loop(&mut term, &mut app, &mut events, &mut snap_rx).await;

    terminal::disable_raw_mode().ok();
    execute!(term.backend_mut(), terminal::LeaveAlternateScreen).ok();
    term.show_cursor().ok();
    let _ = io::stdout().flush();

    res.map(|_| 0)
}

async fn run_loop<B: ratatui::backend::Backend>(
    term: &mut Terminal<B>,
    app: &mut App,
    events: &mut EventStream,
    snap_rx: &mut tokio::sync::watch::Receiver<crate::state::Snapshot>,
) -> anyhow::Result<()> {
    term.draw(|f| ui::render(f, app))?;
    let mut redraw_tick = tokio::time::interval(Duration::from_millis(500));
    redraw_tick.tick().await; // discard first immediate tick

    loop {
        tokio::select! {
            Some(ev) = events.next() => {
                match ev? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => {
                        app.handle_key(k);
                        if app.should_quit { break; }
                    }
                    Event::Resize(_, _) => {}
                    _ => continue,
                }
            }
            res = snap_rx.changed() => {
                if res.is_err() {
                    // Source task ended (channel closed). Keep
                    // rendering — the TUI remains usable showing the
                    // last seen snapshot until the user quits.
                    continue;
                }
                let snap = snap_rx.borrow_and_update().clone();
                app.ingest(snap);
            }
            _ = redraw_tick.tick() => {
                // Periodic redraw lets transient status messages decay
                // without depending on a key/snap arriving.
            }
        }
        term.draw(|f| ui::render(f, app))?;
    }
    Ok(())
}

/// Spawn the platform's URL opener. macOS uses `open`, Windows uses
/// `cmd /c start`, all other Unix targets use `xdg-open`. Errors are
/// swallowed — the TUI surfaces success/failure via a status line
/// message, not a panic.
pub(crate) fn open_in_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        // The empty title arg is intentional: `cmd /c start` treats the
        // first quoted argument as the window title, so we pass "" and
        // then the URL. Avoids interpreting URL fragments as a title.
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}
