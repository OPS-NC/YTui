# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Read AGENTS.md first

This repo has a detailed [AGENTS.md](AGENTS.md) written for AI coding agents — it is the primary source of
truth for architecture, hard rules, and conventions here. Read it in full before making changes. Highlights:

- **Prime directive: minimize RAM/CPU/battery.** This app exists specifically to be lighter than a browser
  tab playing YouTube; it was rewritten from Python/Textual to Rust for that reason. Weigh every change
  against that; a change that makes the UI nicer but the process fatter is a regression.
- **Never run the app or hit the network to test.** No `yt-dlp` calls, no playback, no `cargo run`.
  `cargo build`, `cargo clippy` and offline `cargo test` are fine. Hand hands-on testing to the user with
  exact steps to try.
- **No in-process yt-dlp, HTTP or TLS** — yt-dlp and the stream probe (curl) run as throw-away
  subprocesses so their allocations go back to the OS. See the top of `src/sources.rs`.
- **One ffmpeg process** for playback (sink audio + analyser PCM + optional video tap on fd 3).
  **One theme** (`src/theme.rs`). **No new crates** without asking.
- Blocking work runs in an `exec::spawn` worker and returns a `Msg` (with a request token) over the main
  channel — never block the main loop.

## Commands

```
cargo build --release   # target/release/ytui (LTO, strip, panic=abort)
cargo test              # offline only: URL parsing, yt-dlp line parsing, layout into a Buffer
cargo clippy            # must stay warning-free
./ytui.sh               # builds if needed, then runs (user only — agents don't run the app)
```

Toolchain: stable Rust, edition 2024. The distributed binaries embed a minimal ffmpeg and quickjs-ng
(`scripts/build-bundle.sh <target>` → `bundle/`, embedded by `build.rs`; `scripts/dist.sh` → `dist/`) and
install/update yt-dlp themselves (`src/tools.rs`). Still required on the system: `curl`, and on Linux
PulseAudio/PipeWire + glibc ≥ 2.17. Without `bundle/`, the system's ffmpeg / JS runtime are used.

```
cargo test -- --ignored   # managed yt-dlp install (downloads from GitHub, not YouTube)
scripts/dist.sh           # needs nasm, pkg-config, cmake, zig, cargo-zigbuild
git tag vX.Y && git push origin vX.Y   # .github/workflows/build.yml builds, tests and releases
```

Crates (the whole budget — AGENTS.md rule 5): `crossterm`, `ratatui-core`, `ratatui-crossterm`, `libc`,
`unicode-width`, all with default features off where possible.

## Architecture

Terminal YouTube client, audio-first. One main thread owns `App`; it sleeps on an `mpsc` channel fed by an
input thread (crossterm events), worker threads (yt-dlp / ffmpeg results) and the player. It wakes at 20 Hz
only while something animates, else once a second (RAM readout), and redraws only when `dirty`.

| File | Role |
|------|------|
| `src/main.rs` | CLI flags (`--firefox`…), terminal setup/restore, input thread, main loop |
| `src/app.rs` | `App` — state, key/mouse routing, workers, queue logic, layout + drawing, offline layout test |
| `src/widgets.rs` | tall border, `TextInput`, `VList`, spectrum, seek bar, `HalfBlock` clip, `ThumbGrid` |
| `src/theme.rs` | every colour — the only theme |
| `src/sources.rs` | yt-dlp calls (search, related, playlist, home feed), URL/ID parsing, stream resolution, curl probe |
| `src/player.rs` | `Player` — one ffmpeg: sink + 16 kHz PCM tap + optional 320x180 RGB tap; `Analyser` (Goertzel) |
| `src/exec.rs` | subprocess with timeout, `which`, small-stack thread spawn |
| `src/tools.rs` | embedded ffmpeg/qjs extraction, JS runtime choice, managed yt-dlp (install + daily update) |
| `build.rs` | embeds `bundle/<target>/{ffmpeg,qjs}` when present (`cfg(bundled)`) |
| `src/history.rs` | persisted search history (↑/↓ in the search field) |
| `src/meminfo.rs` | RSS of the process tree (`/proc` on Linux, libproc on macOS) |

**Playback model:** YouTube rejects open-ended `Range` requests on audio-only formats for PO-token-free
clients, which is what ffmpeg sends. `sources::resolve` tries strategies in order (audio-only, android
progressive/muxed, then cookie-authenticated web client when logged in), probing each signed URL the way
ffmpeg will fetch it (`stream_playable`) before handing it over; the last working strategy is remembered in
`PREFERRED`. `Player` takes a `Provider` (`Fn(refresh)`) rather than a bare URL so retries can re-sign.
Pause is `SIGSTOP`; seek/volume restart ffmpeg at the current position; each start bumps an `epoch` so
stale threads exit silently. Video is decoded only when asked for (`v`/`V`), only from a progressive stream.

**Queue/playlist model:** the "UP NEXT" panel is dual-purpose — auto mode (default, refilled from
`sources::related()` per track) vs. playlist mode (`queue` non-empty, loaded from a pasted playlist URL/ID).
`play_video(.., keep_queue = true)` skips `load_suggestions` so the panel isn't overwritten mid-playlist;
`n` and track-end advance `queue_index`. Playing anything from the results clears the queue and reverts to
auto mode. Any new playback path must decide explicitly whether it keeps or clears the queue.

## Conventions

- Everything is English: user-facing strings (terse: `"playing"`, `"end of playlist"`), code,
  comments and all docs (README, AGENTS.md, CLAUDE.md, release notes).
- Comments explain *why*, not *what* — this codebase carries a lot of rationale about YouTube behaviour,
  403s, PO tokens, ffmpeg quirks. Preserve and update them, don't strip them.
- Clippy-clean, no `unwrap` on user- or network-controlled data, plain structs for records.
- Small, surgical diffs — don't reformat untouched code. Don't touch `target/`, `.idea/`.
  Update `README.md` when a key binding or user-visible behaviour changes.
- Commits: Conventional Commits, scope `ytui`, subject ≤50 chars. Commit only when asked.
