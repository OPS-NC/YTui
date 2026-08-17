"""Rendering-only widgets.

Visual language: a late-70s hi-fi separate. Warm near-black chassis, amber
legends, a peak-hold VU meter that runs cold amber at the bottom and burns
ember at the top. Everything is drawn with box-drawing glyphs — no images, no
fonts to load, no allocations beyond one Strip per visible line.
"""

from __future__ import annotations

from rich.color import Color
from textual import events
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

# Bottom -> top: the ramp of a VU meter. Cold amber, then ember, then a red
# "clipping" zone that only the loudest transients ever reach.
_RAMP = [
    (0x6B, 0x45, 0x14),
    (0x9A, 0x62, 0x1B),
    (0xC4, 0x7F, 0x1E),
    (0xE0, 0x9B, 0x2A),
    (0xF2, 0xB2, 0x3C),
    (0xFF, 0xC4, 0x59),
    (0xFF, 0x8A, 0x3D),
    (0xFF, 0x5C, 0x2E),
    (0xE0, 0x32, 0x1F),
]

_CHASSIS = Color.from_rgb(0x2A, 0x24, 0x1D)      # unlit segment
_EMBER = Color.from_rgb(0xFF, 0x6B, 0x35)


def _ramp(t: float) -> Color:
    t = min(max(t, 0.0), 0.999)
    pos = t * (len(_RAMP) - 1)
    i = int(pos)
    f = pos - i
    a, b = _RAMP[i], _RAMP[min(i + 1, len(_RAMP) - 1)]
    return Color.from_rgb(*(a[c] + (b[c] - a[c]) * f for c in range(3)))


class Spectrum(Widget):
    """Peak-hold analyser. Drawn line by line, one Strip per row."""

    def __init__(self, player, **kwargs):
        super().__init__(**kwargs)
        self.player = player
        self._bands: list[float] = []
        self._peaks: list[float] = []
        # Row colours never change: compute them once per resize, not per frame.
        self._row_styles: list[Style] = []
        self._row_height = 0

    def poll(self) -> None:
        bands, peaks = self.player.spectrum()
        self._bands = list(bands)
        self._peaks = list(peaks)
        self.refresh()

    def _styles_for(self, height: int) -> list[Style]:
        if height != self._row_height:
            self._row_height = height
            self._row_styles = [
                Style(color=_ramp((r + 0.5) / height)) for r in range(height)
            ]
        return self._row_styles

    def render_line(self, y: int) -> Strip:
        height = self.size.height or 1
        width = self.size.width
        if not self._bands or width <= 0:
            return Strip.blank(width)

        styles = self._styles_for(height)
        row = height - 1 - y                      # counted from the baseline
        lit = styles[row]
        unlit = Style(color=_CHASSIS)
        peak_style = Style(color=_EMBER, bold=True)

        nbands = len(self._bands)
        bar = max(1, width // nbands)
        gap = 1 if bar > 1 else 0
        glyph_width = bar - gap

        segments = []
        for value, peak in zip(self._bands, self._peaks):
            cell = value * height - row

            if cell >= 1.0:
                glyph, style = "█", lit
            elif cell > 0.0:
                glyph, style = _EIGHTHS[max(1, int(cell * 8))], lit
            elif peak > 0.02 and int(peak * height) - 1 == row:
                # Peak hold: the segment that lingers above the bar.
                glyph, style = "▔", peak_style
            elif row == 0:
                glyph, style = "▁", unlit          # the meter's resting floor
            else:
                glyph, style = " ", unlit

            segments.append(Segment(glyph * glyph_width, style))
            if gap:
                segments.append(Segment(" "))

        return Strip(segments, width).simplify()


class SeekBar(Static):
    """Transport line: `▶  1:23  ━━━━━◆┄┄┄┄┄  4:56`."""

    position = reactive(0.0)
    duration = reactive(0.0)
    paused = reactive(False)

    def render(self) -> RenderResult:
        pos = self.position
        dur = self.duration or 0.0
        width = max(self.size.width, 24)
        ratio = min(pos / dur, 1.0) if dur > 0 else 0.0

        head = "❙❙" if self.paused else "▶ "
        right = f"  {fmt_time(dur) if dur else 'DIRECT'}"
        track = max(4, width - len(head) - len(fmt_time(pos)) - len(right) - 4)
        done = int(ratio * track)

        text = Text(no_wrap=True)
        text.append(head, style="bold #ff6b35" if not self.paused else "bold #8a7f70")
        text.append(f"  {fmt_time(pos)}  ", style="bold #f2b23c")
        text.append("━" * done, style="#e09b2a")
        text.append("◆", style="bold #ff6b35")
        text.append("┄" * max(0, track - done - 1), style="#2a241d")
        text.append(right, style="#8a7f70" if dur else "bold #ff5c2e")
        return text


class VideoItem(ListItem):
    """One entry of a track listing."""

    def __init__(self, video: Video, index: int | None = None):
        super().__init__()
        self.video = video
        self.index_label = index
        self._label = Static(self._text(False))
        self.pending_chain: int | None = None

    def compose(self):
        yield self._label

    def _on_click(self, event: events.Click) -> None:
        """A click always highlights; only a double click starts playback.
        The click count is stashed here because ListView.Selected, which the
        app reacts to, carries no mouse information."""
        self.pending_chain = event.chain
        super()._on_click(event)

    def _text(self, hl: bool) -> Text:
        """Rows carry explicit colours, so the highlight is repainted here:
        CSS cannot override a Rich style already baked into the text."""
        bg = "on #33240f" if hl else ""
        text = Text(no_wrap=True, overflow="ellipsis", style=bg)
        text.append("▌" if hl else " ", style=f"{'#ff6b35' if hl else '#151210'} {bg}")

        if self.index_label is not None:
            text.append(
                f" {self.index_label:02d} ",
                style=f"{'bold #ffc459' if hl else '#6b6055'} {bg}",
            )
        else:
            text.append(" ", style=bg)

        text.append(
            f"{self.video.duration_str:>7}  ",
            style=f"{'#f2b23c' if hl else '#7d7266'} {bg}",
        )
        text.append(
            self.video.title,
            style=f"{'bold #fff6e6' if hl else '#cbc0b2'} {bg}",
        )
        if self.video.uploader:
            text.append(
                f"  {self.video.uploader}",
                style=f"{'#e0b98a' if hl else '#6b6055'} {bg}",
            )
        return text

    def watch_highlighted(self, value: bool) -> None:
        self.set_class(value, "-highlight")     # keep ListItem's own behaviour
        self._label.update(self._text(value))
        self._label.set_class(value, "hl")
