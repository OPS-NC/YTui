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
from textual.geometry import Size
from textual.reactive import reactive
from textual.scroll_view import ScrollView
from textual.strip import Strip
from textual.widget import Widget
from textual.widgets import Input, ListItem, Static

from .history import SearchHistory
from .player import VID_H, VID_W
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

_CHASSIS = Color.from_rgb(0x33, 0x2C, 0x24)      # unlit segment
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
        text.append("┄" * max(0, track - done - 1), style="#5f564c")
        text.append(right, style="#8a7f70" if dur else "bold #ff5c2e")
        return text


class SearchInput(Input):
    """The search field, with a shell-style history on up/down."""

    def __init__(self, history: SearchHistory, **kwargs):
        super().__init__(**kwargs)
        self.history = history

    async def _on_key(self, event: events.Key) -> None:
        if event.key == "up":
            recalled = self.history.previous(self.value)
        elif event.key == "down":
            recalled = self.history.next(self.value)
        else:
            await super()._on_key(event)
            return

        event.prevent_default()
        event.stop()
        if recalled is None:
            return
        self.value = recalled
        self.cursor_position = len(recalled)


class VideoItem(ListItem):
    """One entry of a track listing."""

    def __init__(self, video: Video):
        super().__init__()
        self.video = video
        self.playing = False
        # Built empty, then painted: `_text()` reads the `highlighted`
        # reactive, and Textual fires `watch_highlighted` on that very first
        # access — reentrantly, before this assignment would otherwise finish.
        self._label = Static()
        self._label.update(self._text())

    def compose(self):
        yield self._label

    def _on_click(self, event: events.Click) -> None:
        """A click always highlights; only a double click starts playback.
        The click count is reported to the app because ListView.Selected,
        which the app reacts to, carries no mouse information."""
        notify = getattr(self.app, "note_click", None)
        if notify:
            notify(self, event.chain)
        super()._on_click(event)

    def _text(self) -> Text:
        """Rows carry explicit colours, so highlight/playing are repainted
        here: CSS cannot override a Rich style already baked into the text.
        `playing` marks the track actually coming out of the speakers and
        takes priority over `highlighted`, the list's own cursor (keyboard
        or mouse) — the two are independent and often disagree."""
        if self.playing:
            bg, marker, marker_color = "on #3d2015", "▶ ", "#ff6b35"
            dur_color, title_style, uploader_color = "#ff8a3d", "bold #fff6e6", "#e0b98a"
        elif self.highlighted:
            bg, marker, marker_color = "on #3b2b14", "▌ ", "#ff6b35"
            dur_color, title_style, uploader_color = "#f2b23c", "bold #fff6e6", "#e0b98a"
        else:
            bg, marker, marker_color = "", "  ", "#191615"
            dur_color, title_style, uploader_color = "#7d7266", "#cbc0b2", "#6b6055"

        text = Text(no_wrap=True, overflow="ellipsis", style=bg)
        text.append(marker, style=f"{marker_color} {bg}")
        text.append(f"{self.video.duration_str:>7}  ", style=f"{dur_color} {bg}")
        text.append(self.video.title, style=f"{title_style} {bg}")
        if self.video.uploader:
            text.append(f"  {self.video.uploader}", style=f"{uploader_color} {bg}")
        return text

    def set_playing(self, value: bool) -> None:
        """Toggled by the app as playback moves through the "suite" list."""
        if self.playing == value:
            return
        self.playing = value
        self._label.update(self._text())
        self._label.set_class(value, "playing")
        self.set_class(value, "-playing")

    def watch_highlighted(self, value: bool) -> None:
        self.set_class(value, "-highlight")     # keep ListItem's own behaviour
        self._label.update(self._text())
        self._label.set_class(value, "hl")


class HalfBlockImage:
    """Nearest-neighbour half-block sampling: one cell carries two source
    pixels via `▀` (top pixel as foreground, bottom pixel as background), so
    the picture keeps the terminal's full colour depth at twice the vertical
    resolution. Shared by `Clip` (one live frame, full widget) and
    `ThumbGrid` (many static frames, one per cell) — same maths either way,
    only the pixel buffer and target rect differ.
    """

    _STYLE_CACHE_MAX = 8192

    def __init__(self):
        self._grid: tuple[int, int, int, int] = (0, 0, 0, 0)
        self._cols: list[int] = []
        self._rows: list[int] = []
        self._styles: dict[bytes, Style] = {}

    def _tables(self, width: int, height: int, src_w: int, src_h: int) -> None:
        """Nearest-neighbour sampling tables, letterboxed to keep the aspect
        ratio. -1 marks a padding column or row."""
        if self._grid == (width, height, src_w, src_h):
            return
        self._grid = (width, height, src_w, src_h)
        out_w, out_h = width, height * 2       # half-blocks: square-ish pixels
        draw_w = min(out_w, max(1, round(out_h * src_w / src_h)))
        draw_h = min(out_h, max(1, round(out_w * src_h / src_w)))
        off_x, off_y = (out_w - draw_w) // 2, (out_h - draw_h) // 2
        self._cols = [-1 if x < off_x or x >= off_x + draw_w
                      else min(src_w - 1, (x - off_x) * src_w // draw_w)
                      for x in range(out_w)]
        self._rows = [-1 if y < off_y or y >= off_y + draw_h
                      else min(src_h - 1, (y - off_y) * src_h // draw_h)
                      for y in range(out_h)]

    def _style(self, top: bytes, bottom: bytes) -> Style:
        key = top + bottom
        style = self._styles.get(key)
        if style is None:
            if len(self._styles) >= self._STYLE_CACHE_MAX:
                self._styles.clear()
            style = Style(color=Color.from_rgb(*top), bgcolor=Color.from_rgb(*bottom))
            self._styles[key] = style
        return style

    def segments(self, data: bytes, src_w: int, src_h: int,
                 width: int, height: int, y: int) -> list[Segment]:
        """One row's worth of `width` one-cell Segments — the unit `Clip`
        wraps into a Strip and `ThumbGrid` concatenates across a grid row."""
        self._tables(width, height, src_w, src_h)
        top_row, bottom_row = self._rows[2 * y], self._rows[2 * y + 1]
        if top_row < 0 and bottom_row < 0:
            return [Segment(" " * width)]

        black = b"\x00\x00\x00"
        top_base = top_row * src_w * 3 if top_row >= 0 else -1
        bottom_base = bottom_row * src_w * 3 if bottom_row >= 0 else -1

        segments = []
        for col in self._cols:
            if col < 0:
                segments.append(Segment(" "))
                continue
            shift = col * 3
            top = data[top_base + shift:top_base + shift + 3] if top_base >= 0 else black
            bottom = (data[bottom_base + shift:bottom_base + shift + 3]
                      if bottom_base >= 0 else black)
            segments.append(Segment("▀", self._style(top, bottom)))
        return segments

    def line(self, data: bytes, src_w: int, src_h: int,
              width: int, height: int, y: int) -> Strip:
        if width <= 0 or height <= 0:
            return Strip.blank(width)
        segments = self.segments(data, src_w, src_h, width, height, y)
        return Strip(segments, width).simplify()


class Clip(Widget):
    """The video track drawn with half-blocks via `HalfBlockImage`. The decode
    grid is fixed (player.VID_W x VID_H) and sampled down here, which means a
    window resize costs a table rebuild instead of an ffmpeg restart.
    """

    def __init__(self, player, **kwargs):
        super().__init__(**kwargs)
        self.player = player
        self._snap: tuple[bytes, int, int, int] | None = None
        self._shown = -1
        self._image = HalfBlockImage()

    def _on_click(self, event: events.Click) -> None:
        """Clicking the picture swaps between the deck panel and full screen."""
        toggle = getattr(self.app, "action_toggle_fullscreen", None)
        if toggle is not None:
            toggle()

    def poll(self) -> None:
        snap = self.player.frame()
        if snap is None:
            if self._snap is not None:
                self._snap = None
                self.refresh()
            return
        if snap[3] != self._shown:
            self._snap, self._shown = snap, snap[3]
            self.refresh()

    def render_line(self, y: int) -> Strip:
        width, height = self.size.width, self.size.height
        if self._snap is None or width <= 0 or height <= 0:
            return Strip.blank(width)
        data, src_w, src_h, _ = self._snap
        return self._image.line(data, src_w, src_h, width, height, y)


class ThumbGrid(ScrollView, can_focus=True):
    """Alternative "SUITE" display for playlist mode: the *whole* queue as a
    scrollable grid of thumbnails instead of a text list. The video list is
    set once and never re-sliced as playback advances — nothing reflows
    under the user while they're browsing it, only the thumbnails already on
    screen get decoded. `playing_id` (ember) and `selected` (amber) are
    tracked independently, same as `VideoItem` in the text list — the track
    coming out of the speakers and the keyboard/mouse cursor are often not
    the same cell.

    Built on `ScrollView` (the same base `Log`/`RichLog` use for a custom
    Line API + real scrolling) instead of a bare `Widget`, so mouse-wheel and
    Home/End/PageUp/PageDown scrolling come from Textual itself rather than
    a hand-rolled offset — `virtual_size` declares the full content height,
    `render_line` reads `self.scroll_offset` for which row to draw, and only
    the rows actually inside the viewport ever get their thumbnail
    requested (`_sync_visible`), so a long playlist never decodes more than
    what's on screen.

    Each cell is decoded once by ffmpeg (`player.decode_thumbnail`) into the
    exact pixel format `Clip` already knows how to draw, and all cells share
    one `HalfBlockImage` since every cell samples the same (VID_W, VID_H)
    source into the same (CELL_W, IMAGE_H) target — identical sampling
    tables, only the pixel buffer changes.
    """

    CELL_W = 24
    IMAGE_H = 7
    BORDER_ROW = 0
    TITLE_ROW = IMAGE_H + 1
    CELL_H = TITLE_ROW + 1      # border/highlight bar + image + title line
    COL_GAP = 1

    def __init__(self, **kwargs):
        super().__init__(**kwargs)
        self.videos: list[Video] = []
        self.frames: dict[str, bytes] = {}
        self.selected = 0
        self.playing_id: str | None = None
        self._image = HalfBlockImage()
        self._columns = 1

    # ----------------------------------------------------------------- data

    def set_videos(self, videos: list[Video]) -> None:
        self.videos = videos
        # Drop frames for anything no longer in the list: keeps this
        # widget's RAM bounded to what's actually browsable, not whatever
        # accumulated across a long session.
        ids = {v.id for v in videos}
        self.frames = {vid: frame for vid, frame in self.frames.items() if vid in ids}
        self.selected = min(self.selected, max(0, len(videos) - 1))
        self._update_virtual_size()
        self._sync_visible()
        self.refresh()

    def set_frame(self, video_id: str, frame: bytes | None) -> None:
        if frame is not None:
            self.frames[video_id] = frame
            self.refresh()

    def set_playing(self, video_id: str | None) -> None:
        if video_id != self.playing_id:
            self.playing_id = video_id
            self.refresh()

    def clear(self) -> None:
        self.videos = []
        self.frames = {}
        self.selected = 0
        self.playing_id = None
        self.scroll_to(y=0, animate=False, immediate=True)
        self._update_virtual_size()
        self.refresh()

    def scroll_to_index(self, index: int) -> None:
        """Position the selection (and, once sized, the viewport) on
        `index` — used once, when grid mode is entered, to start near the
        upcoming track instead of the top of the playlist."""
        if not self.videos:
            return
        self.selected = min(max(0, index), len(self.videos) - 1)
        self._ensure_visible()

    # ------------------------------------------------------------- geometry

    def _layout(self, width: int) -> int:
        columns = max(1, (width + self.COL_GAP) // (self.CELL_W + self.COL_GAP))
        self._columns = columns
        return columns

    def _update_virtual_size(self) -> None:
        width = self.size.width
        if width <= 0:
            return
        columns = self._layout(width)
        rows = -(-len(self.videos) // columns) if self.videos else 0
        self.virtual_size = Size(width, rows * self.CELL_H)

    def _ensure_visible(self) -> None:
        """Scrolls just enough to bring the selected cell into view —
        called after keyboard navigation and once geometry is known."""
        if self._columns <= 0 or self.size.height <= 0:
            return
        row = self.selected // self._columns
        top, bottom = row * self.CELL_H, (row + 1) * self.CELL_H
        view_top = self.scroll_offset.y
        view_bottom = view_top + self.size.height
        if top < view_top:
            self.scroll_to(y=top, animate=False)
        elif bottom > view_bottom:
            self.scroll_to(y=bottom - self.size.height, animate=False)
        self._sync_visible()

    def _sync_visible(self) -> None:
        """Requests thumbnails only for rows currently in the viewport —
        the point of switching to a real scrollable list instead of a
        preloaded window: a 300-track playlist never decodes more than a
        screenful."""
        width, height = self.size.width, self.size.height
        if width <= 0 or height <= 0 or not self.videos:
            return
        columns = self._layout(width)
        scroll_y = self.scroll_offset.y
        first_row = max(0, scroll_y // self.CELL_H)
        last_row = (scroll_y + height) // self.CELL_H
        start = first_row * columns
        end = min(len(self.videos), (last_row + 1) * columns)
        missing = [v for v in self.videos[start:end] if v.id not in self.frames]
        if missing:
            request = getattr(self.app, "request_thumbs", None)
            if request is not None:
                request(missing)

    def watch_scroll_y(self, old_value: float, new_value: float) -> None:
        super().watch_scroll_y(old_value, new_value)
        self._sync_visible()

    def _on_resize(self, event) -> None:
        self._update_virtual_size()
        self._ensure_visible()

    def _activate(self, index: int) -> None:
        if not (0 <= index < len(self.videos)):
            return
        callback = getattr(self.app, "play_from_thumb", None)
        if callback is not None:
            callback(self.videos[index])

    # ------------------------------------------------------------------ i/o

    def _on_click(self, event: events.Click) -> None:
        """A single click only selects — playback needs a double click, the
        same threshold `VideoItem` uses in the text list — so that browsing
        the grid with the mouse can't fire a track by accident."""
        if not self.videos or self._columns <= 0:
            return
        content_y = event.y + self.scroll_offset.y
        grid_row = content_y // self.CELL_H
        col = event.x // (self.CELL_W + self.COL_GAP)
        index = grid_row * self._columns + col
        if not (0 <= index < len(self.videos)):
            return
        self.selected = index
        self.refresh()
        if event.chain >= 2:
            self._activate(index)

    def _on_key(self, event: events.Key) -> None:
        if not self.videos or event.key not in ("left", "right", "up", "down", "enter"):
            return
        n = len(self.videos)
        if event.key == "left":
            self.selected = max(0, self.selected - 1)
        elif event.key == "right":
            self.selected = min(n - 1, self.selected + 1)
        elif event.key == "up":
            self.selected = max(0, self.selected - self._columns)
        elif event.key == "down":
            self.selected = min(n - 1, self.selected + self._columns)
        else:
            event.prevent_default()
            event.stop()
            self._activate(self.selected)
            return
        event.prevent_default()
        event.stop()
        self._ensure_visible()
        self.refresh()

    # -------------------------------------------------------------- render

    def _border_segments(self, index: int) -> list[Segment]:
        # A solid bright bar reads as "selected"/"playing" regardless of how
        # busy the thumbnail under it is — a tinted background alone got
        # lost next to a colourful image, which is the whole reason this row
        # exists. Playing (ember, matches the "▶" marker elsewhere in the
        # app) wins over the keyboard/mouse cursor (amber) when a track is
        # both — two different questions, two different colours.
        if self.videos[index].id == self.playing_id:
            return [Segment(" " * self.CELL_W, Style(bgcolor="#ff6b35"))]
        if index == self.selected:
            return [Segment(" " * self.CELL_W, Style(bgcolor="#e09b2a"))]
        return [Segment(" " * self.CELL_W)]

    def _title_segments(self, index: int, width: int) -> list[Segment]:
        video = self.videos[index]
        if video.id == self.playing_id:
            bg, marker, marker_color = "on #3d2015", "▶ ", "#ff6b35"
            title_style = "bold #fff6e6"
        elif index == self.selected:
            bg, marker, marker_color = "on #5a3a12", "▌ ", "#ffb454"
            title_style = "bold #fff6e6"
        else:
            bg, marker, marker_color = "", "  ", "#191615"
            title_style = "#cbc0b2"

        room = max(0, width - len(marker))
        title = video.title
        if len(title) > room:
            title = title[: max(0, room - 1)] + "…" if room else ""
        title = title.ljust(room)
        return [
            Segment(marker, Style.parse(f"{marker_color} {bg}")),
            Segment(title, Style.parse(f"{title_style} {bg}")),
        ]

    def _cell_segments(self, index: int, row_in_cell: int) -> list[Segment]:
        blank = [Segment(" " * self.CELL_W)]
        if not (0 <= index < len(self.videos)):
            return blank
        if row_in_cell == self.BORDER_ROW:
            return self._border_segments(index)
        if row_in_cell == self.TITLE_ROW:
            return self._title_segments(index, self.CELL_W)
        frame = self.frames.get(self.videos[index].id)
        if frame is None:
            return blank
        return self._image.segments(
            frame, VID_W, VID_H, self.CELL_W, self.IMAGE_H, row_in_cell - 1
        )

    def render_line(self, y: int) -> Strip:
        width, height = self.size.width, self.size.height
        if width <= 0 or height <= 0:
            return Strip.blank(width)
        columns = self._layout(width)

        if not self.videos:
            if y == height // 2:
                text = "aucune piste suivante"
                pad = max(0, (width - len(text)) // 2)
                return Strip([Segment(" " * pad + text.ljust(width - pad),
                                       Style(color="#6b6055"))], width).simplify()
            return Strip.blank(width)

        content_y = y + self.scroll_offset.y
        row_in_cell = content_y % self.CELL_H
        grid_row = content_y // self.CELL_H
        cell_span = columns * self.CELL_W + (columns - 1) * self.COL_GAP

        segments: list[Segment] = []
        for col in range(columns):
            if col:
                segments.append(Segment(" " * self.COL_GAP))
            index = grid_row * columns + col
            segments.extend(self._cell_segments(index, row_in_cell))
        if cell_span < width:
            segments.append(Segment(" " * (width - cell_span)))
        return Strip(segments, width).simplify()
