# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Read AGENTS.md first

This repo has a detailed [AGENTS.md](AGENTS.md) written for AI coding agents — it is the primary source of
truth for architecture, hard rules, and conventions here. Read it in full before making changes. Highlights:

- **Prime directive: minimize RAM/CPU/battery.** This app exists specifically to be lighter than a browser
  tab playing YouTube. Weigh every change against that; a change that makes the UI nicer but the process
  fatter is a regression.
- **Never run the app or hit the network to test.** No `yt-dlp` calls, no playback, no `python -m ytui`.
  Import-checking a module is fine. Hand hands-on testing to the user with exact steps to try.
- **Do not `import yt_dlp`** — it's invoked as a subprocess on purpose so its ~60 MB parse allocation is
  freed back to the OS. See `ytui/sources.py`'s module docstring.
- **One ffmpeg process** total (sink audio + analyser PCM + optional video tap). **One theme**
  (`ytui/app.tcss`); the command palette is disabled. **No new dependencies** without asking.
- Blocking work (yt-dlp, network) runs in a `@work(thread=True)` worker and returns via `call_from_thread` —
  never block the event loop.

## Commands

There is no test suite, linter, or formatter configured in this repo.

Run the app:
```
./ytui.sh
```
This creates `.venv` and installs `requirements.txt` on first run, then execs `python -m ytui`.

Manual equivalent:
```
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
.venv/bin/python -m ytui
```

Dependencies (the whole budget — see AGENTS.md rule 5): `textual`, `yt-dlp`, `certifi`, all pinned loosely
in [requirements.txt](requirements.txt).

## Architecture

Terminal YouTube client, audio-first: Textual TUI, yt-dlp (binary subprocess) for metadata/stream
resolution, a single ffmpeg process for playback + spectrum analysis + optional inline video clip. No web
server, no database, no async I/O framework beyond Textual's own loop.

| File | Role |
|------|------|
| `ytui/__main__.py` | entry point (`python -m ytui`) |
| `ytui/app.py` | `YtuiApp` — layout, key bindings, workers, playback orchestration |
| `ytui/app.tcss` | all colours and layout — the only theme |
| `ytui/sources.py` | yt-dlp calls: search, related, playlist, URL/ID parsing, stream resolution |
| `ytui/player.py` | `Player` — one ffmpeg process: sink output + PCM tap + optional RGB video tap |
| `ytui/widgets.py` | render-only widgets: `Spectrum`, `SeekBar`, `SearchInput`, `VideoItem`, `Clip` |
| `ytui/history.py` | persisted search history (↑/↓ in the search field) |
| `ytui/meminfo.py` | RSS of the process tree, shown in the status bar |

**Playback model:** YouTube rejects open-ended `Range` requests on audio-only formats for PO-token-free
clients, which is what ffmpeg sends. `sources.resolve_audio` tries strategies in order (audio-only, then
android progressive/muxed), probing each signed URL the way ffmpeg will fetch it (`stream_playable`) before
handing it over; the last working strategy is remembered in `_preferred`. `Player` takes a
`provider(refresh)` callable rather than a bare URL so retries can re-sign instead of reusing a dead one.
Pause is `SIGSTOP` on the ffmpeg process; seek/volume restart it at the current position. Video is decoded
only while displayed (`v`/`V`), and only from a progressive stream.

**Queue/playlist model:** `#suggestions` (the "SUITE" panel) is dual-purpose — auto mode (default, refilled
from `sources.related()` per track) vs. playlist mode (`self.queue` non-empty, loaded from a pasted
playlist URL/ID). `play_video(..., keep_queue=True)` skips `load_suggestions` so the panel isn't overwritten
mid-playlist; `n` and track-end advance `queue_index`. Playing anything from `#results` clears the queue and
reverts to auto mode. Any new playback path must decide explicitly whether it keeps or clears the queue.

## Conventions

- User-facing strings are French, terse (`"lecture"`, `"fin de la playlist"`). Code/identifiers/most
  comments are English; existing French comments stay French — match the local file.
- Comments explain *why*, not *what* — this codebase carries a lot of rationale about YouTube behaviour,
  403s, PO tokens, ffmpeg quirks. Preserve and update them, don't strip them.
- `from __future__ import annotations`, type hints everywhere, dataclasses for records.
- Line length ~88, 4-space indent, no formatter enforced — match the surrounding file.
- Widget ids are kebab-case (`#now-title`, `#clip-layer`); query with `query_one(selector, Type)` and guard
  teardown races with `NoMatches`.
- Small, surgical diffs — don't reformat untouched code. Don't touch `.venv/`, `__pycache__/`, `.idea/`.
  Update `README.md` when a key binding or user-visible behaviour changes.
- Commits: Conventional Commits, scope `ytui`, subject ≤50 chars. Commit only when asked.
