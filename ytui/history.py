"""Persistent search history.

One query per line, oldest first, in a plain text file — the format a shell
history uses, and for the same reason: it survives being read by a human, and
appending costs one write.
"""

from __future__ import annotations

import os
from pathlib import Path

MAX_ENTRIES = 200


def history_path() -> Path:
    """~/.local/share/ytui/search_history (XDG_DATA_HOME honoured)."""
    base = os.environ.get("XDG_DATA_HOME")
    root = Path(base) if base else Path.home() / ".local" / "share"
    return root / "ytui" / "search_history"


class SearchHistory:
    """The list plus a cursor into it, as a shell prompt sees its history.

    Cursor semantics: `len(entries)` means "at the live line, nothing recalled".
    """

    def __init__(self, path: Path | None = None):
        self.path = path or history_path()
        self.entries: list[str] = self._load()
        self.cursor = len(self.entries)
        self.draft = ""

    # ------------------------------------------------------------------ disk

    def _load(self) -> list[str]:
        try:
            lines = self.path.read_text(encoding="utf-8").splitlines()
        except (OSError, UnicodeDecodeError):
            return []
        return [line for line in (l.strip() for l in lines) if line][-MAX_ENTRIES:]

    def _save(self) -> None:
        try:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            self.path.write_text(
                "\n".join(self.entries) + "\n", encoding="utf-8"
            )
        except OSError:
            pass        # a read-only home must not take the app down

    # ----------------------------------------------------------------- edits

    def add(self, query: str) -> None:
        query = query.strip()
        if not query:
            return
        # A repeated query moves to the end rather than piling up.
        if query in self.entries:
            self.entries.remove(query)
        self.entries.append(query)
        del self.entries[:-MAX_ENTRIES]
        self.reset()
        self._save()

    def reset(self) -> None:
        self.cursor = len(self.entries)
        self.draft = ""

    # -------------------------------------------------------------- browsing

    def previous(self, current: str) -> str | None:
        """Older entry, or None at the top of the list."""
        if self.cursor == len(self.entries):
            self.draft = current            # keep what was being typed
        if self.cursor == 0:
            return None
        self.cursor -= 1
        return self.entries[self.cursor]

    def next(self, _current: str = "") -> str | None:
        """Newer entry; past the newest, hands the unsent draft back."""
        if self.cursor >= len(self.entries):
            return None
        self.cursor += 1
        if self.cursor == len(self.entries):
            return self.draft
        return self.entries[self.cursor]
