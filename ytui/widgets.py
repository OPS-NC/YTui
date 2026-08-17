"""Rendering-only widgets: spectrum analyser, seek bar, list rows."""

from __future__ import annotations

from rich.color import Color
from rich.segment import Segment
from rich.style import Style
from rich.text import Text
from textual.app import RenderResult
from textual.reactive import reactive
from textual.strip import Strip
from textual.widget import Widget
from textual.widgets import ListItem, Static

from .sources import Video, fmt_time

# Vertical eighths, index 0 == empty.
_EIGHTHS = " ▁▂▃▄▅▆▇█"

# Bottom -> top gradient, deliberately muted.
_GRADIENT = [
    (0x38, 0xBD, 0xF8),
    (0x60, 0xA5, 0xFA),
    (0x81, 0x8C, 0xF8),
    (0xA7, 0x8B, 0xFA),
    (0xC0, 0x84, 0xFC),
    (0xE8, 0x79, 0xF9),
    (0xF4, 0x72, 0xB6),
    (0xFB, 0x71, 0x85),
]


def _ramp(t: float) -> Color:
    t = min(max(t, 0.0), 0.999)
    pos = t * (len(_GRADIENT) - 1)
    i = int(pos)
    f = pos - i
    a, b = _GRADIENT[i], _GRADIENT[min(i + 1, len(_GRADIENT) - 1)]
    return Color.from_rgb(*(a[c] + (b[c] - a[c]) * f for c in range(3)))


class Spectrum(Widget):
    """FFT bars. Drawn line by line so no intermediate objects pile up."""

    DEFAULT_CSS = """
    Spectrum { height: 11; }
    """

    def __init__(self, player, **kwargs):
        super().__init__(**kwargs)
        self.player = player
        self._bands = []
        self._peaks = []

    def poll(self) -> None:
        bands, peaks = self.player.spectrum()
        self._bands = list(bands)
        self._peaks = list(peaks)
        self.refresh()

    def render_line(self, y: int) -> Strip:
        height = self.size.height or 1
        width = self.size.width
        if not self._bands:
            return Strip.blank(width)

        # Rows are counted from the bottom of the widget.
        row_from_bottom = height - 1 - y
        segments = []
        dim = Style(color=Color.from_rgb(0x2A, 0x2E, 0x3E))
        nbands = len(self._bands)
        bar = max(1, width // nbands)           # columns of glyph per band
        gap = 1 if bar > 1 else 0

        for i, value in enumerate(self._bands):
            filled = value * height
            cell = filled - row_from_bottom
            colour = _ramp((row_from_bottom + 0.5) / height)

            if cell >= 1.0:
                glyph, style = "█", Style(color=colour)
            elif cell > 0.0:
                glyph = _EIGHTHS[max(1, int(cell * 8))]
                style = Style(color=colour)
            else:
                peak_row = int(self._peaks[i] * height) - 1
                if peak_row == row_from_bottom and self._peaks[i] > 0.02:
                    glyph, style = "▁", Style(color=colour, dim=True)
                elif row_from_bottom == 0:
                    glyph, style = "▁", dim
                else:
                    glyph, style = " ", dim

            segments.append(Segment(glyph * (bar - gap), style))
            if gap:
                segments.append(Segment(" "))

        return Strip(segments, width).simplify()


class SeekBar(Static):
    """`▸ 1:23 ━━━━●──────── 4:56` — one line, no allocations to speak of."""

    position = reactive(0.0)
    duration = reactive(0.0)
    paused = reactive(False)

    def render(self) -> RenderResult:
        width = max(self.size.width, 24)
        pos, dur = self.position, self.duration or 0.0
        ratio = min(pos / dur, 1.0) if dur > 0 else 0.0

        left = f"{'⏸' if self.paused else '▶'} {fmt_time(pos)} "
        right = f" {fmt_time(dur) if dur else '--:--'}"
        track = max(4, width - len(left) - len(right) - 1)
        done = int(ratio * track)

        text = Text()
        text.append(left, style="bold #a78bfa")
        text.append("━" * done, style="#a78bfa")
        text.append("●", style="bold #f472b6")
        text.append("─" * max(0, track - done - 1), style="#3f3f52")
        text.append(right, style="#6b7280")
        return text


class VideoItem(ListItem):
    """A search result / suggestion row."""

    def __init__(self, video: Video, index: int | None = None):
        super().__init__()
        self.video = video
        self.index_label = index
        self._label = Static(self._text(False))

    def compose(self):
        yield self._label

    def _text(self, hl: bool) -> Text:
        """Rows carry explicit colours, so the highlight has to be repainted
        here — CSS alone cannot override a Rich style."""
        bg = "on #7c3aed" if hl else ""
        text = Text(no_wrap=True, overflow="ellipsis", style=bg)
        text.append("▎ " if hl else "  ", style=f"bold #f472b6 {bg}")
        if self.index_label is not None:
            text.append(f"{self.index_label:>2}  ", style=f"{'#e9d5ff' if hl else '#4b5563'} {bg}")
        text.append(
            f"{self.video.duration_str:>7}  ",
            style=f"{'#f5d0fe' if hl else '#7c8598'} {bg}",
        )
        text.append(
            self.video.title,
            style=f"{'bold #ffffff' if hl else '#e5e7eb'} {bg}",
        )
        if self.video.uploader:
            text.append(
                f"  · {self.video.uploader}",
                style=f"{'#e9d5ff' if hl else '#6b7280'} {bg}",
            )
        return text

    def watch_highlighted(self, value: bool) -> None:
        self.set_class(value, "-highlight")     # keep ListItem's own behaviour
        self._label.update(self._text(value))
        self._label.set_class(value, "hl")
