# ZeroBeat CLI

ZeroBeat CLI is a Linux-first music player for the terminal. It combines a
TUI, a small per-user daemon, native audio playback, and a local library.

## Features

- Home, search, library, downloads, queue, lyrics, and settings views.
- Streamed playback with prebuffering, seek, volume, mute, shuffle, repeat,
  and equal-power crossfade.
- Radio queue generation, likes, recent plays, lyrics cache, and offline
  downloads.
- Guest-first local use; account login and sync are not included in this
  release.
- Per-device signed API requests without embedded static API secrets.

ZeroBeat Native DJ remains exclusive to the Android app. The Linux CLI uses
the lightweight native playback and crossfade engine.

The official prebuilt target is Ubuntu 24.04 Linux x86_64. Source builds are
verified on Arch Linux x86_64. ARM, macOS, Windows, and other targets are not
official release targets.

## Quick install

The installer is user-scoped, verifies the release archive and SHA-256
checksum, and never invokes `sudo`. Download it to a file, inspect it, and
then run that file; do not pipe a network response into a shell:

```bash
installer=$(mktemp)
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/Finsiii/ZeroBeat-CLI/main/install.sh \
  -o "$installer"
less "$installer"
sh "$installer"
rm "$installer"
```

The installer needs `curl`, `sha256sum`, `tar`, `mktemp`, `install`, and the
standard POSIX tools. It also uses `readelf` from `binutils` and `ldconfig`
from the system glibc package to validate the two ELF binaries. On a minimal
system, install those packages before running it. The installer supports
Linux x86_64 only and requires a writable absolute `PREFIX`; by default it
uses `$HOME/.local`.

Add the default bin directory to the current shell and start the player:

```bash
export PATH="$HOME/.local/bin:$PATH"
zerobeat
```

The TUI starts `zerobeatd` automatically for the current user. For a specific
release, set `VERSION=vX.Y.Z`; for another destination, set an absolute
`PREFIX`, for example `PREFIX="$HOME/.local"`.

An existing installation created by this installer can be updated safely:
the manifest and both executable hashes are checked before replacement, and a
failed pair update is rolled back. An unmanifested existing binary is never
overwritten. To uninstall, run:

```bash
PREFIX="$HOME/.local" sh ./install.sh --uninstall
```

Uninstall removes only the two verified executables and the install manifest;
it preserves the database, downloads, identity, and other user data. It
refuses to remove files that are missing, modified, unowned, or symlinks.

## Manual release installation

Download the archive and checksum for a release, then verify before
extracting:

```bash
version=v0.1.0
base="https://github.com/Finsiii/ZeroBeat-CLI/releases/download/$version"
mkdir -p "$HOME/tmp/zerobeat-$version"
cd "$HOME/tmp/zerobeat-$version"
curl --proto '=https' --tlsv1.2 -fLO "$base/zerobeat-linux-x86_64.tar.gz"
curl --proto '=https' --tlsv1.2 -fLO "$base/zerobeat-linux-x86_64.tar.gz.sha256"
sha256sum -c zerobeat-linux-x86_64.tar.gz.sha256
```

Optionally verify GitHub's build provenance with GitHub CLI:

```bash
gh attestation verify zerobeat-linux-x86_64.tar.gz \
  --repo Finsiii/ZeroBeat-CLI
```

```bash
tar -xzf zerobeat-linux-x86_64.tar.gz
install -Dm755 zerobeat "$HOME/.local/bin/zerobeat"
install -Dm755 zerobeatd "$HOME/.local/bin/zerobeatd"
export PATH="$HOME/.local/bin:$PATH"
zerobeat
```

## Build from source

The native audio build needs a C++20 compiler, `pkg-config`, SQLite, curl,
FFmpeg (`avformat`, `avcodec`, `avutil`, and `swresample`), and an audio
backend. The repository pins Rust toolchain 1.96.0.

Ubuntu 24.04:

```bash
sudo apt-get update
sudo apt-get install --yes \
  build-essential pkg-config curl \
  libsqlite3-dev libcurl4-openssl-dev \
  libavformat-dev libavcodec-dev libavutil-dev libswresample-dev \
  libasound2-dev libpulse-dev libpipewire-0.3-dev
```

Arch Linux:

```bash
sudo pacman -S --needed base-devel pkgconf curl sqlite ffmpeg \
  alsa-lib libpulse pipewire
```

Build and install both release siblings from the locked workspace:

```bash
rustup toolchain install 1.96.0 --profile minimal
rustup component add --toolchain 1.96.0 rustfmt clippy
rustup default 1.96.0
cargo build --workspace --release --locked
install -Dm755 target/release/zerobeat "$HOME/.local/bin/zerobeat"
install -Dm755 target/release/zerobeatd "$HOME/.local/bin/zerobeatd"
export PATH="$HOME/.local/bin:$PATH"
zerobeat
```

## Controls

`1` Library · `2` Recently played · `3` Downloads · `4` Home · `5` Search ·
`6` Queue · `7` Lyrics · `8` Settings. `/` focuses search. In a list,
`↑`/`↓` or `j`/`k` select; `Enter` plays; `a` queues; `l` likes; and `d`
downloads.

`Space` pauses/resumes, `p`/`n` go previous/next, `s` toggles shuffle, `r`
cycles repeat, `m` toggles mute, `-`/`+` change volume, and `←`/`→` seek.
`y` opens lyrics, `u` opens the queue, `x` clears the queue, and `q` quits.
In Settings, press the literal `[` to decrease crossfade and literal `]` to
increase it.

## Guest mode, network, and local state

The first run is Guest mode: playback, search, library, downloads, and lyrics
are available without an account. Catalog search and stream resolution use
the configured ZeroBeat API. The default endpoint is
`https://api.zerobits.tech/music`; override it with `ZEROBEAT_API_URL` when
needed. Uncached search and streaming need network access; downloaded tracks
and cached local data remain available according to their stored state.

Data is stored at `$XDG_DATA_HOME/zerobeat`, or
`$HOME/.local/share/zerobeat` when `XDG_DATA_HOME` is unset. This includes the
local database, downloads, lyrics cache, and `device.identity`. The daemon
socket is under `$XDG_RUNTIME_DIR/zerobeat`; without that variable the fallback
is `/tmp/zerobeat-<uid>`. Data and runtime directories are private and checked
for ownership and symlinks.

## Security and troubleshooting

The client creates a per-device signing identity with private file permissions;
API requests are signed and no API secret is embedded in the binaries or
README. The installer verifies checksums, restricts the archive contents,
validates both ELF dependencies, rejects unsafe symlink paths, and records an
ownership-checked manifest.

- `zerobeat: command not found`: run `export PATH="$HOME/.local/bin:$PATH"`
  or invoke the absolute path under your chosen prefix.
- Native library errors: install the dependencies above, then run `ldd
  target/release/zerobeat` and `ldd target/release/zerobeatd`; neither should
  report `not found`.
- No audio: check that an ALSA, PulseAudio, or PipeWire session is available
  for the current user and that output is not muted.
- Search or stream errors: confirm network access and inspect
  `ZEROBEAT_API_URL`. The daemon is started by the TUI; it should not need to
  be launched manually.

## Benchmarking

`scripts/benchmark.sh` is read-only. It measures already-running processes via
`/proc`, never launches or signals them, and never changes playback or state.
By default it selects exactly one owned `zerobeat` TUI and one owned
`zerobeatd`. If multiple daemons exist, it selects only the daemon whose
parent is that TUI; ambiguous or missing pairs fail with an explicit-PID
instruction. Use exactly two `--pid PID` options to select one process of
each role and avoid stale-process ambiguity.

Idle measurement: leave the player open with no track playing.

```bash
scripts/benchmark.sh --duration 30 --interval 1
```

Muted playback measurement: start a track, press `m`, and run this in another
terminal:

```bash
scripts/benchmark.sh --duration 60 --interval 1
```

Explicit pair selection:

```bash
tui_pid="<TUI_PID>"
daemon_pid="<DAEMON_PID>"
scripts/benchmark.sh --duration 60 --interval 1 \
  --pid "$tui_pid" \
  --pid "$daemon_pid"
```

Replace the placeholders with the owned `zerobeat` and `zerobeatd` PIDs.

The report includes distro, kernel, CPU, PIDs, samples, combined mean/peak
RSS, overall mean CPU, and peak interval CPU. CPU uses `100% = one core`.

Reference measurement (2026-08-25): release binaries built with
`cargo build --workspace --release --locked`, a fresh state in temporary XDG
data/runtime directories, and an 80x24 terminal on Arch Linux, kernel
`7.0.11-zen1-1-zen`, Intel i5-12450H (12 online CPUs). The figures below are a
single-machine reference, not a guarantee; hardware, terminal, audio backend,
and network conditions change results. The muted-playback run selected
“Lantas” by Juicy Luicy from network search, muted before play, and began
sampling after `PLAYING` while network streaming/cache was active.

| Scenario (requested / actual) | Combined mean RSS | Combined peak RSS | Overall mean CPU | Peak interval CPU |
| --- | ---: | ---: | ---: | ---: |
| Idle (30 s / 31.030 s) | 37.2 MiB | 37.2 MiB | 1.7% | 2.9% |
| Muted playback (60 s / 61.830 s) | 49.9 MiB | 59.2 MiB | 3.2% | 6.8% |

## License

Mozilla Public License 2.0. See [LICENSE](LICENSE).
