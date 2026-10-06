//! ytui — a lean, audio-first YouTube client for the terminal.
//!
//! `App` owns the whole UI state and is only ever touched from the main
//! thread. Blocking work (yt-dlp, network, ffmpeg start-up) runs on worker
//! threads that report back through the `Msg` channel; every reply carries
//! the token of the request it answers, so a slow reply for something the
//! user has since moved past is simply dropped.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::exec;
use crate::history::SearchHistory;
use crate::meminfo;
use crate::player::{self, Analyser, Player, PlayerEvent, Provider};
use crate::sources::{self, Meta, Stream, Video};
use crate::theme as t;
use crate::widgets::{self as w, HalfBlock, TextInput, ThumbGrid, VList, bold, fg, spans};

pub const FPS_PERIOD: Duration = Duration::from_millis(50); // 20 fps for the analyser
const MEM_PERIOD: Duration = Duration::from_secs(1);
const DOUBLE_CLICK: Duration = Duration::from_millis(500);
const DECK_H: u16 = 20; // border + padding + title/sub + 11-row meter + seek bar
const METER_H: u16 = 11;

const PLACEHOLDER: &str =
    "Rechercher, ou coller une URL / un ID de vidéo ou de playlist…  (↑ historique)";
const TITLE_RESULTS: &str = "R É S U L T A T S";
const TITLE_HOME: &str = "S U G G E S T I O N S";
const TITLE_AUTO: &str = "S U I T E   ·   auto";
const NO_TRACK: &str = "— aucune piste —";

pub enum Msg {
    Term(Event),
    Listing { token: u64, home: bool, result: Result<Vec<Video>, String> },
    Playlist { token: u64, start_id: Option<String>, result: Result<Vec<Video>, String> },
    Related { seq: u64, result: Result<Vec<Video>, String> },
    Stream { token: u64, result: Result<StreamInfo, String> },
    Thumb { id: String, frame: Vec<u8> },
    Player(PlayerEvent),
    Notice(String),
}

pub struct StreamInfo {
    meta: Meta,
    source: String,
    has_video: bool,
    want_video: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Search,
    Results,
    Suggestions,
    Grid,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListId {
    Results,
    Suggestions,
}

#[derive(Clone, Copy)]
enum Action {
    TogglePause,
    Next,
    SeekBack,
    SeekFwd,
    VolUp,
    VolDown,
    Clip,
    Fullscreen,
    Thumbs,
    Login,
    ClipSmall,
    FocusSearch,
    Stop,
    Quit,
}

/// The footer, in display order — also clickable, like Textual's.
const BINDINGS: [(&str, &str, Action); 14] = [
    ("space", "Pause", Action::TogglePause),
    ("n", "Suivant", Action::Next),
    ("←", "-10s", Action::SeekBack),
    ("→", "+10s", Action::SeekFwd),
    ("+", "Vol +", Action::VolUp),
    ("-", "Vol -", Action::VolDown),
    ("v", "Vidéo", Action::Clip),
    ("V", "Plein écran", Action::Fullscreen),
    ("t", "Miniatures", Action::Thumbs),
    ("L", "Connexion", Action::Login),
    ("esc", "Réduire", Action::ClipSmall),
    ("/", "Recherche", Action::FocusSearch),
    ("s", "Stop", Action::Stop),
    ("q", "Quitter", Action::Quit),
];

struct Toast {
    text: String,
    until: Instant,
    rect: Rect,
}

#[derive(Default)]
struct Hits {
    search: Rect,
    deck_clip: Rect,
    full_clip: Rect,
    suggestions: Rect,
    footer: Vec<(Rect, Action)>,
}

/// Per-group request counters: Textual's `exclusive=True` workers, minus the
/// part where a superseded thread still delivers its stale result.
#[derive(Default)]
struct Tokens {
    listing: u64,
    playlist: u64,
    thumbs: Arc<AtomicU64>,
    stream: Arc<AtomicU64>,
}

pub struct App {
    tx: Sender<Msg>,
    player: Player,
    history: SearchHistory,
    pub quit: bool,
    dirty: bool,

    focus: Focus,
    input: TextInput,
    results: VList,
    results_visible: bool,
    suggestions: VList,
    grid: ThumbGrid,
    hover: Option<(ListId, usize)>,
    last_click: Option<(ListId, usize, Instant)>,
    last_grid_click: Option<(usize, Instant)>,

    status: String,
    now_title: String,
    now_sub: String,
    mem: String,
    toasts: Vec<Toast>,

    current: Option<Video>,
    play_seq: u64,
    // The resolved stream of `current`, shared with the player's provider so
    // a retry or a clip toggle re-signs only when it has to.
    stream_cache: Arc<Mutex<Option<(Stream, Meta)>>>,
    source: String,
    // File d'attente chargée depuis une playlist : tant qu'elle est là, la
    // colonne « suite » ne se recharge plus toute seule.
    queue: Vec<Video>,
    queue_index: isize,
    deck_video: bool, // image à la place du spectre dans la platine
    fullscreen: bool,
    thumbs_mode: bool, // grille de miniatures à la place de la liste « suite »
    thumbs_requested: Vec<String>,

    analyser: Analyser,
    clip_small: HalfBlock,
    clip_full: HalfBlock,
    shown_frame: Option<u64>,
    shown_seek: (u64, u64, bool),
    next_anim: Instant,
    next_mem: Instant,
    tokens: Tokens,
    hits: Hits,
}

impl App {
    pub fn new(tx: Sender<Msg>) -> Self {
        let ptx = tx.clone();
        let player = Player::new(move |ev| {
            let _ = ptx.send(Msg::Player(ev));
        });
        let now = Instant::now();
        let mut app = Self {
            tx,
            player,
            history: SearchHistory::load(),
            quit: false,
            dirty: true,
            focus: Focus::Search,
            input: TextInput::default(),
            results: VList::new(TITLE_RESULTS),
            results_visible: true,
            suggestions: VList::new(TITLE_AUTO),
            grid: ThumbGrid::default(),
            hover: None,
            last_click: None,
            last_grid_click: None,
            status: "TUNER · AUDIO SEUL".into(),
            now_title: NO_TRACK.into(),
            now_sub: "prêt".into(),
            mem: String::new(),
            toasts: Vec::new(),
            current: None,
            play_seq: 0,
            stream_cache: Arc::default(),
            source: String::new(),
            queue: Vec::new(),
            queue_index: -1,
            deck_video: false,
            fullscreen: false,
            thumbs_mode: false,
            thumbs_requested: Vec::new(),
            analyser: Analyser::new(),
            clip_small: HalfBlock::default(),
            clip_full: HalfBlock::default(),
            shown_frame: None,
            shown_seek: (0, 0, false),
            next_anim: now,
            next_mem: now,
            tokens: Tokens::default(),
            hits: Hits::default(),
        };
        app.tick_mem();
        {
            // The managed yt-dlp: installed on first launch, refreshed daily.
            let tx = app.tx.clone();
            exec::spawn("yt-dlp-update", move || {
                crate::tools::maintain_ytdlp(|text| {
                    let _ = tx.send(Msg::Notice(text));
                });
            });
        }
        if crate::tools::js_runtime().is_none() {
            // Without one, yt-dlp cannot solve YouTube's `n` challenge: the
            // URLs it hands over are throttled below playback speed, and the
            // sound stalls and restarts every few seconds.
            app.notify(
                "Aucun moteur JavaScript (deno, node, bun, quickjs) : YouTube bridera \
                 les flux et la lecture saccadera — installez-en un.",
                10,
            );
        }
        if sources::cookie_browser().is_some() {
            // Only when already authenticated at launch — "L" mid-session
            // doesn't retrigger this, so it never clobbers whatever the user
            // is doing by then.
            app.results.title = TITLE_HOME.into();
            app.set_status("chargement des suggestions…");
            app.run_listing(None);
        }
        app
    }

    pub fn shutdown(&self) {
        self.player.stop();
    }

    // ------------------------------------------------------------ the loop

    pub fn needs_draw(&self) -> bool {
        self.dirty
    }

    /// When the main loop must wake up next if no message arrives first.
    /// Idle — nothing playing, meter at rest — that is once a second for the
    /// RAM readout, and nothing else.
    pub fn next_deadline(&self) -> Instant {
        let mut next = self.next_mem;
        if self.animating() {
            next = next.min(self.next_anim);
        }
        if let Some(toast) = self.toasts.iter().map(|t| t.until).min() {
            next = next.min(toast);
        }
        next
    }

    fn animating(&self) -> bool {
        self.player.playing() || (!self.fullscreen && !self.deck_video && self.analyser.active())
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        if now >= self.next_anim {
            self.next_anim = now + FPS_PERIOD;
            if self.animating() {
                self.animate();
            }
        }
        if now >= self.next_mem {
            self.next_mem = now + MEM_PERIOD;
            self.tick_mem();
        }
        let before = self.toasts.len();
        self.toasts.retain(|t| t.until > now);
        self.dirty |= self.toasts.len() != before;
    }

    fn animate(&mut self) {
        if self.fullscreen || self.deck_video {
            let no = self.player.frame_no();
            if no != self.shown_frame {
                self.shown_frame = no;
                self.dirty = true;
            }
        } else if self.player.loaded() || self.analyser.active() {
            let before = (self.analyser.bands, self.analyser.peaks);
            self.analyser.update(&self.player);
            self.dirty |= before != (self.analyser.bands, self.analyser.peaks);
        }
        let seek = (
            self.player.position() as u64,
            self.player.duration().unwrap_or(0.0) as u64,
            self.player.paused(),
        );
        if seek != self.shown_seek {
            self.shown_seek = seek;
            self.dirty = true;
        }
    }

    fn tick_mem(&mut self) {
        let backend = if self.player.loaded() { player::BACKEND } else { "idle" };
        let login = sources::cookie_browser()
            .map(|b| format!("   ·   connecté ({b})"))
            .unwrap_or_default();
        let text =
            format!("audio: {backend}   ·   RAM {}{login}", meminfo::human(meminfo::total_rss()));
        if text != self.mem {
            self.mem = text;
            self.dirty = true;
        }
    }

    pub fn handle(&mut self, msg: Msg) {
        // Handlers clear `dirty` for events that change nothing (a mouse
        // move within the same row…); a redraw already pending must survive.
        let pending = self.dirty;
        self.dirty = true;
        self.dispatch(msg);
        self.dirty |= pending;
    }

    fn dispatch(&mut self, msg: Msg) {
        match msg {
            Msg::Term(ev) => self.on_event(ev),
            Msg::Listing { token, home, result } => {
                if token != self.tokens.listing {
                    return;
                }
                match result {
                    Ok(videos) => {
                        let n = videos.len();
                        self.fill_results(videos);
                        if home {
                            self.set_status(&if n > 0 {
                                format!("{n} suggestions")
                            } else {
                                "aucune suggestion".into()
                            });
                        }
                    }
                    Err(e) => self.notify_error(&e),
                }
            }
            Msg::Playlist { token, start_id, result } => {
                if token != self.tokens.playlist {
                    return;
                }
                match result {
                    Err(e) => self.notify_error(&e),
                    Ok(videos) if videos.is_empty() => self.set_status("playlist vide"),
                    Ok(videos) => {
                        let start = start_id
                            .and_then(|id| videos.iter().position(|v| v.id == id))
                            .unwrap_or(0);
                        self.install_queue(videos, start);
                    }
                }
            }
            Msg::Related { seq, result } => self.suggestions_done(seq, result),
            Msg::Stream { token, result } => {
                if token != self.tokens.stream.load(Ordering::SeqCst) {
                    return;
                }
                match result {
                    Ok(info) => self.stream_started(info),
                    Err(e) => self.notify_error(&e),
                }
            }
            // A frame from a superseded worker is still a valid frame.
            Msg::Thumb { id, frame } => {
                if self.thumbs_mode {
                    self.grid.frames.insert(id, frame);
                }
            }
            Msg::Player(PlayerEvent::Finished) => self.action_next_track(),
            Msg::Player(PlayerEvent::Error(e)) => self.notify_error(&e),
            Msg::Player(PlayerEvent::Attempt(n, total)) => {
                self.set_status(&format!("URL refusée, nouvelle tentative {n}/{total}"))
            }
            Msg::Notice(text) => {
                self.set_status(&text);
                let mut chars = text.chars();
                let capitalised: String = chars.next().into_iter().flat_map(char::to_uppercase).chain(chars).collect();
                self.notify(&capitalised, 5);
            }
            Msg::Player(PlayerEvent::Resumed(at, reason)) => {
                let why = if reason.is_empty() { String::new() } else { format!(" ({reason})") };
                self.set_status(&format!("flux coupé à {}, reprise…", sources::fmt_time(at)));
                self.notify(&format!("Flux interrompu à {}{why} — reprise", sources::fmt_time(at)), 4);
            }
        }
    }

    // ------------------------------------------------------------- search

    fn submit(&mut self) {
        let query = self.input.value().trim().to_string();
        if query.is_empty() {
            return;
        }
        self.history.add(&query);
        if let Some(playlist_id) = sources::parse_playlist_id(&query) {
            self.set_results_visible(false);
            self.set_status("Playlist reconnue, chargement…");
            self.load_playlist(playlist_id, sources::parse_video_id(&query));
            return;
        }
        if let Some(video_id) = sources::parse_video_id(&query) {
            self.set_results_visible(false);
            self.set_status("Lien reconnu, ouverture…");
            let title = format!("youtube.com/watch?v={video_id}");
            self.play_video(Video::new(&video_id, title), false);
            return;
        }
        self.set_results_visible(true);
        self.results.title = TITLE_RESULTS.into();
        self.set_status(&format!("Recherche « {query} »…"));
        self.run_listing(Some(query));
    }

    /// Affiche la colonne de résultats uniquement pendant une recherche.
    fn set_results_visible(&mut self, visible: bool) {
        self.results_visible = visible;
        if !visible && self.focus == Focus::Results {
            self.focus = Focus::Search;
        }
    }

    /// A search, or the home feed when `query` is None. One group: a new
    /// search supersedes the launch-time suggestions and vice versa.
    fn run_listing(&mut self, query: Option<String>) {
        self.tokens.listing += 1;
        let (token, tx) = (self.tokens.listing, self.tx.clone());
        exec::spawn("listing", move || {
            let home = query.is_none();
            let result = match query {
                Some(q) => sources::search(&q),
                None => sources::home_feed(),
            };
            let _ = tx.send(Msg::Listing { token, home, result });
        });
    }

    fn fill_results(&mut self, videos: Vec<Video>) {
        let n = videos.len();
        self.results.set_items(videos);
        self.set_status(&format!("{n} résultats"));
        if n > 0 && self.results_visible {
            self.focus = Focus::Results;
            self.results.select(0);
        }
    }

    fn fill_suggestions(&mut self, videos: Vec<Video>) {
        self.suggestions.set_items(videos);
    }

    // ------------------------------------------------------------ playback

    /// Enter or double click on a row.
    fn activate(&mut self, list: ListId, index: usize) {
        // Une piste choisie dans la file garde la playlist ; un résultat de
        // recherche la remplace par les suggestions automatiques.
        if list == ListId::Suggestions && !self.queue.is_empty() {
            let id = &self.suggestions.items[index].id;
            if let Some(i) = self.queue.iter().position(|v| &v.id == id) {
                self.play_queue_index(i as isize);
                return;
            }
        }
        let video = match list {
            ListId::Results => self.results.items[index].clone(),
            ListId::Suggestions => self.suggestions.items[index].clone(),
        };
        self.play_video(video, false);
    }

    /// Every playback path goes through here and must say whether it keeps
    /// the playlist queue (`keep_queue`) or drops back to auto suggestions.
    fn play_video(&mut self, video: Video, keep_queue: bool) {
        // Results only hide on a pasted link/id (see `submit`) — picking a
        // track from the list itself must leave it exactly as it was.
        if !keep_queue {
            self.clear_queue();
        }
        self.play_seq += 1;
        self.stream_cache = Arc::default();
        self.source.clear();
        self.now_title = video.title.clone();
        self.now_sub = format!("{}  ·  ouverture du flux…", or_dash(&video.uploader));
        self.current = Some(video);
        // Resolving takes seconds: say so, rather than leaving a stale
        // "double-clic pour lire" up that reads as if the click was lost.
        self.set_status("ouverture du flux…");
        // Une piste enchaînée garde l'image si elle est déjà affichée.
        self.start_stream(0.0, self.deck_video || self.fullscreen);
        if !keep_queue {
            self.load_suggestions();
        }
    }

    /// Audio seul par défaut : la piste vidéo n'est décodée que si « v » l'a
    /// demandée, ce qui rouvre le flux à la position courante.
    fn start_stream(&mut self, start: f64, want_video: bool) {
        let Some(video) = self.current.clone() else { return };
        let cache = self.stream_cache.clone();
        let tokens = self.tokens.stream.clone();
        let token = tokens.fetch_add(1, Ordering::SeqCst) + 1;
        let (player, tx) = (self.player.clone(), self.tx.clone());
        let listed_duration = video.duration;
        exec::spawn("stream", move || {
            let provider_cache = cache.clone();
            // Called again by the player on every failed attempt, so each
            // retry gets a freshly signed URL rather than the dead one. A
            // clip asked for over an audio-only URL also forces a re-resolve.
            let provider: Provider = Arc::new(move |refresh| {
                let mut slot = provider_cache.lock().unwrap_or_else(|e| e.into_inner());
                let stale = match slot.as_ref() {
                    None => true,
                    Some((s, _)) => refresh || (want_video && !s.has_video),
                };
                if stale {
                    *slot = Some(sources::resolve(&video, want_video)?);
                }
                let (s, _) = slot.as_ref().expect("resolved above");
                Ok((s.url.clone(), s.headers.clone()))
            });
            let send = |result| {
                let _ = tx.send(Msg::Stream { token, result });
            };
            if let Err(e) = provider(false) {
                return send(Err(e));
            }
            if tokens.load(Ordering::SeqCst) != token {
                return; // superseded while yt-dlp was running
            }
            let (source, has_video, meta) = {
                let slot = cache.lock().unwrap_or_else(|e| e.into_inner());
                let (s, m) = slot.as_ref().expect("resolved above");
                (s.source.clone(), s.has_video, m.clone())
            };
            player.set_video(want_video && has_video);
            let duration = meta.duration.or(listed_duration).map(f64::from);
            if let Err(e) = player.play(provider, duration, start) {
                return send(Err(e));
            }
            send(Ok(StreamInfo { meta, source, has_video, want_video }));
        });
    }

    fn stream_started(&mut self, info: StreamInfo) {
        let Some(video) = self.current.as_mut() else { return };
        // A pasted link starts with a placeholder title; resolving fills it in.
        if let Some(title) = info.meta.title {
            video.title = title;
        }
        if video.uploader.is_empty()
            && let Some(uploader) = info.meta.uploader {
                video.uploader = uploader;
            }
        if info.meta.duration.is_some() {
            video.duration = info.meta.duration;
        }
        self.source = info.source;
        self.now_title = video.title.clone();
        self.now_sub = format!(
            "{}  ·  {}  ·  {}  ·  {}",
            or_dash(&video.uploader),
            video.duration_str(),
            if self.source.is_empty() { "audio seul" } else { &self.source },
            player::BACKEND
        );
        if info.want_video && !info.has_video {
            self.notify("Aucune piste vidéo sur ce flux — spectre affiché.", 5);
            self.no_picture();
        }
        self.set_status("lecture");
    }

    fn load_suggestions(&mut self) {
        let Some(id) = self.current.as_ref().map(|v| v.id.clone()) else { return };
        let (seq, tx) = (self.play_seq, self.tx.clone());
        exec::spawn("related", move || {
            let result = sources::related(&id);
            let _ = tx.send(Msg::Related { seq, result });
        });
    }

    /// A slow fetch for a track the user has since left can land after a
    /// faster, newer one: anything not for the track still playing is
    /// dropped.
    fn suggestions_done(&mut self, seq: u64, result: Result<Vec<Video>, String>) {
        if seq != self.play_seq || !self.queue.is_empty() {
            return;
        }
        match result {
            Err(_) => self.set_status("suggestions indisponibles"),
            Ok(videos) => {
                let empty = videos.is_empty();
                self.fill_suggestions(videos);
                if empty {
                    self.set_status("aucune suggestion pour cette piste");
                }
            }
        }
    }

    fn action_next_track(&mut self) {
        if !self.queue.is_empty() {
            self.play_queue_index(self.queue_index + 1);
            return;
        }
        match self.suggestions.items.first().cloned() {
            Some(video) => self.play_video(video, false),
            None => self.set_status("Aucune suggestion à enchaîner."),
        }
    }

    // ---------------------------------------------------------------- queue

    fn load_playlist(&mut self, playlist_id: String, start_id: Option<String>) {
        self.tokens.playlist += 1;
        let (token, tx) = (self.tokens.playlist, self.tx.clone());
        exec::spawn("playlist", move || {
            let result = sources::playlist(&playlist_id);
            let _ = tx.send(Msg::Playlist { token, start_id, result });
        });
    }

    fn install_queue(&mut self, videos: Vec<Video>, start: usize) {
        let n = videos.len();
        self.queue = videos.clone();
        self.fill_suggestions(videos);
        self.suggestions.title = format!("S U I T E   ·   playlist ({n})");
        self.set_status(&format!("playlist : {n} pistes"));
        self.play_queue_index(start as isize);
    }

    fn play_queue_index(&mut self, index: isize) {
        if index < 0 || index as usize >= self.queue.len() {
            self.set_status("fin de la playlist");
            return;
        }
        self.queue_index = index;
        let i = index as usize;
        if i < self.suggestions.items.len() {
            self.suggestions.select(i);
        }
        self.play_video(self.queue[i].clone(), true);
    }

    fn clear_queue(&mut self) {
        if self.thumbs_mode {
            self.thumbs_mode = false;
            self.apply_thumbs();
        }
        if self.queue.is_empty() {
            return;
        }
        self.queue.clear();
        self.queue_index = -1;
        self.suggestions.title = TITLE_AUTO.into();
    }

    // ----------------------------------------------------------- miniatures

    /// « t » : grille de miniatures de la playlist à la place de la liste
    /// texte. N'existe qu'en mode playlist — en mode auto il n'y a pas de
    /// "prochaines pistes" stables à précharger.
    fn action_toggle_thumbs(&mut self) {
        if self.queue.is_empty() {
            self.notify("Miniatures disponibles en mode playlist.", 3);
            return;
        }
        self.thumbs_mode = !self.thumbs_mode;
        self.apply_thumbs();
    }

    fn apply_thumbs(&mut self) {
        if self.thumbs_mode {
            self.focus = Focus::Grid;
            // Start near the upcoming track instead of the top of the list.
            self.grid.selected = self.queue_index.max(0) as usize;
            self.grid.follow = true;
        } else {
            if self.focus == Focus::Grid {
                self.focus = Focus::Suggestions;
            }
            self.grid.clear();
            self.thumbs_requested.clear();
            self.tokens.thumbs.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// After each draw: thumbnails for whatever is on screen and not decoded
    /// yet, one ffmpeg at a time — a thumbnail is a tiny JPEG so each is
    /// fast, and this avoids a burst of concurrent ffmpeg processes.
    pub fn after_draw(&mut self) {
        if !self.thumbs_mode || self.fullscreen {
            return;
        }
        let missing: Vec<String> =
            self.grid.missing(&self.queue).into_iter().map(|v| v.id.clone()).collect();
        // Still covered by the worker in flight: let it finish rather than
        // restarting it every time one of its frames lands.
        if missing.iter().all(|id| self.thumbs_requested.contains(id)) {
            return;
        }
        self.thumbs_requested = missing.clone();
        let tokens = self.tokens.thumbs.clone();
        let token = tokens.fetch_add(1, Ordering::SeqCst) + 1;
        let tx = self.tx.clone();
        exec::spawn("thumbs", move || {
            for id in missing {
                if tokens.load(Ordering::SeqCst) != token {
                    return;
                }
                if let Some(frame) = player::decode_thumbnail(&id, w::CELL_W, w::IMAGE_H * 2) {
                    let _ = tx.send(Msg::Thumb { id, frame });
                }
            }
        });
    }

    fn play_from_thumb(&mut self, index: usize) {
        if index < self.queue.len() {
            self.play_queue_index(index as isize);
        }
    }

    // ----------------------------------------------------------------- clip

    fn apply_deck(&mut self) {
        self.shown_frame = None;
    }

    /// Rouvre le flux courant avec sa piste vidéo si besoin. False = rien à
    /// afficher (aucune piste en cours).
    fn ensure_video_stream(&mut self) -> bool {
        if self.current.is_none() {
            self.notify("Aucune piste en cours.", 3);
            return false;
        }
        if !self.player.video() {
            self.set_status("ouverture de la vidéo…");
            self.start_stream(self.player.position(), true);
        }
        true
    }

    /// « v » : l'image prend la place du spectre dans la platine, et un
    /// second appui rend la platine à l'analyseur.
    fn action_toggle_clip(&mut self) {
        if self.deck_video {
            self.deck_video = false;
            self.apply_deck();
            return;
        }
        if !self.ensure_video_stream() {
            return;
        }
        self.deck_video = true;
        self.apply_deck();
    }

    /// « V » (maj + v), ou un clic sur l'image : plein écran.
    fn action_toggle_fullscreen(&mut self) {
        if self.fullscreen {
            self.leave_fullscreen();
            return;
        }
        if !self.ensure_video_stream() {
            return;
        }
        self.fullscreen = true;
        self.shown_frame = None;
        self.notify("Plein écran — « Échap » pour revenir", 3);
    }

    /// Flux sans piste vidéo : retour à l'analyseur, dans la platine comme
    /// en plein écran.
    fn no_picture(&mut self) {
        self.deck_video = false;
        self.apply_deck();
        self.leave_fullscreen();
    }

    fn leave_fullscreen(&mut self) {
        if !self.fullscreen {
            return;
        }
        self.fullscreen = false;
        self.notify("Retour à la platine", 2);
    }

    // ------------------------------------------------------------- actions

    fn run(&mut self, action: Action) {
        match action {
            Action::TogglePause => {
                if self.player.loaded() {
                    self.player.toggle_pause();
                }
            }
            Action::Next => self.action_next_track(),
            Action::SeekBack => self.seek(-10.0),
            Action::SeekFwd => self.seek(10.0),
            Action::VolUp => self.volume(0.1),
            Action::VolDown => self.volume(-0.1),
            Action::Clip => self.action_toggle_clip(),
            Action::Fullscreen => self.action_toggle_fullscreen(),
            Action::Thumbs => self.action_toggle_thumbs(),
            Action::Login => self.action_cycle_login(),
            // « Échap » only ever shrinks: it never opens the full screen.
            Action::ClipSmall => self.leave_fullscreen(),
            Action::FocusSearch => self.focus = Focus::Search,
            Action::Stop => {
                self.player.stop();
                self.now_title = NO_TRACK.into();
                self.now_sub = "prêt".into();
                self.set_status("arrêt");
            }
            Action::Quit => self.quit = true,
        }
    }

    fn seek(&mut self, by: f64) {
        if let Err(e) = self.player.seek(by) {
            self.notify_error(&e);
        }
    }

    fn volume(&mut self, by: f32) {
        if let Err(e) = self.player.set_volume(self.player.volume() + by) {
            self.notify_error(&e);
        }
        let pct = (self.player.volume() * 100.0).round();
        self.set_status(&format!("Volume {pct}%"));
    }

    /// « L » : cycle explicitement entre "pas connecté" et les navigateurs
    /// pris en charge par yt-dlp pour l'authentification par cookies (vidéos
    /// limitées par âge, réservées aux membres, etc.). Pas d'automatisme au
    /// démarrage — c'est une action délibérée, visible dans la barre du bas
    /// tant qu'elle est active.
    fn action_cycle_login(&mut self) {
        match sources::cycle_cookie_browser() {
            Some(b) => {
                self.set_status(&format!("connexion : {b}"));
                self.notify(&format!("Authentification via les cookies de {b}"), 3);
            }
            None => {
                self.set_status("déconnecté");
                self.notify("Authentification désactivée", 3);
            }
        }
        self.tick_mem();
    }

    // ------------------------------------------------------------- input

    fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.on_key(key),
            Event::Mouse(m) => self.on_mouse(m),
            Event::Paste(text) if self.focus == Focus::Search && !self.fullscreen => {
                self.input.insert(&text.replace(['\r', '\n'], " "));
                self.history.reset();
            }
            Event::Resize(..) => {}
            _ => self.dirty = false,
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char('c' | 'q')) {
            self.quit = true;
            return;
        }
        if !self.fullscreen {
            let consumed = match self.focus {
                Focus::Search => self.input_key(key),
                Focus::Results => self.list_key(ListId::Results, key),
                Focus::Suggestions => self.list_key(ListId::Suggestions, key),
                Focus::Grid => self.grid_key(key),
            };
            if consumed {
                return;
            }
            match key.code {
                KeyCode::Tab => return self.cycle_focus(1),
                KeyCode::BackTab => return self.cycle_focus(-1),
                _ => {}
            }
        }
        let action = match key.code {
            KeyCode::Char(' ') => Action::TogglePause,
            KeyCode::Char('n') => Action::Next,
            KeyCode::Left => Action::SeekBack,
            KeyCode::Right => Action::SeekFwd,
            KeyCode::Char('+' | '=') => Action::VolUp,
            KeyCode::Char('-') => Action::VolDown,
            KeyCode::Char('v') => Action::Clip,
            KeyCode::Char('V') => Action::Fullscreen,
            KeyCode::Char('t') => Action::Thumbs,
            KeyCode::Char('L') => Action::Login,
            KeyCode::Esc => Action::ClipSmall,
            KeyCode::Char('/') => Action::FocusSearch,
            KeyCode::Char('s') => Action::Stop,
            KeyCode::Char('q') => Action::Quit,
            _ => {
                self.dirty = false;
                return;
            }
        };
        self.run(action);
    }

    fn focus_chain(&self) -> Vec<Focus> {
        let mut chain = vec![Focus::Search];
        if self.results_visible {
            chain.push(Focus::Results);
        }
        chain.push(if self.thumbs_mode { Focus::Grid } else { Focus::Suggestions });
        chain
    }

    fn cycle_focus(&mut self, step: isize) {
        let chain = self.focus_chain();
        let at = chain.iter().position(|f| *f == self.focus).unwrap_or(0) as isize;
        self.focus = chain[(at + step).rem_euclid(chain.len() as isize) as usize];
    }

    fn input_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Enter => self.submit(),
            KeyCode::Up => {
                if let Some(text) = self.history.previous(&self.input.value()) {
                    self.input.set(&text);
                }
            }
            KeyCode::Down => {
                if let Some(text) = self.history.next() {
                    self.input.set(&text);
                }
            }
            KeyCode::Left if ctrl || alt => self.input.word_left(),
            KeyCode::Right if ctrl || alt => self.input.word_right(),
            KeyCode::Left => self.input.left(),
            KeyCode::Right => self.input.right(),
            KeyCode::Home => self.input.home(),
            KeyCode::End => self.input.end(),
            KeyCode::Backspace if ctrl || alt => self.input.delete_word_left(),
            KeyCode::Backspace => self.input.backspace(),
            KeyCode::Delete => self.input.delete(),
            KeyCode::Char('a') if ctrl => self.input.home(),
            KeyCode::Char('e') if ctrl => self.input.end(),
            KeyCode::Char('u') if ctrl => self.input.delete_to_start(),
            KeyCode::Char('k') if ctrl => self.input.delete_to_end(),
            KeyCode::Char('w') if ctrl => self.input.delete_word_left(),
            KeyCode::Char(c) if !ctrl && !alt => {
                self.input.insert(c.encode_utf8(&mut [0; 4]));
                self.history.reset();
            }
            _ => return false,
        }
        true
    }

    fn list(&mut self, id: ListId) -> &mut VList {
        match id {
            ListId::Results => &mut self.results,
            ListId::Suggestions => &mut self.suggestions,
        }
    }

    fn list_key(&mut self, id: ListId, key: KeyEvent) -> bool {
        let list = self.list(id);
        match key.code {
            KeyCode::Up => list.move_by(-1),
            KeyCode::Down => list.move_by(1),
            KeyCode::Home => list.select(0),
            KeyCode::End => list.select(usize::MAX),
            KeyCode::PageUp => list.page_by(-1),
            KeyCode::PageDown => list.page_by(1),
            KeyCode::Enter => {
                if let Some(i) = list.index.filter(|i| *i < list.items.len()) {
                    self.activate(id, i);
                }
            }
            _ => return false,
        }
        true
    }

    fn grid_key(&mut self, key: KeyEvent) -> bool {
        let n = self.queue.len();
        match key.code {
            KeyCode::Left => self.grid.move_sel('l', n),
            KeyCode::Right => self.grid.move_sel('r', n),
            KeyCode::Up => self.grid.move_sel('u', n),
            KeyCode::Down => self.grid.move_sel('d', n),
            KeyCode::Enter => self.play_from_thumb(self.grid.selected),
            KeyCode::Home => self.grid.scroll_home(),
            KeyCode::End => self.grid.scroll_end(n),
            KeyCode::PageUp => self.grid.scroll(-self.grid.page(), n),
            KeyCode::PageDown => self.grid.scroll(self.grid.page(), n),
            _ => return false,
        }
        true
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        let (x, y) = (m.column, m.row);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => self.click(x, y),
            MouseEventKind::ScrollUp => self.wheel(x, y, -3),
            MouseEventKind::ScrollDown => self.wheel(x, y, 3),
            MouseEventKind::Moved => {
                let hover = self.list_at(x, y);
                if hover == self.hover {
                    self.dirty = false;
                }
                self.hover = hover;
            }
            _ => self.dirty = false,
        }
    }

    fn list_at(&self, x: u16, y: u16) -> Option<(ListId, usize)> {
        if self.fullscreen {
            return None;
        }
        if self.results_visible
            && let Some(i) = self.results.hit(x, y) {
                return Some((ListId::Results, i));
            }
        if !self.thumbs_mode
            && let Some(i) = self.suggestions.hit(x, y) {
                return Some((ListId::Suggestions, i));
            }
        None
    }

    fn click(&mut self, x: u16, y: u16) {
        // A click on a toast dismisses it, as in Textual.
        if let Some(i) = self.toasts.iter().position(|t| contains(t.rect, x, y)) {
            self.toasts.remove(i);
            return;
        }
        if self.fullscreen {
            if contains(self.hits.full_clip, x, y) {
                self.action_toggle_fullscreen();
            }
            return;
        }
        if let Some(&(_, action)) = self.hits.footer.iter().find(|(r, _)| contains(*r, x, y)) {
            return self.run(action);
        }
        if contains(self.hits.search, x, y) {
            self.focus = Focus::Search;
            self.input.click(x);
            return;
        }
        if self.deck_video && contains(self.hits.deck_clip, x, y) {
            // Clicking the picture swaps between the deck and full screen.
            return self.action_toggle_fullscreen();
        }
        if self.thumbs_mode && contains(self.grid_area(), x, y) {
            self.focus = Focus::Grid;
            // A single click only selects — playback needs a double click,
            // the same threshold as the text list — so browsing the grid with
            // the mouse can't fire a track by accident.
            if let Some(i) = self.grid.hit(x, y, self.queue.len()) {
                self.grid.selected = i;
                let now = Instant::now();
                if matches!(self.last_grid_click, Some((j, at)) if j == i && now - at < DOUBLE_CLICK) {
                    self.last_grid_click = None;
                    self.play_from_thumb(i);
                } else {
                    self.last_grid_click = Some((i, now));
                }
            }
            return;
        }
        for id in [ListId::Results, ListId::Suggestions] {
            let shown = match id {
                ListId::Results => self.results_visible,
                ListId::Suggestions => !self.thumbs_mode,
            };
            let area = self.list(id).view;
            if !shown || !contains(grow(area), x, y) {
                continue;
            }
            self.focus = if id == ListId::Results { Focus::Results } else { Focus::Suggestions };
            let Some(i) = self.list(id).hit(x, y) else { return };
            // A click always highlights; only a double click starts playback.
            self.list(id).select(i);
            let now = Instant::now();
            if matches!(self.last_click, Some((l, j, at)) if l == id && j == i && now - at < DOUBLE_CLICK) {
                self.last_click = None;
                self.activate(id, i);
            } else {
                self.last_click = Some((id, i, now));
                self.set_status("double-clic pour lire");
            }
            return;
        }
        self.dirty = false;
    }

    fn wheel(&mut self, x: u16, y: u16, delta: isize) {
        if self.fullscreen {
            return;
        }
        if self.thumbs_mode && contains(self.grid_area(), x, y) {
            self.grid.scroll(delta, self.queue.len());
            return;
        }
        if self.results_visible && contains(grow(self.results.view), x, y) {
            self.results.scroll(delta);
        } else if !self.thumbs_mode && contains(grow(self.suggestions.view), x, y) {
            self.suggestions.scroll(delta);
        } else {
            self.dirty = false;
        }
    }

    fn grid_area(&self) -> Rect {
        self.hits.suggestions
    }

    // ---------------------------------------------------------------- misc

    fn set_status(&mut self, message: &str) {
        self.status = message.to_uppercase();
        self.dirty = true;
    }

    /// One theme: info, warning and error toasts all wear the same ember
    /// frame, only their timeout differs.
    fn notify(&mut self, text: &str, secs: u64) {
        self.toasts.push(Toast {
            text: text.to_string(),
            until: Instant::now() + Duration::from_secs(secs),
            rect: Rect::default(),
        });
        self.dirty = true;
    }

    fn notify_error(&mut self, e: &str) {
        let text = if e.is_empty() { "erreur" } else { e };
        self.notify(text, 6);
    }
}

fn or_dash(s: &str) -> &str {
    if s.is_empty() { "—" } else { s }
}

fn contains(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.right() && y >= r.y && y < r.bottom()
}

/// A list's inner view grown back to its frame, for focus clicks.
fn grow(r: Rect) -> Rect {
    if r.width == 0 {
        return r;
    }
    Rect::new(r.x.saturating_sub(1), r.y.saturating_sub(1), r.width + 2, r.height + 2)
}

// ------------------------------------------------------------------ drawing

impl App {
    pub fn draw(&mut self, buf: &mut Buffer) {
        self.dirty = false;
        let area = buf.area;
        self.hits.footer.clear();
        if self.fullscreen {
            self.draw_fullscreen(buf, area);
            self.draw_toasts(buf, area, area.height, t::CLIP_BG);
            return;
        }
        w::fill(buf, area, t::SCREEN_BG);
        let (width, height) = (area.width, area.height);
        if width < 8 || height < 4 {
            return;
        }

        // top bar
        w::fill(buf, Rect::new(0, 0, width, 1), t::BAR_BG);
        let brand = "y t u i";
        spans(buf, 2, 0, width.saturating_sub(2), &[(brand, bold(t::BRAND))]);
        let after_brand = 2 + brand.len() as u16 + 2;
        let status_w = (self.status.width() as u16).min(width.saturating_sub(after_brand + 2));
        spans(buf, width - 2 - status_w, 0, status_w, &[(&self.status, fg(t::STATUS))]);

        // search
        let search = Rect::new(2, 2, width - 4, 3.min(height.saturating_sub(3)));
        self.hits.search = search;
        self.input.render(buf, search, self.focus == Focus::Search, PLACEHOLDER);

        self.draw_bottom(buf, Rect::new(0, height - 1, width, 1));

        // body: `padding: 1 2 0 2` under the search box
        let body_y = search.bottom() + 1;
        let body = Rect::new(2, body_y, width - 4, (height - 1).saturating_sub(body_y));
        if body.height == 0 {
            self.draw_toasts(buf, area, height - 1, t::SCREEN_BG);
            return;
        }
        let right = if self.results_visible {
            let left_w = body.width * 42 / 100;
            let left = Rect::new(body.x, body.y, left_w, body.height);
            let hover = self.hover.filter(|(l, _)| *l == ListId::Results).map(|(_, i)| i);
            let focused = self.focus == Focus::Results;
            self.results.render(buf, left, focused, None, hover);
            Rect::new(body.x + left_w + 2, body.y, body.width.saturating_sub(left_w + 2), body.height)
        } else {
            body
        };

        let deck = Rect::new(right.x, right.y, right.width, DECK_H.min(right.height));
        self.draw_deck(buf, deck);

        let rest_y = (deck.bottom() + 1).min(right.bottom());
        let rest = Rect::new(right.x, rest_y, right.width, right.bottom() - rest_y);
        self.hits.suggestions = rest;
        let playing = self.current.as_ref().map(|v| v.id.as_str());
        if self.thumbs_mode {
            self.grid.render(buf, rest, self.focus == Focus::Grid, &self.queue, playing);
        } else {
            let hover = self.hover.filter(|(l, _)| *l == ListId::Suggestions).map(|(_, i)| i);
            let focused = self.focus == Focus::Suggestions;
            self.suggestions.render(buf, rest, focused, playing, hover);
        }
        self.draw_toasts(buf, area, height - 1, t::SCREEN_BG);
    }

    fn draw_bottom(&mut self, buf: &mut Buffer, bar: Rect) {
        w::fill(buf, bar, t::BAR_BG);
        let mem_w = (self.mem.width() as u16 + 4).min(bar.width);
        let mem_x = bar.right() - mem_w;
        spans(buf, mem_x + 2, bar.y, mem_w.saturating_sub(4), &[(&self.mem, fg(t::MEM))]);
        let mut x = bar.x;
        for (key, desc, action) in BINDINGS {
            let key = format!(" {key} ");
            let desc = format!("{desc} ");
            let span_w = (key.width() + desc.width()) as u16;
            if x + span_w > mem_x {
                break;
            }
            spans(buf, x, bar.y, span_w, &[(&key, bold(t::FOOTER_KEY)), (&desc, fg(t::FOOTER_DESC))]);
            self.hits.footer.push((Rect::new(x, bar.y, span_w, 1), action));
            x += span_w + 1;
        }
    }

    fn draw_deck(&mut self, buf: &mut Buffer, deck: Rect) {
        w::tall_box(buf, deck, t::BORDER, t::PANEL_BG, t::SCREEN_BG, Some(("P L A T I N E", t::BORDER_TITLE)));
        let inner = w::inner(deck);
        // `padding: 1 2`
        let c = Rect::new(inner.x + 2, inner.y + 1, inner.width.saturating_sub(4), inner.height.saturating_sub(2));
        if c.width == 0 || c.height == 0 {
            return;
        }
        spans(buf, c.x, c.y, c.width, &[(&self.now_title, bold(t::NOW_TITLE))]);
        if c.height > 1 {
            spans(buf, c.x, c.y + 1, c.width, &[(&self.now_sub, fg(t::NOW_SUB))]);
        }
        // Analyseur par défaut ; « v » met l'image à sa place, « V »
        // l'envoie en plein écran.
        let meter = Rect::new(c.x, c.y + 3, c.width, METER_H.min(c.height.saturating_sub(3)));
        if self.deck_video {
            self.hits.deck_clip = meter;
            let frame = self.player.frame();
            self.clip_small.render(buf, meter, &frame, player::VID_W, player::VID_H);
        } else {
            self.hits.deck_clip = Rect::default();
            w::spectrum(buf, meter, &self.analyser);
        }
        let seek_y = c.y + 3 + METER_H + 1;
        if seek_y < c.bottom() {
            self.draw_seek(buf, Rect::new(c.x, seek_y, c.width, 1));
        }
    }

    fn draw_seek(&self, buf: &mut Buffer, area: Rect) {
        let dur = self.player.duration().unwrap_or(0.0);
        w::seek_bar(buf, area, self.player.position(), dur, self.player.paused());
    }

    /// Full-screen clip: the picture over the whole terminal, caption and
    /// transport under it on the same black ground; « Échap » comes back.
    fn draw_fullscreen(&mut self, buf: &mut Buffer, area: Rect) {
        w::fill(buf, area, t::CLIP_BG);
        let clip = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(2));
        self.hits.full_clip = clip;
        {
            let frame = self.player.frame();
            self.clip_full.render(buf, clip, &frame, player::VID_W, player::VID_H);
        }
        if area.height < 2 || area.width < 5 {
            return;
        }
        let caption = match &self.current {
            None => NO_TRACK.to_string(),
            Some(v) => format!("{}   ·   {}   ·   {}", v.title, or_dash(&v.uploader), self.source),
        };
        let inner_w = area.width - 4;
        spans(buf, 2, area.bottom() - 2, inner_w, &[(&caption, bold(t::NOW_TITLE))]);
        self.draw_seek(buf, Rect::new(2, area.bottom() - 1, inner_w, 1));
    }

    /// Bottom-right stack, newest lowest, like Textual's toast rack.
    fn draw_toasts(&mut self, buf: &mut Buffer, area: Rect, bottom: u16, behind: ratatui_core::style::Color) {
        let width = 60.min(area.width / 2).max(area.width.min(24));
        let text_w = width.saturating_sub(4) as usize;
        let mut bottom = bottom;
        for toast in self.toasts.iter_mut().rev() {
            toast.rect = Rect::default();
            if text_w == 0 {
                continue;
            }
            let lines = wrap(&toast.text, text_w);
            let h = lines.len() as u16 + 4; // tall border + `padding: 1 1`
            if bottom < h + area.y {
                continue;
            }
            let rect = Rect::new(area.right() - width - 1, bottom - h, width, h);
            w::tall_box(buf, rect, t::TOAST_BORDER, t::TOAST_BG, behind, None);
            for (i, line) in lines.iter().enumerate() {
                spans(buf, rect.x + 2, rect.y + 2 + i as u16, text_w as u16, &[(line, fg(t::TOAST_FG))]);
            }
            toast.rect = rect;
            bottom = rect.y.saturating_sub(1); // `margin-top: 1`
        }
    }
}

/// Greedy word wrap on display width; over-long words are hard-split.
fn wrap(text: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_w = 0;
    for word in text.split_whitespace() {
        let word_w = word.width();
        if line_w > 0 && line_w + 1 + word_w > width {
            lines.push(std::mem::take(&mut line));
            line_w = 0;
        }
        if line_w > 0 {
            line.push(' ');
            line_w += 1;
        }
        for ch in word.chars() {
            let cw = ch.width().unwrap_or(0);
            if line_w + cw > width {
                lines.push(std::mem::take(&mut line));
                line_w = 0;
            }
            line.push(ch);
            line_w += cw;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(n: usize) -> Vec<Video> {
        (0..n)
            .map(|i| Video {
                id: format!("vid{i:08}"),
                title: format!("Morceau numéro {i} — un titre assez long pour être coupé"),
                uploader: "Chaîne".into(),
                duration: Some(60 + i as u32 * 7),
            })
            .collect()
    }

    fn dump(app: &mut App, w: u16, h: u16) -> String {
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        app.draw(&mut buf);
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Offline layout check: no network, no ffmpeg, nothing spawned.
    #[test]
    fn layout() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx);
        app.results.set_items(fake(30));
        app.results.select(2);
        app.suggestions.set_items(fake(12));
        app.current = Some(app.suggestions.items[1].clone());
        app.hover = Some((ListId::Suggestions, 3));
        app.notify("Plein écran — « Échap » pour revenir", 3);
        println!("{}", dump(&mut app, 140, 42));

        app.results_visible = false;
        app.queue = fake(12);
        app.queue_index = 7;
        app.thumbs_mode = true;
        app.apply_thumbs();
        app.grid.frames.insert("vid00000007".into(), vec![200; w::CELL_W * w::IMAGE_H * 2 * 3]);
        let out = dump(&mut app, 100, 42);
        println!("{out}");
        assert!(out.contains("miniatures"));
        assert!(app.grid.scroll_y > 0, "grid follows the playing track");
    }
}
