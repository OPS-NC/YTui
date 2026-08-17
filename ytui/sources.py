"""YouTube metadata via the yt-dlp *binary*.

yt-dlp is deliberately NOT imported: every call runs in a throw-away process,
so the ~60 MB it allocates while parsing a page is returned to the OS instead
of staying resident in the TUI for the whole session.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass, field
from urllib.parse import parse_qs, urlparse

TIMEOUT = 90


@dataclass
class Video:
    id: str
    title: str
    uploader: str = ""
    duration: int | None = None
    stream_url: str | None = field(default=None, repr=False)
    headers: dict = field(default_factory=dict, repr=False)

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


def _ytdlp_cmd() -> list[str]:
    local = os.path.join(os.path.dirname(sys.executable), "yt-dlp")
    if os.path.exists(local):
        return [local]
    found = shutil.which("yt-dlp")
    if found:
        return [found]
    return [sys.executable, "-m", "yt_dlp"]


def _run(args: list[str]) -> str:
    proc = subprocess.run(
        _ytdlp_cmd() + ["--no-warnings", "--ignore-config", *args],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=TIMEOUT, text=True,
    )
    if proc.returncode != 0 and not proc.stdout.strip():
        err = (proc.stderr or "").strip().splitlines()
        raise RuntimeError(err[-1] if err else "yt-dlp a échoué")
    return proc.stdout


def _entry_to_video(entry: dict) -> Video | None:
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
    out = _run(["--flat-playlist", "--dump-single-json", *extra, url])
    if not out.strip():
        return []
    info = json.loads(out)
    return [v for v in map(_entry_to_video, info.get("entries") or []) if v]


def search(query: str, limit: int = 60) -> list[Video]:
    return _flat(f"ytsearch{limit}:{query}", [])


def related(video: Video, limit: int = 40) -> list[Video]:
    """Suggestions = the YouTube auto-mix (radio) built around the video."""
    mix = f"https://www.youtube.com/watch?v={video.id}&list=RD{video.id}"
    videos = _flat(mix, ["--playlist-end", str(limit + 1)])
    return [v for v in videos if v.id != video.id][:limit]


def resolve_audio(video: Video) -> Video:
    """Direct audio-only stream URL. Video streams are never requested."""
    out = _run([
        "-f", "bestaudio[abr<=160]/bestaudio/best",
        # The signed URL is only valid for the exact headers yt-dlp negotiated;
        # without them googlevideo answers 403.
        "--print", "%(url)s\t%(duration)s\t%(title)s\t%(uploader)s\t%(http_headers)j",
        video.url,
    ])
    line = next((ln for ln in out.splitlines() if ln.startswith("http")), "")
    if not line:
        raise RuntimeError("flux audio introuvable")
    parts = (line.split("\t") + ["", "", "", ""])[:5]
    video.stream_url = parts[0]
    if parts[1].isdigit():
        video.duration = int(parts[1])
    if parts[2] and parts[2] != "NA":
        video.title = parts[2]
    if not video.uploader and parts[3] and parts[3] != "NA":
        video.uploader = parts[3]
    try:
        headers = json.loads(parts[4])
        video.headers = {k: v for k, v in headers.items() if isinstance(v, str)}
    except (ValueError, AttributeError):
        video.headers = {}
    return video
