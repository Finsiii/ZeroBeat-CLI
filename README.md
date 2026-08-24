# ZeroBeat CLI

A modern, Linux-first ZeroBeat player for the terminal. It combines a responsive TUI with a small background daemon and native C++ audio engine.

## Features

- Home, search, library, downloads, queue, lyrics, and settings
- fast streamed playback with prebuffering, seek, volume, and telemetry
- automatic queue progression and configurable equal-power crossfade
- synced lyrics with offline cache
- local likes, recent plays, settings, and private offline downloads
- guest mode with no account required
- signed device identity and API requests without embedded static secrets

ZeroBeat Native DJ is currently exclusive to the Android app. Linux uses the lightweight native playback engine.

## Build and install

The build needs Rust 1.88, a C++20 compiler, `pkg-config`, SQLite, libcurl, and the FFmpeg development libraries for `avformat`, `avcodec`, `avutil`, and `swresample`.

```bash
cargo build --workspace --release
sudo install -Dm755 target/release/zerobeat /usr/local/bin/zerobeat
sudo install -Dm755 target/release/zerobeatd /usr/local/bin/zerobeatd
zerobeat
```

The TUI starts the per-user daemon automatically. User state is stored under `$XDG_DATA_HOME/zerobeat` and runtime IPC under `$XDG_RUNTIME_DIR/zerobeat`, both with private permissions.

## Controls

`1`–`5` switch pages, `/` focuses search, `Enter` plays, `a` queues, `d` downloads, `l` likes, `y` opens lyrics, and `u` opens the queue. Use `Space` to pause, `n` for next, arrow keys to seek, `-`/`+` for volume, `[`/`]` for crossfade, and `q` to quit.

## License

Mozilla Public License 2.0.
