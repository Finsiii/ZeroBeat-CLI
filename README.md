# ZeroBeat CLI

An interactive, Linux-first ZeroBeat client built for a rich terminal experience with a small memory footprint.

## Current status

The first development milestone includes:

- responsive full-screen TUI
- Guest-first navigation with persistent search state
- per-user daemon over a private Unix socket
- local SQLite library, play history, and download state
- lightweight audio queue and equal-power crossfade state machine
- catalog and stream provider boundaries without embedded API secrets

ZeroBeat Native DJ remains exclusive to the Android app. The Linux client uses the lightweight playback engine.

## Build

```bash
cargo build --workspace --release
./target/release/zerobeat
```

Rust 1.96 and SQLite development libraries are required.

## License

Mozilla Public License 2.0.
