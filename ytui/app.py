"""ytui — a lean, audio-only YouTube client for the terminal."""

from __future__ import annotations

from time import monotonic

from textual import on, work
from textual.app import App, ComposeResult
from textual.containers import Horizontal, Vertical
from textual.css.query import NoMatches
from textual.widgets import Footer, Input, ListView, Static

from . import meminfo, sources
from .history import SearchHistory
from .player import Player
from .sources import Video
from .widgets import SearchInput, Clip, SeekBar, Spectrum, VideoItem

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
        ("v", "toggle_clip", "Vidéo"),
        ("V,shift+v", "toggle_fullscreen", "Plein écran"),
        ("escape", "clip_small", "Réduire"),
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
        self.history = SearchHistory()
        self.current: Video | None = None
        # File d'attente chargée depuis une playlist : tant qu'elle est là, la
        # colonne « suite » ne se recharge plus toute seule.
        self.queue: list[Video] = []
        self.queue_index: int = -1
        self.deck_video = False     # image à la place du spectre dans la platine
        self.fullscreen = False
        self._click_stamp: tuple[int, float, int] = (0, 0.0, 0)

    # ------------------------------------------------------------------ view

    def compose(self) -> ComposeResult:
        with Horizontal(id="topbar"):
            yield Static("y t u i", id="brand")
            yield Static("TUNER · AUDIO SEUL", id="status")
        yield SearchInput(
            self.history,
            placeholder="Rechercher, ou coller une URL / un ID de vidéo ou de playlist…  (↑ historique)",
            id="search",
        )
        with Horizontal(id="body"):
            with Vertical(id="left"):
                yield ListView(id="results")
            with Vertical(id="right"):
                with Vertical(id="deck"):
                    yield Static("— aucune piste —", id="now-title")
                    yield Static("prêt", id="now-sub")
                    # Analyseur par défaut ; « v » met l'image à sa place,
                    # « V » l'envoie en plein écran.
                    yield Clip(self.player, id="clip")
                    yield Spectrum(self.player, id="spectrum")
                    yield SeekBar(id="seek")
                yield ListView(id="suggestions")
        with Horizontal(id="bottom"):
            yield Footer()
            yield Static("", id="mem")
        # Full-screen clip: hidden until the picture is clicked, transport laid
        # over it; « Échap » comes back to the deck.
        with Vertical(id="clip-layer"):
            yield Clip(self.player, id="clip-full")
            yield Static("", id="clip-title")
            yield SeekBar(id="clip-seek")

    def on_mount(self) -> None:
        # Inline border titles: the panel frame doubles as its own label.
        self.query_one("#results", ListView).border_title = "R É S U L T A T S"
        self.query_one("#suggestions", ListView).border_title = "S U I T E   ·   auto"
        self.query_one("#deck", Vertical).border_title = "P L A T I N E"
        self.query_one("#search", Input).border_title = "R E C H E R C H E"
        self.query_one("#search", Input).focus()
        self.query_one("#clip-layer", Vertical).display = False
        self._apply_deck()
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
            spectrum = self.query_one("#spectrum", Spectrum)
            bars = list(self.query(SeekBar))
        except NoMatches:   # fired while the screen is being torn down
            return
        if self.fullscreen:
            self.query_one("#clip-full", Clip).poll()
        elif self.deck_video:
            self.query_one("#clip", Clip).poll()
        elif self.player.loaded or any(spectrum._bands or []):
            spectrum.poll()
        for seek in bars:
            seek.position = self.player.position
            seek.duration = self.player.duration or 0.0
            seek.paused = self.player.paused

    def _set_results_visible(self, visible: bool) -> None:
        """Affiche la colonne de résultats uniquement pendant une recherche."""
        try:
            self.query_one("#left").display = visible
        except NoMatches:
            # A search/playback callback can race with screen teardown.
            return

    # --------------------------------------------------------------- search

    @on(Input.Submitted, "#search")
    def _submit(self, event: Input.Submitted) -> None:
        query = event.value.strip()
        if not query:
            return
        self.history.add(query)
        playlist_id = sources.parse_playlist_id(query)
        if playlist_id:
            self._set_results_visible(False)
            self._status("Playlist reconnue, chargement…")
            self.load_playlist(playlist_id, sources.parse_video_id(query))
            return
        video_id = sources.parse_video_id(query)
        if video_id:
            self._set_results_visible(False)
            self._status("Lien reconnu, ouverture…")
            self.play_video(Video(id=video_id, title=f"youtube.com/watch?v={video_id}"))
            return
        self._set_results_visible(True)
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

        # Une piste choisie dans la file garde la playlist ; un résultat de
        # recherche la remplace par les suggestions automatiques.
        if self.queue and event.list_view.id == "suggestions":
            index = next((i for i, v in enumerate(self.queue)
                          if v.id == item.video.id), None)
            if index is not None:
                self._play_queue_index(index)
                return
        self.play_video(item.video)

    def note_click(self, item: VideoItem, chain: int) -> None:
        """Called by VideoItem: records which row was clicked, how many times."""
        self._click_stamp = (id(item), monotonic(), chain)

    # --------------------------------------------------------------- queue

    @work(thread=True, exclusive=True, group="playlist")
    def load_playlist(self, playlist_id: str, start_id: str | None = None) -> None:
        try:
            videos = sources.playlist(playlist_id)
        except Exception as exc:
            self.call_from_thread(self._notify_error, exc)
            return
        if not videos:
            self.call_from_thread(self._status, "playlist vide")
            return
        start = 0
        if start_id:
            start = next((i for i, v in enumerate(videos) if v.id == start_id), 0)
        self.call_from_thread(self._install_queue, videos, start)

    def _install_queue(self, videos: list[Video], start: int) -> None:
        self.queue = videos
        self._fill("#suggestions", videos)
        self.query_one("#suggestions", ListView).border_title = (
            f"S U I T E   ·   playlist ({len(videos)})"
        )
        self._status(f"playlist : {len(videos)} pistes")
        self._play_queue_index(start)

    def _play_queue_index(self, index: int) -> None:
        if not (0 <= index < len(self.queue)):
            self._status("fin de la playlist")
            return
        self.queue_index = index
        view = self.query_one("#suggestions", ListView)
        if index < len(view.children):
            view.index = index
        self.play_video(self.queue[index], keep_queue=True)

    def _clear_queue(self) -> None:
        if not self.queue:
            return
        self.queue = []
        self.queue_index = -1
        self.query_one("#suggestions", ListView).border_title = "S U I T E   ·   auto"

    def play_video(self, video: Video, keep_queue: bool = False) -> None:
        self._set_results_visible(False)
        if not keep_queue:
            self._clear_queue()
        self.current = video
        if self.fullscreen:
            self._refresh_clip_title()
        self.query_one("#now-title", Static).update(video.title)
        self.query_one("#now-sub", Static).update(
            f"{video.uploader or '—'}  ·  ouverture du flux…"
        )
        # Une piste enchaînée garde l'image si elle est déjà affichée.
        self.start_stream(video, want_video=self.deck_video or self.fullscreen)
        if not keep_queue:
            self.load_suggestions(video)

    @work(thread=True, exclusive=True, group="stream")
    def start_stream(
        self, video: Video, start: float = 0.0, want_video: bool = False
    ) -> None:
        # Audio seul par défaut : la piste vidéo n'est décodée que si « v » l'a
        # demandée, ce qui rouvre le flux à la position courante.

        def provider(refresh: bool) -> tuple[str | None, dict]:
            """Called again by the player on every failed attempt, so each
            retry gets a freshly signed URL rather than the dead one. A clip
            asked for over an audio-only URL also forces a re-resolve."""
            if refresh or not video.stream_url or (want_video and not video.has_video):
                sources.resolve_audio(video, want_video=want_video)
            return video.stream_url, video.headers

        try:
            provider(False)
            self.player.video = want_video and video.has_video
            self.player.play(provider, video.duration, start)
        except Exception as exc:
            self.call_from_thread(self._notify_error, exc)
            return
        if want_video and not video.has_video:
            self.call_from_thread(
                self.notify, "Aucune piste vidéo sur ce flux — spectre affiché.",
                severity="warning",
            )
            self.call_from_thread(self._no_picture)
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
        if self.queue:
            self._play_queue_index(self.queue_index + 1)
            return
        view = self.query_one("#suggestions", ListView)
        for item in view.children:
            if isinstance(item, VideoItem):
                self.play_video(item.video)
                return
        self._status("Aucune suggestion à enchaîner.")

    def _apply_deck(self) -> None:
        """Image ou analyseur dans la platine, selon l'état de la bascule."""
        self.query_one("#clip", Clip).display = self.deck_video
        self.query_one("#spectrum", Spectrum).display = not self.deck_video

    def _ensure_video_stream(self) -> bool:
        """Rouvre le flux courant avec sa piste vidéo si besoin. False = rien
        à afficher (aucune piste en cours)."""
        if self.current is None:
            self.notify("Aucune piste en cours.", severity="warning", timeout=3)
            return False
        if not self.player.video:
            self._status("ouverture de la vidéo…")
            self.start_stream(self.current, self.player.position, want_video=True)
        return True

    def action_toggle_clip(self) -> None:
        """« v » : l'image prend la place du spectre dans la platine, et un
        second appui rend la platine à l'analyseur."""
        if self.deck_video:
            self.deck_video = False
            self._apply_deck()
            return
        if not self._ensure_video_stream():
            return
        self.deck_video = True
        self._apply_deck()

    def action_toggle_fullscreen(self) -> None:
        """« V » (maj + v), ou un clic sur l'image : plein écran."""
        if self.fullscreen:
            self._leave_fullscreen()
            return
        if not self._ensure_video_stream():
            return
        self._set_fullscreen(True)
        self._refresh_clip_title()
        self.notify("Plein écran — « Échap » pour revenir", timeout=3)

    def _no_picture(self) -> None:
        """Flux sans piste vidéo : retour à l'analyseur, dans la platine comme
        en plein écran."""
        self.deck_video = False
        self._apply_deck()
        self._leave_fullscreen()

    def _leave_fullscreen(self) -> None:
        if not self.fullscreen:
            return
        self._set_fullscreen(False)
        self.notify("Retour à la platine", timeout=2)

    def _set_fullscreen(self, value: bool) -> None:
        self.fullscreen = value
        for selector in ("#topbar", "#search", "#body", "#bottom"):
            self.query_one(selector).display = not value
        self.query_one("#clip-layer", Vertical).display = value

    def action_clip_small(self) -> None:
        """« Échap » only ever shrinks: it never opens the full screen."""
        self._leave_fullscreen()

    def _refresh_clip_title(self) -> None:
        video = self.current
        if video is None:
            self.query_one("#clip-title", Static).update("— aucune piste —")
            return
        self.query_one("#clip-title", Static).update(
            f"{video.title}   ·   {video.uploader or '—'}   ·   {video.source or ''}"
        )

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
