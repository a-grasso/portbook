# Portbook

[![Release](https://img.shields.io/github/v/release/a-grasso/portbook?display_name=tag)](https://github.com/a-grasso/portbook/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey)](#platform-support)

You've got six dev servers running on six random ports and no idea which is which. Portbook finds them, labels them, and gives you one page to click through.

Runs on **http://localhost:7777**.

## Why

- **Stop guessing what's on `:5173`.** Portbook reads each process's `cwd` and the page's `<title>` so every card is recognizable at a glance.
- **See what's broken without checking.** Ports are classified as **live**, **error**, or **dead** — a crashed `next dev` shows up red instead of disappearing.
- **Zero config.** Start it, leave it open. No registration, no service files, no `ports.json` to maintain.

## Features

- Auto-discovers HTTP servers on every non-standard localhost port
- Labels each card with project name (detected from process `cwd` markers) and page title
- Classifies ports as **live** / **error** / **dead** with visible reasons
- Live updates via Server-Sent Events — no polling, no refresh
- Single static binary, ~5 MB, no runtime dependencies beyond `lsof` (macOS) or `ss` (Linux)
- Tabbed UI: focus on **Live** services, or see the full inventory under **All**

## Install

**Homebrew (macOS / Linuxbrew):**

```sh
brew install a-grasso/tap/portbook
```

**Shell installer:**

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/a-grasso/portbook/releases/latest/download/portbook-installer.sh | sh
```

Then run `portbook` and open http://localhost:7777.

## Command-line usage

```sh
portbook                       # default: serve the web UI on http://localhost:7777
portbook serve                 # explicit
portbook ls                    # one-shot terminal list (grouped, colored)
portbook ls --live             # only live services
portbook ls --all              # also show dead ports (collapsed by default)
portbook ls --json             # one JSON snapshot, machine-readable
portbook watch --json          # stream JSON snapshots on a 3s interval
portbook watch --interval 5    # custom polling interval (seconds)
portbook completions zsh       # shell completion script (bash, zsh, fish, …)
portbook --version
```

Global flags:

- `-v` / `-vv` — increase log verbosity (debug / trace). Overrides `RUST_LOG`.
- `--color=auto|always|never` (on `ls` and `watch`) — color output. Defaults to `auto`; respects the `NO_COLOR` env var.

Environment:

- `PORTBOOK_DEFAULT=ls` — change the no-arg default to `ls` instead of `serve`.
- `PORTBOOK_NO_OPEN=1` — don't auto-open the browser when starting `serve`.
- `NO_COLOR=1` — disable ANSI colors universally.

## Agent / script integration

`portbook ls --json` and `portbook watch --json` emit a stable JSON
schema designed for piping into `jq` and friends. The same shape is
served from `/api/ports` when the daemon is running.

```sh
portbook ls --json | jq '.ports[] | select(.kind=="live") | .url'
portbook watch --json | jq -c '.ports | map(.port)'
```

### Snapshot schema

```jsonc
{
  "ports": [
    {
      "port":          8421,                  // u16, listening port
      "pid":           88330,                 // u32, owning process
      "command":       "python3.12",          // short process name
      "url":           "http://localhost:8421",
      "kind":          "live",                // "live" | "error" | "dead"
      "reason":        null,                  // populated on error/dead (e.g. "HTTP 404", "timeout")
      "title":         "Test Service Alpha",  // <title> from probed page (live only)
      "description":   null,                  // meta-description if present
      "project_root":  "/path/to/project",    // detected from process cwd
      "project_name":  "project-name",        // basename of project_root
      "cwd":           "/path/to/project",
      "cmdline":       "python3 -m http.server 8421",  // redacted (token=…, key=…, etc.)
      "status":        200                    // HTTP status when probed
    }
  ]
}
```

`watch --json` only emits when the snapshot has changed. Identical
consecutive snapshots are suppressed so consumers see real events.

Sensitive substrings in `cmdline` (tokens, passwords, URLs with
userinfo) are redacted at the API boundary — the same redaction is
applied for `/api/ports`, `/api/stream`, `ls`, and `watch`.

### Exit codes

- `0` — success
- `1` — runtime error (scan failed, daemon refused connection, etc.)
- `2` — CLI misuse (unknown flag, bad value); emitted by clap

## Platform support

macOS and Linux (x86_64 and arm64). Requires `lsof` (macOS) or `ss` (Linux) on `$PATH` — both are already installed by default on essentially every modern install.

## Build from source

```sh
cargo run --release
```
