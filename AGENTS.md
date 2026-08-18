# AGENTS.md — ytui

Guide for AI coding agents working in this repo. Read before editing.

## Prime directive

**The point of this app is to spend as little RAM, CPU and battery as
possible.** It exists because a browser tab playing YouTube costs hundreds of
megabytes and burns a laptop's battery. Every design choice below is downstream
of that goal, and it is the tie-breaker for every decision you make here.

Concretely, when weighing a change:

- Memory that can be given back to the OS must be given back — hence yt-dlp as
  a throw-away subprocess, not an import.
- No decoding work that nobody looks at: audio-only by default, the video track
  is decoded only while it is on screen.
- No polling or redrawing beyond what the eye needs: the analyser and clip run
  at fixed low rates (`FPS = 20`, `VID_FPS = 12`, a 320x180 decode grid), and
  the memory readout ticks once per second.
- No new resident processes, threads or buffers unless something equally costly
  goes away.
- The RAM figure in the status bar is a feature, not debug output — it is the
  app holding itself accountable. Don't let it grow.

A change that makes the UI nicer but the process fatter is a regression here.
If you must trade, say so explicitly and let the user decide.

## What this is

`ytui` is a terminal YouTube client, **audio-first**. Textual TUI, yt-dlp for
metadata and stream resolution, a single ffmpeg process for playback, spectrum
analyser and optional inline video clip.

No web server, no database, no async I/O framework beyond Textual's own loop.

## Layout

| File | Role |
|------|------|
| `ytui/__main__.py` | entry point (`python -m ytui`) |
| `ytui/app.py` | `YtuiApp` — layout, key bindings, workers, playback orchestration |
| `ytui/app.tcss` | all colours and layout. **The only theme.** |
| `ytui/sources.py` | yt-dlp calls: search, related, playlist, URL/ID parsing, stream resolution |
| `ytui/player.py` | `Player` — one ffmpeg process: sink output + PCM tap + optional RGB video tap |
| `ytui/widgets.py` | render-only widgets: `Spectrum`, `SeekBar`, `SearchInput`, `VideoItem`, `Clip` |
| `ytui/history.py` | persisted search history (↑/↓ in the search field) |
| `ytui/meminfo.py` | RSS of the process tree, shown in the status bar |
| `ytui.sh` | launcher; creates `.venv` on first run |

## Hard rules

Each of these is the prime directive made concrete. Don't relax one for
convenience.

1. **Never test by running the app or hitting the network.** No `yt-dlp` calls,
   no playback, no `python -m ytui`. Import-checking a module is fine.
   Hand testing to the user: state exactly what to try.
2. **Do not `import yt_dlp`.** It is invoked as a *binary* in a throw-away
   subprocess on purpose — it allocates ~60 MB parsing a page and that memory
   must go back to the OS, not stay resident in the TUI. See the docstring at
   the top of `sources.py`.
3. **One ffmpeg process.** Audio sink, analyser PCM, and video frames all come
   out of the same process. Don't add a second ffmpeg, PortAudio, or numpy.
4. **One theme.** Every colour lives in `app.tcss`. The command palette is
   disabled (`ENABLE_COMMAND_PALETTE = False`) because theme switching was its
   only use here.
5. **No new dependencies** without asking. Current set: `textual`, `yt-dlp`,
   `certifi`. That is the whole budget.
6. Blocking work (yt-dlp, network) goes in a `@work(thread=True)` worker and
   comes back to the UI through `call_from_thread`. Never block the event loop.

## Conventions

- **Language:** user-facing strings are French, lowercase-ish, terse
  (`"lecture"`, `"fin de la playlist"`). Code, identifiers and most comments
  are English. Existing French comments stay French — match the local file.
- **Comments explain *why*, not *what*.** The codebase is dense with
  rationale comments about YouTube's behaviour, 403s, PO tokens, ffmpeg
  quirks. Preserve them. If you change the reasoning, update the comment.
- Type hints everywhere, `from __future__ import annotations` at the top.
- Dataclasses for records (`Video`, `_Strategy`).
- Line length ~88. 4-space indent. No formatter is enforced — match the file.
- Widget ids are kebab-case (`#now-title`, `#clip-layer`); query them with
  `query_one(selector, Type)` and guard teardown races with `NoMatches`.

## Playback model (why it looks like this)

YouTube refuses open-ended `Range: bytes=0-` requests on audio-only formats
for clients that need no PO token, which is exactly the request ffmpeg makes.
So `sources.resolve_audio` tries strategies in order (audio-only, then the
android progressive/muxed stream), **probes each signed URL the way ffmpeg
will fetch it** (`stream_playable`), and only hands over a URL that actually
served bytes. The last working strategy is remembered in `_preferred`.

The player takes a `provider(refresh)` callable rather than a URL, so every
retry can re-sign a fresh URL instead of retrying a dead one.

Pause is `SIGSTOP` on the ffmpeg process. Seek and volume restart the process
at the current position. Video is only decoded when asked for (`v` / `V`), and
only from a progressive stream — picture and sound in one URL, one process.

## Queue / playlist model

`#suggestions` (the "SUITE" panel) is dual-purpose:

- **auto mode** (default): refilled from `sources.related()` on every track.
- **playlist mode**: `self.queue` non-empty. Pasting a playlist URL/ID loads
  it, and `play_video(..., keep_queue=True)` **skips** `load_suggestions` so
  the panel never gets overwritten mid-playlist. `n` and track-end advance
  `queue_index`. Playing anything from `#results` clears the queue and returns
  the panel to auto.

Keep that invariant: any new playback path must decide explicitly whether it
keeps or clears the queue.

## Workflow

- Small, surgical diffs. Don't reformat untouched code.
- Don't touch `.venv/`, `__pycache__/`, `.idea/`.
- Update `README.md` when a key binding or user-visible behaviour changes.
- Commits: Conventional Commits, scope `ytui`, subject ≤50 chars, French or
  English body only when the *why* isn't obvious. Commit only when asked.
