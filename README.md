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

## Platform support

macOS and Linux (x86_64 and arm64). Requires `lsof` (macOS) or `ss` (Linux) on `$PATH` — both are already installed by default on essentially every modern install.

## Build from source

```sh
cargo run --release
```
