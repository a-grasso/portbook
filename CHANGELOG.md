# Changelog

All notable changes to portbook are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/) and this
project adheres to [Semantic Versioning](https://semver.org/).

## [0.2.1] - 2026-05-08

### Bug Fixes

- Handle redirect chains as HTTP, not "not HTTP"

### Features

- Show shrunken cwd on TUI/ls/web cards

### Refactor

- Trim restate-the-code comments

## [0.2.0] - 2026-05-07

### Bug Fixes

- Only render pending placeholders for first-time-seen ports
- SSE parser cap buffer, decode at event boundary
- Give EXIT_NOT_A_TTY a dedicated exit code (4)
- Handle Windows in open_in_browser
- Wire popover title to aria-labelledby
- Preserve cached cards across re-probe cycles

### Chores

- V0.2.0

### Documentation

- Document run_cycle, TUI surface, snapshot signals

### Features

- Add `portbook tui` interactive terminal app
- Show port diagnostics in a floating popover instead of inline
- Progressive skeleton rendering + scan timing telemetry
- Stream probe completions; ls progress meter; sticky tabs
- Add explicit pending bool to PortCard JSON

### Refactor

- Extract Engine::run_cycle, eliminate map clones, type-safe outcomes
- Use tokio::time::timeout for daemon skeleton poll
- Narrow visibility on internal types
- Rename scan_streaming_with_procs → scan_stream
- Swap mpsc for watch on the snapshot channel

## [0.1.6] - 2026-05-07

### Build

- Add `just release` recipe for tagged releases

### Chores

- V0.1.6

### Documentation

- Require annotated tags in release flow

### Features

- Per-port diagnostics, explain subcommand, retry + timeout

## [0.1.5] - 2026-05-06

### Build

- Add cliff.toml for git-cliff changelog generation

### Chores

- V0.1.5

### Documentation

- Add ARCHITECTURE.md — north star for portbook layering
- Document CLI usage, JSON schema, exit codes
- Surface CLI in lede and Features
- Add CONTRIBUTING.md, AGENTS.md, CLAUDE.md for contributor + agent guidance

### Features

- Add subcommands and in-UI version/update display
- Pretty `portbook ls` — grouped, colored, width-aware
- Add `portbook ls --json` for machine-readable output
- Add --color=auto|always|never (replaces --no-color)
- Add -v/-vv verbosity flag wired to tracing
- Add `portbook completions <shell>` subcommand
- Add `portbook watch` — stream snapshots on an interval

### Refactor

- Extract Engine as single core for discovery + probe
- Switch Style helpers to anstyle for typed ANSI
- Split cli.rs into focused submodules
- Rename `ui` subcommand to `serve` (alias kept)

## [0.1.0] - 2026-05-06


