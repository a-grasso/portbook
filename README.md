# Portbook

A local web dashboard that discovers, labels, and links every dev service running on a non-standard localhost port.

Runs on **http://localhost:7777**.

## Build & Run

```sh
cargo run --release
```

Then open http://localhost:7777 in your browser.

## Platform support

macOS and Linux. Requires `lsof` (macOS) or `ss` (Linux) on `$PATH`.
