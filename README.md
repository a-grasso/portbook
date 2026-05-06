# Portbook

A local web dashboard that discovers, labels, and links every dev service running on a non-standard localhost port.

Runs on **http://localhost:7777**.

## Install

Prebuilt binaries (macOS + Linux, x86_64 + arm64) are published on each release.

**Homebrew (macOS / Linuxbrew):**

```sh
brew install a-grasso/tap/portbook
```

**Shell installer:**

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/a-grasso/portbook/releases/latest/download/portbook-installer.sh | sh
```

Then run `portbook` and open http://localhost:7777.

## Build from source

```sh
cargo run --release
```

## Platform support

macOS and Linux. Requires `lsof` (macOS) or `ss` (Linux) on `$PATH`.
