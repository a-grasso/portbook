# Changelog

All notable changes to portbook are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/) and this
project adheres to [Semantic Versioning](https://semver.org/).

## [0.1.6] - 2026-05-07

### Build

- Add `just release` recipe for tagged releases

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


