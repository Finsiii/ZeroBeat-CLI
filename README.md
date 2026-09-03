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

Official prebuilt targets:

- Debian 12 and 13 on x86_64 and ARM64, including matching Chromebook Linux
  Development Environments (Crostini).
- Ubuntu 24.04 on x86_64 and ARM64.
- Arch Linux on x86_64 with FFmpeg 8 or 9.
- Windows x64 (native MSVC build).

ChromeOS itself is not targeted directly; run ZeroBeat inside the Chromebook
Linux environment. macOS, 32-bit Linux, and other distributions are not
official release targets yet.

## Quick install

The installer is user-scoped, verifies the release archive and SHA-256
checksum, and never invokes `sudo`. Download it to a temporary file and run
that file; do not pipe a network response into a shell. No pager or inspection
utility is required:

```bash
# Bash or Zsh
installer=$(mktemp)
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/Finsiii/ZeroBeat-CLI/main/install.sh \
  -o "$installer" &&
  sh "$installer" &&
  rm -f "$installer"
```

```fish
# Fish
set installer (mktemp)
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/Finsiii/ZeroBeat-CLI/main/install.sh \
  -o "$installer"
and sh "$installer"
and rm -f "$installer"
```

The command does not require `less` or another pager. It needs `curl`,
`binutils` (for `readelf`), and the normal media runtime libraries. On a fresh
Debian, Ubuntu, or Chromebook Linux environment, install them once with:

```bash
sudo apt-get update
sudo apt-get install --yes curl ca-certificates binutils ffmpeg \
  libsqlite3-0 libasound2 libpulse0 libpipewire-0.3-0
```

The installer selects the archive for the detected distribution, version, and
CPU architecture. Arch Linux also selects the matching FFmpeg 8 or FFmpeg 9
build. Unsupported targets are rejected before anything is installed. The
installer requires a writable absolute `PREFIX`; by default it uses
`$HOME/.local`.

On a Chromebook, enable **Linux development environment**, open its Terminal,
and run the same command above. Debian 12/13 Crostini containers are supported
on both Intel/AMD and ARM64 Chromebooks. Existing containers may need their
normal system packages updated before installing ZeroBeat.

Add the default bin directory to Bash or Zsh and start the player:

```bash
export PATH="$HOME/.local/bin:$PATH"
zerobeat-cli
```

For Fish:

```fish
fish_add_path $HOME/.local/bin
zerobeat-cli
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

## Windows x64 install

Open PowerShell and download the installer to a temporary file before running
it. The installer downloads `zerobeat-windows-x86_64.zip` over HTTPS, verifies
its SHA-256 checksum, and installs per-user by default; no administrator
access is required:

```powershell
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$installer = Join-Path $env:TEMP 'zerobeat-install.ps1'
Invoke-WebRequest -UseBasicParsing `
  -Uri 'https://raw.githubusercontent.com/Finsiii/ZeroBeat-CLI/main/install.ps1' `
  -OutFile $installer
try {
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File $installer
} finally {
  Remove-Item -LiteralPath $installer -Force
}
```

The default prefix is `$env:LOCALAPPDATA\Programs\ZeroBeat`, and it is added
to the per-user `PATH`; open a new terminal after installation and run:

```powershell
zerobeat-cli.exe
```

Install a specific release or choose another user-writable prefix with
`-Version vX.Y.Z` and `-Prefix C:\path\to\ZeroBeat`. Use `-NoPath` to skip the
per-user `PATH` update. To uninstall, run the downloaded script with
`-Uninstall -Prefix $env:LOCALAPPDATA\Programs\ZeroBeat`; modified or
unmanifested files are never removed, and user data is preserved.

For manual verification, download the archive and its checksum from the
release page, then run:

```powershell
$version = 'v0.1.6'
$base = "https://github.com/Finsiii/ZeroBeat-CLI/releases/download/$version"
$archive = 'zerobeat-windows-x86_64.zip'
Invoke-WebRequest -UseBasicParsing "$base/$archive" -OutFile $archive
Invoke-WebRequest -UseBasicParsing "$base/$archive.sha256" -OutFile "$archive.sha256"
$expected = (Get-Content "$archive.sha256").Trim().Split()[0].ToLowerInvariant()
$actual = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLowerInvariant()
if ($expected -ne $actual) { throw 'archive SHA-256 verification failed' }
Expand-Archive -LiteralPath $archive -DestinationPath .\zerobeat -Force
```

Optionally verify GitHub build provenance with GitHub CLI:

```powershell
gh attestation verify $archive --repo Finsiii/ZeroBeat-CLI
```

## Manual release installation

Download the platform-specific archive and checksum for v0.1.6, then verify
before extracting. Choose exactly one archive name:

```bash
version=v0.1.6
base="https://github.com/Finsiii/ZeroBeat-CLI/releases/download/$version"
mkdir -p "$HOME/tmp/zerobeat-$version"
cd "$HOME/tmp/zerobeat-$version"
# Ubuntu 24.04 x86_64:
archive=zerobeat-linux-x86_64.tar.gz
# Ubuntu 24.04 ARM64:
# archive=zerobeat-linux-ubuntu24-aarch64.tar.gz
# Debian 12 x86_64 / ARM64:
# archive=zerobeat-linux-debian12-x86_64.tar.gz
# archive=zerobeat-linux-debian12-aarch64.tar.gz
# Debian 13 x86_64 / ARM64:
# archive=zerobeat-linux-debian13-x86_64.tar.gz
# archive=zerobeat-linux-debian13-aarch64.tar.gz
# Arch Linux with FFmpeg 8:
# archive=zerobeat-linux-arch-ffmpeg8-x86_64.tar.gz
# Arch Linux with FFmpeg 9:
# archive=zerobeat-linux-arch-ffmpeg9-x86_64.tar.gz
checksum="$archive.sha256"
curl --proto '=https' --tlsv1.2 -fLO "$base/$archive"
curl --proto '=https' --tlsv1.2 -fLO "$base/$checksum"
sha256sum -c "$checksum"
```

Optionally verify GitHub's build provenance with GitHub CLI:

```bash
gh attestation verify "$archive" \
  --repo Finsiii/ZeroBeat-CLI
```

```bash
tar -xzf "$archive"
install -Dm755 zerobeat-cli "$HOME/.local/bin/zerobeat-cli"
install -Dm755 zerobeatd "$HOME/.local/bin/zerobeatd"
export PATH="$HOME/.local/bin:$PATH"
zerobeat-cli
```

## Build from source

The native audio build needs a C++20 compiler, `pkg-config`, SQLite, curl,
FFmpeg (`avformat`, `avcodec`, `avutil`, and `swresample`), and an audio
backend. The repository pins Rust toolchain 1.96.0.

Debian 12/13, including Chromebook Linux environments:

```bash
sudo apt-get update
sudo apt-get install --yes \
  build-essential pkg-config curl \
  libsqlite3-dev libcurl4-openssl-dev \
  libavformat-dev libavcodec-dev libavutil-dev libswresample-dev \
  libasound2-dev libpulse-dev libpipewire-0.3-dev
```

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
install -Dm755 target/release/zerobeat-cli "$HOME/.local/bin/zerobeat-cli"
install -Dm755 target/release/zerobeatd "$HOME/.local/bin/zerobeatd"
export PATH="$HOME/.local/bin:$PATH"
zerobeat-cli
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
README. The installers verify checksums, restrict archive contents, validate
platform binaries and dependencies, reject unsafe paths, and record a manifest.

- `zerobeat-cli: command not found`: run `export PATH="$HOME/.local/bin:$PATH"`
  or invoke the absolute path under your chosen prefix.
- Native library errors: install the dependencies above, then run `ldd
  target/release/zerobeat-cli` and `ldd target/release/zerobeatd`; neither should
  report `not found`.
- No audio: check that an ALSA, PulseAudio, or PipeWire session is available
  for the current user and that output is not muted.
- Search or stream errors: confirm network access and inspect
  `ZEROBEAT_API_URL`. The daemon is started by the TUI; it should not need to
  be launched manually.
- Windows `zerobeat-cli.exe: command not found`: open a new terminal after
  installation, or invoke the executable from the selected prefix.
- Windows audio errors: confirm that a Windows audio output device is enabled
  and available to the current user.

## Benchmarking

`scripts/benchmark.sh` is read-only. It measures already-running processes via
`/proc`, never launches or signals them, and never changes playback or state.
By default it selects exactly one owned `zerobeat-cli` TUI and one owned
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

Replace the placeholders with the owned `zerobeat-cli` and `zerobeatd` PIDs.

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
