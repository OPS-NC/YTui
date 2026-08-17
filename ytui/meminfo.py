"""Resident memory of the TUI plus every ffmpeg process it spawned.

Read straight from /proc — no psutil dependency, one file read per process.
"""

from __future__ import annotations

import os

_PAGE = os.sysconf("SC_PAGE_SIZE")


def _rss(pid: int) -> int:
    try:
        with open(f"/proc/{pid}/statm", "rb") as fh:
            return int(fh.read().split()[1]) * _PAGE
    except (OSError, IndexError, ValueError):
        return 0


def _children(pid: int) -> list[int]:
    out = []
    try:
        for task in os.listdir(f"/proc/{pid}/task"):
            with open(f"/proc/{pid}/task/{task}/children", "rb") as fh:
                out.extend(int(p) for p in fh.read().split())
    except OSError:
        pass
    return out


def total_rss() -> int:
    """Bytes resident for this process tree (self + ffmpeg decoder + sink)."""
    me = os.getpid()
    total = _rss(me)
    seen = {me}
    stack = _children(me)
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        total += _rss(pid)
        stack.extend(_children(pid))
    return total


def human(nbytes: int) -> str:
    mb = nbytes / (1024 * 1024)
    return f"{mb:,.1f} Mo".replace(",", " ")
