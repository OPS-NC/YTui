"""ytui — a lean, audio-only YouTube client for the terminal."""

from __future__ import annotations

from textual import on, work
from textual.app import App, ComposeResult
from textual.containers import Horizontal, Vertical
from textual.css.query import NoMatches
from textual.widgets import Footer, Input, ListView, Static

from . import meminfo, sources
from .player import Player
from .sources import Video
from .widgets import SeekBar, Spectrum, VideoItem

FPS = 20


class YtuiApp(App):
    CSS_PATH = "app.tcss"
    TITLE = "ytui"

    BINDINGS = [
        ("space", "toggle_pause", "Pause"),
        ("n", "next_track", "Suivant"),
        ("left", "seek_back", "-10s"),
        ("right", "seek_fwd", "+10s"),
        ("plus,equals_sign", "volume_up", "Vol +"),
        ("minus", "volume_down", "Vol -"),
        ("slash", "focus_search", "Recherche"),
        ("s", "stop", "Stop"),
        ("q", "quit", "Quitter"),
    ]

    def __init__(self):
        super().__init__()
        self.player = Player(
            on_finished=lambda: self.call_from_thread(self._on_track_finished),
            on_error=lambda exc: self.call_from_thread(self._notify_error, exc),
        )
        self.current: Video | None = None

    # ------------------------------------------------------------------ view

    def compose(self) -> ComposeResult:
        yield Static("◈  y t u i   ·   youtube sans image", id="brand")
        yield Input(placeholder="Rechercher sur YouTube…", id="search")
        with Horizontal(id="body"):
            with Vertical(id="left"):
                yield Static("RÉSULTATS", classes="pane-title")
                yield ListView(id="results")
            with Vertical(id="right"):
                yield Static("En attente…", id="now-title")
                yield Static("", id="now-sub")
                yield Spectrum(self.player, id="spectrum")
                yield SeekBar(id="seek")
                yield Static("SUGGESTIONS  ·  lecture auto", classes="pane-title")
                yield ListView(id="suggestions")
        with Horizontal(id="bottom"):
            yield Footer()
            yield Static("", id="mem")

    def on_mount(self) -> None:
        self.query_one("#search", Input).focus()
        self.set_interval(1 / FPS, self._tick)
        self.set_interval(1.0, self._tick_mem)
        self._tick_mem()

    def _tick_mem(self) -> None:
        backend = self.player.backend if self.player.loaded else "idle"
        try:
            widget = self.query_one("#mem", Static)
        except NoMatches:
            return
        widget.update(
            f"audio: {backend}   ·   RAM {meminfo.human(meminfo.total_rss())}"
        )

    def _tick(self) -> None:
        try:
            spectrum = self.query_one(Spectrum)
            seek = self.query_one(SeekBar)
        except NoMatches:   # fired while the screen is being torn down
            return
        if self.player.loaded or any(spectrum._bands or []):
            spectrum.poll()
        seek.position = self.player.position
        seek.duration = self.player.duration or 0.0
        seek.paused = self.player.paused

    # --------------------------------------------------------------- search

    @on(Input.Submitted, "#search")
    def _submit(self, event: Input.Submitted) -> None:
        query = event.value.strip()
        if query:
            self._status(f"Recherche « {query} »…")
            self.run_search(query)

    @work(thread=True, exclusive=True, group="search")
    def run_search(self, query: str) -> None:
        try:
            videos = sources.search(query)
        except Exception as exc:
            self.call_from_thread(self._notify_error, exc)
            return
        self.call_from_thread(self._fill, "#results", videos, True)

    def _fill(self, selector: str, videos: list[Video], numbered: bool) -> None:
        view = self.query_one(selector, ListView)
        view.clear()
        for i, video in enumerate(videos, 1):
            view.append(VideoItem(video, i if numbered else None))
        if selector == "#results":
            self._status(f"{len(videos)} résultats")
            if videos:
                view.focus()
                view.index = 0

    # ------------------------------------------------------------- playback

    @on(ListView.Selected)
    def _selected(self, event: ListView.Selected) -> None:
        item = event.item
        if isinstance(item, VideoItem):
            self.play_video(item.video)

    def play_video(self, video: Video) -> None:
        self.current = video
        self.query_one("#now-title", Static).update(f"♪  {video.title}")
        self.query_one("#now-sub", Static).update(
            f"{video.uploader}   ·   chargement du flux audio…"
        )
        self.start_stream(video)
        self.load_suggestions(video)

    @work(thread=True, exclusive=True, group="stream")
    def start_stream(self, video: Video) -> None:
        try:
            sources.resolve_audio(video)
            self.player.play(video.stream_url, video.duration)
        except Exception as exc:
            self.call_from_thread(self._notify_error, exc)
            return
        self.call_from_thread(
            self.query_one("#now-sub", Static).update,
            f"{video.uploader}   ·   audio seul   ·   {video.duration_str}",
        )

    @work(thread=True, exclusive=True, group="related")
    def load_suggestions(self, video: Video) -> None:
        try:
            videos = sources.related(video)
        except Exception:
            return
        self.call_from_thread(self._fill, "#suggestions", videos, False)

    def _on_track_finished(self) -> None:
        self.action_next_track()

    def action_next_track(self) -> None:
        view = self.query_one("#suggestions", ListView)
        for item in view.children:
            if isinstance(item, VideoItem):
                self.play_video(item.video)
                return
        self._status("Aucune suggestion à enchaîner.")

    def action_toggle_pause(self) -> None:
        if self.player.loaded:
            self.player.toggle_pause()

    def action_seek_back(self) -> None:
        self.player.seek(-10)

    def action_seek_fwd(self) -> None:
        self.player.seek(10)

    def action_volume_up(self) -> None:
        self.player.volume = min(1.5, self.player.volume + 0.1)
        self._status(f"Volume {self.player.volume:.0%}")

    def action_volume_down(self) -> None:
        self.player.volume = max(0.0, self.player.volume - 0.1)
        self._status(f"Volume {self.player.volume:.0%}")

    def action_stop(self) -> None:
        self.player.stop()
        self.query_one("#now-title", Static).update("En attente…")
        self.query_one("#now-sub", Static).update("")

    def action_focus_search(self) -> None:
        self.query_one("#search", Input).focus()

    # ----------------------------------------------------------------- misc

    def _status(self, message: str) -> None:
        self.query_one("#brand", Static).update(
            f"◈  y t u i   ·   {message}"
        )

    def _notify_error(self, exc: Exception) -> None:
        self.notify(str(exc) or exc.__class__.__name__, severity="error", timeout=6)

    def on_unmount(self) -> None:
        self.player.stop()


def main() -> None:
    YtuiApp().run()
