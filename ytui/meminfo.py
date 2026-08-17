"""Resident memory of the TUI plus every ffmpeg process it spawned.

Linux reads straight from /proc — no psutil dependency, one file read per
process. macOS has no /proc, so the whole tree comes from a single `ps` call.
"""

from __future__ import annotations

import os
import subprocess
import sys

IS_MAC = sys.platform == "darwin"

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


def _ps_snapshot() -> tuple[dict[int, int], dict[int, list[int]]]:
    """macOS: pid -> rss and ppid -> children, from one process listing."""
    rss: dict[int, int] = {}
    kids: dict[int, list[int]] = {}
    try:
        out = subprocess.run(
            ["ps", "-A", "-o", "pid=,ppid=,rss="],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, timeout=5,
        ).stdout
    except (OSError, subprocess.SubprocessError):
        return rss, kids
    for line in out.splitlines():
        fields = line.split()
        if len(fields) < 3:
            continue
        try:
            pid, ppid, kb = (int(f) for f in fields[:3])
        except ValueError:
            continue
        rss[pid] = kb * 1024          # ps reports kibibytes
        kids.setdefault(ppid, []).append(pid)
    return rss, kids


def total_rss() -> int:
    """Bytes resident for this process tree (self + ffmpeg decoder + sink)."""
    me = os.getpid()
    if IS_MAC:
        rss, kids = _ps_snapshot()
        total = rss.get(me, 0)
        seen = {me}
        stack = list(kids.get(me, []))
        while stack:
            pid = stack.pop()
            if pid in seen:
                continue
            seen.add(pid)
            total += rss.get(pid, 0)
            stack.extend(kids.get(pid, []))
        return total

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
