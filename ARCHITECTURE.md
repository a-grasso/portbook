# Portbook architecture

> North star: **portbook is a tool with views.** The tool discovers and
> describes processes listening on local ports. The UI and CLI are
> interchangeable surfaces over the same data. Anything visible in one
> should be derivable from the same call the other makes.

## Principles

1. **Single source of truth for "what's listening."**
   One scan primitive lives in the core; every surface (web UI, `ls`,
   `watch`, `stats`, future agent endpoints) goes through it. No
   surface re-implements enumerate / inspect / probe.

2. **Views are thin.**
   A view's job is to *render* a `Snapshot` (or its derivatives) and to
   *invoke* core operations. Views must not contain discovery logic,
   probe logic, caching, or persistence. If a view file grows business
   rules, the rules belong in the core.

3. **Surfaces are equal citizens.**
   The web UI is not the primary surface; the terminal is not the
   primary surface; an agent's HTTP/JSON view is not secondary either.
   Whichever surface the user picks, the same data is reachable.

4. **The core is offline-capable and cheap to call.**
   No surface should require a running daemon. The CLI's one-shot path
   and the scheduler's polling path are two callers of the same core.
   Daemon mode just adds caching, broadcast, and an HTTP handle on top.

5. **Privacy and redaction live below every surface.**
   Sensitive transformations (cmdline redaction, host-header guard,
   etc.) happen in the core or the API boundary, never in a view.

---

## Layer map

```
┌─────────────────────────────────────────────────────────────────┐
│  Surfaces (views)                                                │
│   • src/main.rs        — process entry, dispatch                 │
│   • src/cli.rs         — terminal renderer (`ls`, `watch`, …)    │
│   • assets/            — web UI (HTML/JS/CSS)                    │
│   • src/api.rs         — HTTP boundary (axum handlers)           │
└─────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────┐
│  Daemon layer (optional, only when serving the UI)               │
│   • src/scheduler.rs   — periodic Engine.scan, cache, broadcast  │
│   • src/state.rs       — AppState: snapshot + SSE channel        │
│   • src/lib.rs         — Router, host_guard, build_app           │
│   • src/version.rs     — VersionState + GitHub poll              │
└─────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────┐
│  Engine (the core — the "tool")                                  │
│   • src/engine.rs (TBD) — Engine: scan_all(), scan(listeners)   │
│   • src/discovery/     — enumerate listeners (lsof / ss)         │
│   • src/process/       — inspect a PID → ProcInfo                │
│   • src/probe.rs       — HTTP probe → ProbeResult                │
│   • src/redact.rs      — cmdline redaction                       │
│   • src/project.rs     — cwd → project root/name                 │
└─────────────────────────────────────────────────────────────────┘
```

## The Engine

The Engine owns the enumerator, inspector, and prober. It exposes a
small surface that every caller uses:

```rust
pub struct Engine { /* enumerator, inspector, prober */ }

impl Engine {
    pub fn new() -> Self;

    /// All current listeners on the host, after portbook's standard
    /// filters (port > 1024, not portbook itself).
    pub fn enumerate(&self) -> anyhow::Result<Vec<Listener>>;

    /// Enumerate + inspect (no probing). Used to paint a skeleton
    /// frame before slow probes complete.
    pub fn enumerate_with_procs(&self) -> anyhow::Result<Vec<(Listener, ProcInfo)>>;

    /// Probe + inspect a set of listeners in parallel. The Engine
    /// does not cache; callers layer caching on top.
    pub async fn scan(&self, listeners: Vec<Listener>) -> Vec<PortCard>;

    /// Convenience: enumerate + scan in one call.
    pub async fn scan_all(&self) -> anyhow::Result<Vec<PortCard>>;

    /// Probe pre-resolved (Listener, ProcInfo) pairs in parallel,
    /// yielding each PortCard as it lands. Use when the consumer
    /// can act on partial results.
    pub fn scan_stream<'a>(&'a self, pairs: Vec<(Listener, ProcInfo)>)
        -> impl Stream<Item = PortCard> + 'a;

    /// Run one full poll cycle (enumerate → skeleton → stream probes
    /// → done) and yield producer events. Both the daemon scheduler
    /// and the TUI poll loop go through this method, plugging in their
    /// own cache strategy via `CycleCache`.
    pub fn run_cycle<'a, C: CycleCache + 'a>(&'a self, cache: &'a mut C)
        -> impl Stream<Item = CycleEvent> + 'a;
}
```

This is the *only* place that knows how to turn "I want a snapshot of
local services" into a `Vec<PortCard>`. Optimisations (parallel probes,
TCP pre-peek, skip-list of non-HTTP ports) happen here once.

`run_cycle` is the canonical primitive for repeated polling — it owns
the skeleton-then-stream-then-final shape, parameterized by cache
strategy. `scan` and `scan_all` remain as one-shot convenience
collectors. The producer yields `Skeleton` (when any listener is
uncached), `Resolved(card)` per probe completion, and `Done { elapsed_ms }`
last; consumers map these to their sink (broadcast snapshot for the
daemon, watch channel for the TUI).

The Snapshot view contract uses `scan_elapsed_ms = None` as the "skeleton
in flight" signal and `Some(_)` as "resolved". Per-card,
`PortCard.pending: bool` is the explicit, machine-readable skeleton
flag (the historical `reason="probing…"` string is kept only for
human-facing display and pre-v0.1.7 compat).

### Who calls it

- `cli::run_ls` → `Engine::scan_all()` (or daemon's `/api/ports`).
- `cli::one_shot_scan_with_progress` → `Engine::scan_stream()` for the
  `ls` progress meter.
- `scheduler::cycle` → `Engine::run_cycle()` with a PID+command-keyed
  cache, then publish to `AppState`.
- `cli::tui` → `Engine::run_cycle()` with a port-only cache, then
  forward snapshots over a `watch` channel to the renderer.
- Future `portbook watch` → `Engine::scan_all()` in a loop, JSON to
  stdout.
- Future `portbook stats` → reads counters that the Engine emits.

### What the Engine does *not* do

- It does not cache (that's a daemon-layer concern).
- It does not persist (that's `history.rs`'s concern, see backlog B).
- It does not format for any surface.
- It does not know about HTTP routes or the SSE channel.

---

## Surface contracts

### CLI (`src/cli.rs`)

- May call `Engine` directly, or fetch `/api/ports` from a running daemon.
- Renders `Snapshot` to a tty (colored, grouped) or to stdout (JSON, TSV)
  when not a tty or when `--json` is passed.
- Must respect `--color`, `$NO_COLOR`, and `PORTBOOK_HEADLESS=1`.
- Must not import `discovery::*`, `process::*`, or `probe::*` directly
  for anything other than constructing an `Engine`.
- Multi-file feature subdirs are allowed under `src/cli/` once a feature
  exceeds ~300 LOC (`cli/tui/{mod,app,source,ui}.rs` is the precedent).
  Single-file features stay flat at `src/cli/<name>.rs`.

### TUI (`src/cli/tui/*`)

- Consumes the daemon's SSE stream when present, falls back to
  `Engine::run_cycle` polling otherwise. Source label ("daemon" vs
  "polling") is shown in the footer.
- Snapshots flow into the renderer over a `tokio::sync::watch` channel:
  the producer never blocks on a slow renderer and intermediate frames
  coalesce under load.
- `App` is a pure state machine; `ui::render` takes `&App` so the borrow
  checker enforces "view never mutates state". Handler logic lives on
  `App`, drawing logic in `ui.rs`.

### Web UI (`assets/*`)

- Consumes `/api/ports`, `/api/stream`, `/api/version`, `/api/stats` (TBD).
- No business logic beyond filtering/sorting the snapshot it receives.
- All rendering decisions (live/all tab, kind badges) operate on the
  shape `PortCard` produces.

### HTTP API (`src/api.rs`)

- The boundary where redaction is *guaranteed* to have happened.
- Stable JSON schema — UI, CLI-via-daemon, and external agents all
  consume it. Schema changes are a documented breaking change.

### Daemon (`src/scheduler.rs`, `src/state.rs`)

- Adds three things the Engine doesn't provide: periodic polling,
  inter-cycle caching, and a broadcast channel for SSE.
- These three are independent of one another and could in principle
  be opted out (e.g. a "scan once and serve" mode for short-lived
  agent inspection).

---

## Decision matrix: where does this new feature go?

| Question                                         | Goes in                |
| ------------------------------------------------ | ---------------------- |
| New way to discover listeners (eg Windows port)  | `src/discovery/`       |
| New thing to learn about a PID                   | `src/process/`         |
| New probe behavior (TCP peek, HTTPS, headers)    | `src/probe.rs`         |
| New transformation of `PortCard`                 | Engine (or sibling)    |
| New surface (terminal command, JSON endpoint)    | view layer only        |
| New persistence (history, stats over time)       | new core module        |
| Cross-cycle reasoning (flap detection, latency)  | daemon layer           |

If you're tempted to put discovery / probe / redact logic in
`cli.rs` or `assets/`, stop — it goes in the Engine.

---

## Evolution roadmap (anchored to BACKLOG.md)

- **Pretty `ls` (item A):** view-layer change only. Engine untouched.
- **Last-seen history (item B):** new core module `src/history.rs`,
  consumed by both views via Engine or AppState.
- **Self-telemetry (item C):** counters live in Engine; `/api/stats`
  and `portbook stats` are two views over the same data.
- **Headless / agent mode (item D):** view-layer flags
  (`--json`, `--headless`, `watch`); the Engine already supports it.

If a new feature can't be expressed as "engine work + thin view," that's
a signal the architecture needs a new layer, not a workaround.
