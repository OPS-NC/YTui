# AGENTS.md — ytui

Guide for AI coding agents working in this repo. Read before editing.

## Prime directive

**The point of this app is to spend as little RAM, CPU and battery as
possible.** It exists because a browser tab playing YouTube costs hundreds of
megabytes and burns a laptop's battery. Every design choice below is downstream
of that goal, and it is the tie-breaker for every decision you make here.

It was rewritten from Python/Textual to Rust for exactly this reason: no
interpreter, no widget tree, no CSS engine — a cell buffer and a diffing
terminal backend.

Concretely, when weighing a change:

- Memory that can be given back to the OS must be given back — hence yt-dlp
  and curl as throw-away subprocesses, never linked in, and the embedded
  ffmpeg/qjs bytes `madvise`d away once extracted.
- No decoding work that nobody looks at: audio-only by default, the video track
  is decoded only when asked for (`v` / `V`).
- No polling or redrawing beyond what the eye needs. The main loop sleeps on a
  channel: 20 Hz (`FPS_PERIOD`) only while something moves on screen (playing,
  or the meter decaying), once a second otherwise for the RAM readout. Redraws
  happen only when `dirty`; the clip redraws only on a new frame
  (`VID_FPS = 24`, a 320x180 decode grid, pushed by the player rather than polled). No blinking cursor.
- No new resident processes, threads or buffers unless something equally costly
  goes away. Worker threads get a 256 KiB stack (`exec::spawn`).
- The RAM figure in the status bar is a feature, not debug output — it is the
  app holding itself accountable. Don't let it grow.

A change that makes the UI nicer but the process fatter is a regression here.
If you must trade, say so explicitly and let the user decide.

## What this is

`ytui` is a terminal YouTube client, **audio-first**. Rust, ratatui-core for
the cell buffer + crossterm backend (no ratatui widget crate, no layout
solver), yt-dlp for metadata and stream resolution, curl to probe stream URLs,
a single ffmpeg process for playback, spectrum analyser and optional inline
video clip.

No web server, no database, no async runtime. One main thread owns all UI
state; workers report back over an `mpsc` channel.

## Layout

| File | Role |
|------|------|
| `src/main.rs` | entry point: CLI flags, terminal setup, input thread, main loop |
| `src/app.rs` | `App` — state, key/mouse handling, workers, playback orchestration, layout + drawing |
| `src/widgets.rs` | render-only pieces: tall border, `TextInput`, `VList`, spectrum, seek bar, `HalfBlock` clip, `ThumbGrid` |
| `src/theme.rs` | all colours. **The only theme.** |
| `src/sources.rs` | yt-dlp calls: search, related, playlist, home feed, URL/ID parsing, stream resolution, curl probe |
| `src/player.rs` | `Player` — one ffmpeg process: sink output + PCM tap + optional RGB video tap; `Analyser` (Goertzel) |
| `src/exec.rs` | subprocess with timeout, `which`, small-stack worker spawn |
| `src/tools.rs` | where ffmpeg / JS runtime / yt-dlp come from: embedded-tool extraction, managed yt-dlp install + daily update |
| `src/history.rs` | persisted search history (↑/↓ in the search field) |
| `src/meminfo.rs` | RSS of the process tree (`/proc` on Linux, libproc on macOS) |
| `build.rs` | embeds `bundle/<target>/{ffmpeg,qjs}` when present (`cfg(bundled)`) |
| `scripts/build-bundle.sh` | builds the minimal ffmpeg + qjs for one target into `bundle/` |
| `scripts/dist.sh` | self-contained binaries for macOS arm64 / Linux x86_64 / aarch64 into `dist/` |
| `ytui.sh` | launcher; builds the release binary when needed |

## Hard rules

Each of these is the prime directive made concrete. Don't relax one for
convenience.

1. **Never test by running the app or hitting the network.** No `yt-dlp`
   calls, no playback, no `cargo run`. `cargo build`, `cargo clippy` and
   `cargo test` are fine — tests must stay offline (pure parsing, layout
   rendered into an in-memory `Buffer`). Hand testing to the user: state
   exactly what to try.
2. **No in-process yt-dlp, HTTP or TLS.** yt-dlp is invoked as a *binary* in a
   throw-away subprocess on purpose — it allocates ~60 MB parsing a page and
   that memory must go back to the OS. Same for the 8 kB stream probe: a curl
   subprocess, not a TLS stack linked into the TUI. Listings are read as one
   `--print` line per entry, not a JSON dump. See the top of `sources.rs`.
3. **One ffmpeg process** for playback. The embedded ffmpeg is configured
   with `--disable-everything` plus exactly what ytui runs (see
   `build-bundle.sh`); enabling a component means justifying its size, and
   it must stay LGPL (no `--enable-gpl`). Audio sink, analyser PCM (stdout) and
   video frames (fd 3) all come out of the same process. Don't add a second
   ffmpeg, PortAudio, cpal, an FFT crate or an image crate. (Thumbnails are
   separate one-shot ffmpeg decodes, sequential, only for what is on screen.)
4. **One theme.** Every colour lives in `src/theme.rs`. Widgets never spell an
   RGB value.
5. **No new crates** without asking. Current set: `crossterm`, `ratatui-core`,
   `ratatui-crossterm`, `libc`, `unicode-width`. That is the whole budget.
   Default features stay off.
6. Blocking work (yt-dlp, network, ffmpeg start-up) goes in an `exec::spawn`
   worker and comes back as a `Msg` carrying the token of its request group;
   stale replies are dropped. Never block the main loop.

## Bundled tools

- ffmpeg and quickjs-ng are compiled by `scripts/build-bundle.sh` (macOS
  native with SecureTransport + AudioToolbox; Linux with zig against glibc
  2.17, static mbedTLS, and a link-time stub of `libpulse.so.0` — the
  system's is loaded at run time). `bundle/` and `dist/` are not committed.
- The embedded ffmpeg is preferred over the system's (lighter resident); the
  system's is the fallback when the embedded one doesn't start.
- ffmpeg 9 verifies TLS by default and the Linux bundle's mbedTLS has no
  trust store: every https input of the embedded Linux ffmpeg must carry
  `tools::ffmpeg_tls_args()` (`-ca_file` of the system bundle). Local-file
  tests can't catch this; `cargo test -- --ignored ffmpeg_https` does.
- yt-dlp is never embedded: `tools::maintain_ytdlp` installs it under
  `~/.local/share/ytui/bin/yt-dlp/` (zipapp if python ≥ 3.10 on Linux, else
  the *onedir* zip — the onefile builds unpack on every call), checks the
  latest release at most once a day, verifies SHA2-256SUMS, swaps directories
  atomically. `YTUI_YTDLP` pins another one and disables this.
- Testing those needs no YouTube: `cargo test` extracts and starts the
  embedded tools; `cargo test -- --ignored` installs yt-dlp from GitHub into
  a temp dir. Linux builds can be checked in a `debian` container.

## Conventions

- **Language:** everything is English — user-facing strings (lowercase-ish,
  terse: `"playing"`, `"end of playlist"`), code, comments and all
  documentation (README, AGENTS.md, CLAUDE.md, release notes). No French
  left anywhere in the interface.
- **Comments explain *why*, not *what*.** The codebase is dense with
  rationale comments about YouTube's behaviour, 403s, PO tokens, ffmpeg
  quirks. Preserve them. If you change the reasoning, update the comment.
- Edition 2024 (`gen` is reserved — the player uses `epoch`). `cargo clippy`
  stays warning-free. No `unwrap` on anything user- or network-controlled.
- Records are plain structs (`Video`, `Stream`, `Meta`, `Strategy`).
- Line length ~100, rustfmt-ish style; match the file.
- Hit-testing uses the rects stored during the last draw (`Hits`, `VList::view`,
  `ThumbGrid::view`); draw is the only place geometry is computed.

## Playback model (why it looks like this)

YouTube refuses open-ended `Range: bytes=0-` requests on audio-only formats
for clients that need no PO token, which is exactly the request ffmpeg makes.
So `sources::resolve` tries strategies in order (audio-only, then the android
progressive/muxed stream, then — only when logged in — the web client with
cookies), **probes each signed URL the way ffmpeg will fetch it**
(`stream_playable`), and only hands over a URL that actually served bytes.
The last working strategy is remembered in `PREFERRED`.

The player takes a `Provider` (`Fn(refresh) -> (url, headers)`) rather than a
URL, so every retry can re-sign a fresh URL instead of retrying a dead one.
The resolved stream is cached in `App::stream_cache`, shared with the
provider, so a clip toggle re-resolves only when the cached URL has no video.

Pause is `SIGSTOP` on the ffmpeg process. Seek and volume restart the process
at the current position. Every start bumps the player's `epoch`; threads of a
superseded process notice and exit without reporting. Video is only decoded
when asked for (`v` / `V`), and only from a progressive stream — picture and
sound in one URL, one process.

## Queue / playlist model

`suggestions` (the "UP NEXT" panel) is dual-purpose:

- **auto mode** (default): refilled from `sources::related()` on every track.
- **playlist mode**: `queue` non-empty. Pasting a playlist URL/ID loads it,
  and `play_video(..., keep_queue = true)` **skips** `load_suggestions` so the
  panel never gets overwritten mid-playlist. `n` and track-end advance
  `queue_index`. Playing anything from the results clears the queue and
  returns the panel to auto.

Keep that invariant: any new playback path must decide explicitly whether it
keeps or clears the queue.

## Workflow

- Small, surgical diffs. Don't reformat untouched code.
- Don't touch `target/`, `.idea/`.
- Update `README.md` when a key binding or user-visible behaviour changes.
- Commits: Conventional Commits, scope `ytui`, subject ≤50 chars, French or
  English body only when the *why* isn't obvious. Commit only when asked.
