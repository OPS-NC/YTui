"""YouTube metadata via the yt-dlp *binary*.

yt-dlp is deliberately NOT imported: every call runs in a throw-away process,
so the ~60 MB it allocates while parsing a page is returned to the OS instead
of staying resident in the TUI for the whole session.
"""

from __future__ import annotations

import json
import os
import shutil
import ssl
import subprocess
import sys
import urllib.request
from dataclasses import dataclass, field
from functools import lru_cache
from urllib.parse import parse_qs, urlparse

TIMEOUT = 90
PROBE_BYTES = 8192
PROBE_TIMEOUT = 8


@dataclass
class Video:
    id: str
    title: str
    uploader: str = ""
    duration: int | None = None
    stream_url: str | None = field(default=None, repr=False)
    headers: dict = field(default_factory=dict, repr=False)
    source: str = field(default="", repr=False)
    has_video: bool = field(default=False, repr=False)

    @property
    def url(self) -> str:
        return f"https://www.youtube.com/watch?v={self.id}"

    @property
    def duration_str(self) -> str:
        return fmt_time(self.duration) if self.duration else "--:--"


def fmt_time(seconds: float | None) -> str:
    if seconds is None:
        return "--:--"
    seconds = int(max(0, seconds))
    h, rem = divmod(seconds, 3600)
    m, s = divmod(rem, 60)
    return f"{h}:{m:02d}:{s:02d}" if h else f"{m}:{s:02d}"


_ID_CHARS = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_"


def parse_video_id(text: str) -> str | None:
    """Accept a bare id, a watch/shorts/embed URL or a youtu.be link."""
    text = text.strip()
    if not text or " " in text:
        return None

    if len(text) == 11 and all(c in _ID_CHARS for c in text):
        return text

    if "://" not in text:
        if not text.startswith(("youtube.com", "www.youtube.com", "youtu.be",
                                "m.youtube.com", "music.youtube.com")):
            return None
        text = "https://" + text

    parsed = urlparse(text)
    host = parsed.netloc.lower().removeprefix("www.")
    candidate = ""

    if host == "youtu.be":
        candidate = parsed.path.lstrip("/").split("/")[0]
    elif host in ("youtube.com", "m.youtube.com", "music.youtube.com", "youtube-nocookie.com"):
        if parsed.path == "/watch":
            candidate = parse_qs(parsed.query).get("v", [""])[0]
        else:
            parts = [p for p in parsed.path.split("/") if p]
            if parts and parts[0] in ("shorts", "embed", "live", "v"):
                candidate = parts[1] if len(parts) > 1 else ""

    if len(candidate) == 11 and all(c in _ID_CHARS for c in candidate):
        return candidate
    return None


_PLAYLIST_PREFIXES = ("PL", "OL", "UU", "LL", "FL", "RD", "UL", "TL")


def parse_playlist_id(text: str) -> str | None:
    """Accept a bare playlist id or any URL carrying a `list=` parameter."""
    text = text.strip()
    if not text or " " in text:
        return None

    if text.startswith(_PLAYLIST_PREFIXES) and len(text) >= 12 \
            and all(c in _ID_CHARS for c in text):
        return text

    if "://" not in text:
        if not text.startswith(("youtube.com", "www.youtube.com", "youtu.be",
                                "m.youtube.com", "music.youtube.com")):
            return None
        text = "https://" + text

    parsed = urlparse(text)
    host = parsed.netloc.lower().removeprefix("www.")
    if host not in ("youtube.com", "m.youtube.com", "music.youtube.com",
                    "youtube-nocookie.com", "youtu.be"):
        return None
    candidate = parse_qs(parsed.query).get("list", [""])[0]
    if candidate and all(c in _ID_CHARS for c in candidate):
        return candidate
    return None


def playlist(playlist_id: str, limit: int = 500) -> list[Video]:
    """Flat listing of a playlist, in playlist order."""
    # Auto-mix ids (RD…) are only served from a watch page, not from /playlist.
    if playlist_id.startswith("RD"):
        seed = playlist_id[2:]
        url = f"https://www.youtube.com/watch?v={seed}&list={playlist_id}"
    else:
        url = f"https://www.youtube.com/playlist?list={playlist_id}"
    return _flat(url, ["--playlist-end", str(limit)])


def _ytdlp_cmd() -> list[str]:
    local = os.path.join(os.path.dirname(sys.executable), "yt-dlp")
    if os.path.exists(local):
        return [local]
    found = shutil.which("yt-dlp")
    if found:
        return [found]
    return [sys.executable, "-m", "yt_dlp"]


@lru_cache(maxsize=1)
def _js_runtime_args() -> tuple[str, ...]:
    """yt-dlp needs a JS engine to solve YouTube's signature challenges and
    only auto-enables deno. Any of these will do, so whatever is installed is
    declared — extraction without one is deprecated upstream."""
    for runtime in ("deno", "node", "bun", "quickjs"):
        if shutil.which(runtime):
            return ("--js-runtimes", runtime)
    return ()


def _explain(stderr: str) -> str:
    """Turn yt-dlp's last stderr line into something actionable."""
    lines = [ln for ln in (stderr or "").strip().splitlines() if ln.strip()]
    last = lines[-1] if lines else "yt-dlp a échoué"
    if "CERTIFICATE_VERIFY_FAILED" in stderr:
        return ("aucun certificat CA disponible pour Python — installez certifi "
                "(.venv/bin/pip install certifi) ou lancez "
                "« /Applications/Python 3.x/Install Certificates.command »")
    return last


def _run(args: list[str]) -> tuple[str, str]:
    """Returns (stdout, stderr). yt-dlp often exits non-zero while still
    printing usable JSON, so a failed return code alone is not an error."""
    proc = subprocess.run(
        _ytdlp_cmd() + ["--no-warnings", "--ignore-config",
                        *_js_runtime_args(), *args],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=TIMEOUT, text=True,
    )
    if proc.returncode != 0 and not proc.stdout.strip():
        raise RuntimeError(_explain(proc.stderr))
    return proc.stdout, proc.stderr


def _entry_to_video(entry: dict | None) -> Video | None:
    # A partially failed extraction yields None entries — YouTube errors, a
    # blocked page, or no network at all — so this is never assumed to be a dict.
    if not isinstance(entry, dict):
        return None
    vid = entry.get("id")
    if not vid or len(vid) != 11:
        return None
    return Video(
        id=vid,
        title=entry.get("title") or "(sans titre)",
        uploader=entry.get("uploader") or entry.get("channel") or "",
        duration=int(entry["duration"]) if entry.get("duration") else None,
    )


def _flat(url: str, extra: list[str]) -> list[Video]:
    out, err = _run(["--flat-playlist", "--dump-single-json", *extra, url])
    if not out.strip():
        raise RuntimeError(_explain(err))
    info = json.loads(out) or {}
    entries = info.get("entries") or []
    videos = [v for v in map(_entry_to_video, entries) if v]
    # Every entry unusable while stderr complained: report the real cause
    # instead of an empty, silent result list.
    if not videos and err.strip():
        raise RuntimeError(_explain(err))
    return videos


def search(query: str, limit: int = 60) -> list[Video]:
    return _flat(f"ytsearch{limit}:{query}", [])


def related(video: Video, limit: int = 40) -> list[Video]:
    """Suggestions = the YouTube auto-mix (radio) built around the video."""
    mix = f"https://www.youtube.com/watch?v={video.id}&list=RD{video.id}"
    videos = _flat(mix, ["--playlist-end", str(limit + 1)])
    return [v for v in videos if v.id != video.id][:limit]


@dataclass(frozen=True)
class _Strategy:
    """One way of asking YouTube for something playable."""
    label: str
    extractor_args: tuple[str, ...]
    fmt: str


# YouTube only hands a plain https URL to the clients that need no PO token,
# and it now refuses long-range requests on their audio-only formats: ffmpeg
# opens a stream with `Range: bytes=0-` and gets 403 every single time, which
# is what turned playback into a losing retry loop. The progressive stream of
# the android client still answers, at the price of downloading a 360p video
# track that ffmpeg throws away. So: audio-only first, muxed as a safety net.
STRATEGIES = (
    _Strategy("audio seul", (), "bestaudio[abr<=160]/bestaudio"),
    _Strategy("flux muxé", ("--extractor-args", "youtube:player_client=android"),
              "bestaudio/18/best[acodec!=none]"),
)

# Only a progressive stream carries picture and sound in one URL, which is what
# the single-ffmpeg design needs to show the clip without a second download.
VIDEO_STRATEGY = _Strategy("clip 360p",
                           ("--extractor-args", "youtube:player_client=android"),
                           "18/best[acodec!=none][vcodec!=none]")

_PRINT = "%(url)s\t%(duration)s\t%(title)s\t%(uploader)s\t%(vcodec)s\t%(http_headers)j"

# What worked for the previous track, tried first for the next one: whichever
# way YouTube is treating this session tends to hold for the whole session, and
# a doomed first strategy costs a yt-dlp round trip per track.
_preferred = 0


@lru_cache(maxsize=1)
def _ssl_context() -> ssl.SSLContext:
    try:
        import certifi
        return ssl.create_default_context(cafile=certifi.where())
    except Exception:
        return ssl.create_default_context()


def stream_playable(url: str, headers: dict) -> bool:
    """Probe the URL exactly the way ffmpeg will fetch it — one open-ended
    range request — because that is the request YouTube rejects. A URL that
    fails here would fail ten times in a row in the player."""
    if ".m3u8" in url or "/manifest/" in url:
        return True                      # HLS: fetched segment by segment
    request = urllib.request.Request(url, headers={**headers, "Range": "bytes=0-"})
    try:
        with urllib.request.urlopen(request, timeout=PROBE_TIMEOUT,
                                    context=_ssl_context()) as response:
            return bool(response.read(PROBE_BYTES))
    except Exception:
        return False


def _resolve_with(video: Video, strategy: _Strategy) -> tuple[str, dict, str, bool]:
    out, err = _run([*strategy.extractor_args, "-f", strategy.fmt,
                     # The signed URL is only valid for the exact headers
                     # yt-dlp negotiated; without them googlevideo answers 403.
                     "--print", _PRINT, video.url])
    line = next((ln for ln in out.splitlines() if ln.startswith("http")), "")
    if not line:
        raise RuntimeError(_explain(err) if err.strip() else "flux audio introuvable")
    parts = (line.split("\t") + [""] * 6)[:6]
    if parts[1].isdigit():
        video.duration = int(parts[1])
    if parts[2] and parts[2] != "NA":
        video.title = parts[2]
    if not video.uploader and parts[3] and parts[3] != "NA":
        video.uploader = parts[3]
    try:
        headers = {k: v for k, v in json.loads(parts[5]).items() if isinstance(v, str)}
    except (ValueError, AttributeError, TypeError):
        headers = {}
    has_video = parts[4] not in ("none", "", "NA")
    kind = f"{strategy.label} 360p" if has_video and "360p" not in strategy.label \
        else strategy.label
    return parts[0], headers, kind, has_video


def resolve_audio(video: Video, want_video: bool = False) -> Video:
    """Fill in a stream URL that has been checked against the real fetch.

    Every strategy is tried in order and validated; only a URL that actually
    served bytes is handed to the player. `want_video` puts the progressive
    stream first — the audio-only ones stay in the list, so asking for the clip
    can never cost the sound.
    """
    global _preferred
    last = "flux audio introuvable"
    order = [STRATEGIES[i]
             for i in sorted(range(len(STRATEGIES)), key=lambda i: i != _preferred)]
    if want_video:
        order.insert(0, VIDEO_STRATEGY)
    for strategy in order:
        try:
            url, headers, kind, has_video = _resolve_with(video, strategy)
        except Exception as exc:
            last = str(exc)
            continue
        if not stream_playable(url, headers):
            last = (f"403 sur « {strategy.label} » — YouTube exige un PO token "
                    "pour ce client")
            continue
        video.stream_url, video.headers = url, headers
        video.source, video.has_video = kind, has_video
        if strategy in STRATEGIES:
            _preferred = STRATEGIES.index(strategy)
        return video
    raise RuntimeError(last)
