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

TIMEOUT = 90


@dataclass
class Video:
    id: str
    title: str
    uploader: str = ""
    duration: int | None = None
    stream_url: str | None = field(default=None, repr=False)

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


def search(query: str, limit: int = 25) -> list[Video]:
    return _flat(f"ytsearch{limit}:{query}", [])


def related(video: Video, limit: int = 25) -> list[Video]:
    """Suggestions = the YouTube auto-mix (radio) built around the video."""
    mix = f"https://www.youtube.com/watch?v={video.id}&list=RD{video.id}"
    videos = _flat(mix, ["--playlist-end", str(limit + 1)])
    return [v for v in videos if v.id != video.id][:limit]


def resolve_audio(video: Video) -> Video:
    """Direct audio-only stream URL. Video streams are never requested."""
    out = _run([
        "-f", "bestaudio[abr<=160]/bestaudio/best",
        "--print", "%(url)s\t%(duration)s\t%(title)s\t%(uploader)s",
        video.url,
    ])
    line = next((ln for ln in out.splitlines() if ln.startswith("http")), "")
    if not line:
        raise RuntimeError("flux audio introuvable")
    parts = (line.split("\t") + ["", "", ""])[:4]
    video.stream_url = parts[0]
    if parts[1].isdigit():
        video.duration = int(parts[1])
    if parts[2] and parts[2] != "NA":
        video.title = parts[2]
    if not video.uploader and parts[3] and parts[3] != "NA":
        video.uploader = parts[3]
    return video
