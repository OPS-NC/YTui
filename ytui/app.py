"""ytui — a lean, audio-only YouTube client for the terminal."""

from __future__ import annotations

from time import monotonic

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
CLICK_WINDOW = 0.8      # seconds a click keeps authority over ListView.Selected


class YtuiApp(App):
    CSS_PATH = "app.tcss"
    TITLE = "ytui"
    # One theme only: every colour is fixed in app.tcss. This also drops the
    # command palette, whose sole use here was switching themes.
    ENABLE_COMMAND_PALETTE = False

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
            on_attempt=lambda n, total: self.call_from_thread(
                self._status, f"URL refusée, nouvelle tentative {n}/{total}"
            ),
        )
        self.current: Video | None = None
        self._click_stamp: tuple[int, float, int] = (0, 0.0, 0)

    # ------------------------------------------------------------------ view

    def compose(self) -> ComposeResult:
        with Horizontal(id="topbar"):
            yield Static("y t u i", id="brand")
            yield Static("TUNER · AUDIO SEUL", id="status")
        yield Input(
            placeholder="Rechercher, ou coller une URL / un ID de vidéo…",
            id="search",
        )
        with Horizontal(id="body"):
            with Vertical(id="left"):
                yield ListView(id="results")
            with Vertical(id="right"):
                with Vertical(id="deck"):
                    yield Static("— aucune piste —", id="now-title")
                    yield Static("prêt", id="now-sub")
                    yield Spectrum(self.player, id="spectrum")
                    yield SeekBar(id="seek")
                yield ListView(id="suggestions")
        with Horizontal(id="bottom"):
            yield Footer()
            yield Static("", id="mem")

    def on_mount(self) -> None:
        # Inline border titles: the panel frame doubles as its own label.
        self.query_one("#results", ListView).border_title = "R É S U L T A T S"
        self.query_one("#suggestions", ListView).border_title = "S U I T E   ·   auto"
        self.query_one("#deck", Vertical).border_title = "P L A T I N E"
        self.query_one("#search", Input).border_title = "R E C H E R C H E"
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
        if not query:
            return
        video_id = sources.parse_video_id(query)
        if video_id:
            self._status("Lien reconnu, ouverture…")
            self.play_video(Video(id=video_id, title=f"youtube.com/watch?v={video_id}"))
            return
        self._status(f"Recherche « {query} »…")
        self.run_search(query)

    @work(thread=True, exclusive=True, group="search")
    def run_search(self, query: str) -> None:
        try:
            videos = sources.search(query)
        except Exception as exc:
            self.call_from_thread(self._notify_error, exc)
            return
        self.call_from_thread(self._fill, "#results", videos)

    def _fill(self, selector: str, videos: list[Video]) -> None:
        view = self.query_one(selector, ListView)
        view.clear()
        for video in videos:
            view.append(VideoItem(video))
        if selector == "#results":
            self._status(f"{len(videos)} résultats")
            if videos:
                view.focus()
                view.index = 0

    # ------------------------------------------------------------- playback

    @on(ListView.Selected)
    def _selected(self, event: ListView.Selected) -> None:
        item = event.item
        if not isinstance(item, VideoItem):
            return

        # One mouse click makes ListView post Selected twice, and only the
        # first carries the click count — so the decision is taken on a
        # timestamp, not on a one-shot flag.
        target, when, chain = self._click_stamp
        if target == id(item) and monotonic() - when < CLICK_WINDOW:
            if chain < 2:
                self._status("double-clic pour lire")
                return
            # Consume the double click so its second Selected is ignored.
            self._click_stamp = (target, when, 0)
        self.play_video(item.video)

    def note_click(self, item: VideoItem, chain: int) -> None:
        """Called by VideoItem: records which row was clicked, how many times."""
        self._click_stamp = (id(item), monotonic(), chain)

    def play_video(self, video: Video) -> None:
        self.current = video
        self.query_one("#now-title", Static).update(video.title)
        self.query_one("#now-sub", Static).update(
            f"{video.uploader or '—'}  ·  ouverture du flux…"
        )
        self.start_stream(video)
        self.load_suggestions(video)

    @work(thread=True, exclusive=True, group="stream")
    def start_stream(self, video: Video) -> None:
        def provider(refresh: bool) -> tuple[str | None, dict]:
            """Called again by the player on every failed attempt, so each
            retry gets a freshly signed URL rather than the dead one."""
            if refresh or not video.stream_url:
                sources.resolve_audio(video)
            return video.stream_url, video.headers

        try:
            provider(False)
            self.player.play(provider, video.duration)
        except Exception as exc:
            self.call_from_thread(self._notify_error, exc)
            return
        # A pasted link starts with a placeholder title; resolving fills it in.
        self.call_from_thread(
            self.query_one("#now-title", Static).update, video.title
        )
        self.call_from_thread(
            self.query_one("#now-sub", Static).update,
            f"{video.uploader or '—'}  ·  {video.duration_str}  ·  "
            f"{video.source or 'audio seul'}  ·  {self.player.backend}",
        )
        self.call_from_thread(self._status, "lecture")

    @work(thread=True, exclusive=True, group="related")
    def load_suggestions(self, video: Video) -> None:
        try:
            videos = sources.related(video)
        except Exception:
            return
        self.call_from_thread(self._fill, "#suggestions", videos)

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
        self.player.set_volume(self.player.volume + 0.1)
        self._status(f"Volume {self.player.volume:.0%}")

    def action_volume_down(self) -> None:
        self.player.set_volume(self.player.volume - 0.1)
        self._status(f"Volume {self.player.volume:.0%}")

    def action_stop(self) -> None:
        self.player.stop()
        self.query_one("#now-title", Static).update("— aucune piste —")
        self.query_one("#now-sub", Static).update("prêt")
        self._status("arrêt")

    def action_focus_search(self) -> None:
        self.query_one("#search", Input).focus()

    # ----------------------------------------------------------------- misc

    def _status(self, message: str) -> None:
        try:
            self.query_one("#status", Static).update(message.upper())
        except NoMatches:
            pass

    def _notify_error(self, exc: Exception) -> None:
        self.notify(str(exc) or exc.__class__.__name__, severity="error", timeout=6)

    def on_unmount(self) -> None:
        self.player.stop()


def main() -> None:
    YtuiApp().run()
