//! Every colour of the app lives in this file; widgets never spell out an
//! RGB value of their own. Themes are plain data — one `Theme` per look —
//! and the active one is read through `get()` at draw time, so switching is
//! a single atomic store and the next frame repaints.
//!
//! - `default`: a late-70s hi-fi separate. Warm near-black, brushed-metal
//!   hairlines, amber legends, ember accents.
//! - `dark`: cool graphite, white text, cyan legends, a teal → violet → pink
//!   meter.
//! - `white`: paper and ink. Off-white chassis, white panels, black text,
//!   ink-blue legends, burnt-orange accents, a meter that stays legible on a
//!   light ground.

use std::sync::atomic::{AtomicUsize, Ordering};

use ratatui_core::style::Color;

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub struct Theme {
    pub name: &'static str,

    // screen
    pub screen_bg: Color,
    pub screen_fg: Color,

    // top / bottom bars
    pub bar_bg: Color,
    pub brand: Color,
    pub status: Color,
    pub mem: Color,
    pub footer_key: Color,
    pub footer_desc: Color,

    // panels
    pub border: Color,
    pub border_focus: Color,
    pub border_title: Color,
    pub border_title_focus: Color,
    pub panel_bg: Color,
    pub scrollbar: Color,

    // search
    pub search_bg: Color,
    pub search_bg_focus: Color,
    pub search_fg: Color,
    pub placeholder: Color,
    pub cursor_bg: Color,
    pub cursor_fg: Color,

    // list rows
    pub row_highlight_bg: Color,
    pub row_hover_bg: Color,
    pub row_playing_bg: Color,
    pub row_marker: Color,
    pub row_dur: Color,
    pub row_dur_hl: Color,
    pub row_dur_playing: Color,
    pub row_title: Color,
    pub row_title_hl: Color,
    pub row_uploader: Color,
    pub row_uploader_hl: Color,

    // waiting for yt-dlp: the spinner glyph is lit, its label stays dim
    pub spinner: Color,
    pub spinner_label: Color,

    // deck
    pub now_title: Color,
    pub now_sub: Color,
    /// Bottom -> top: the meter's ramp. Its last steps are the "clipping"
    /// zone only the loudest transients reach.
    pub ramp: [(u8, u8, u8); 9],
    pub chassis: Color, // unlit segment
    pub ember: Color,   // peak hold

    // seek bar
    pub seek_play: Color,
    pub seek_paused: Color,
    pub seek_pos: Color,
    pub seek_done: Color,
    pub seek_head: Color,
    pub seek_todo: Color,
    pub seek_dur: Color,
    pub seek_live: Color,

    // thumb grid
    pub thumb_playing_bar: Color,
    pub thumb_selected_bar: Color,
    pub thumb_selected_bg: Color,
    pub thumb_selected_marker: Color,
    pub thumb_empty: Color,

    // clip: full screen stays black in every theme — it is a video
    pub clip_bg: Color,

    // toasts
    pub toast_bg: Color,
    pub toast_border: Color,
    pub toast_fg: Color,
}

pub const DEFAULT: Theme = Theme {
    name: "default",
    screen_bg: rgb(0x151312),
    screen_fg: rgb(0xcbc0b2),
    bar_bg: rgb(0x1e1a17),
    brand: rgb(0xff6b35),
    status: rgb(0x6b6055),
    mem: rgb(0x4f473e),
    footer_key: rgb(0xe09b2a),
    footer_desc: rgb(0x6b6055),
    border: rgb(0x6b4a1e),
    border_focus: rgb(0xe09b2a),
    border_title: rgb(0x6b6055),
    border_title_focus: rgb(0xf2b23c),
    panel_bg: rgb(0x191615),
    scrollbar: rgb(0x443b32),
    search_bg: rgb(0x1a1716),
    search_bg_focus: rgb(0x211b15),
    search_fg: rgb(0xf0e6d8),
    placeholder: rgb(0x554c43),
    cursor_bg: rgb(0xff6b35),
    cursor_fg: rgb(0x151312),
    row_highlight_bg: rgb(0x3b2b14),
    row_hover_bg: rgb(0x221c18),
    row_playing_bg: rgb(0x3d2015),
    row_marker: rgb(0xff6b35),
    row_dur: rgb(0x7d7266),
    row_dur_hl: rgb(0xf2b23c),
    row_dur_playing: rgb(0xff8a3d),
    row_title: rgb(0xcbc0b2),
    row_title_hl: rgb(0xfff6e6),
    row_uploader: rgb(0x6b6055),
    row_uploader_hl: rgb(0xe0b98a),
    spinner: rgb(0xf2b23c),
    spinner_label: rgb(0x6b6055),
    now_title: rgb(0xfff6e6),
    now_sub: rgb(0x6b6055),
    // Cold amber, then ember, then a red "clipping" zone.
    ramp: [
        (0x6B, 0x45, 0x14),
        (0x9A, 0x62, 0x1B),
        (0xC4, 0x7F, 0x1E),
        (0xE0, 0x9B, 0x2A),
        (0xF2, 0xB2, 0x3C),
        (0xFF, 0xC4, 0x59),
        (0xFF, 0x8A, 0x3D),
        (0xFF, 0x5C, 0x2E),
        (0xE0, 0x32, 0x1F),
    ],
    chassis: rgb(0x332c24),
    ember: rgb(0xff6b35),
    seek_play: rgb(0xff6b35),
    seek_paused: rgb(0x8a7f70),
    seek_pos: rgb(0xf2b23c),
    seek_done: rgb(0xe09b2a),
    seek_head: rgb(0xff6b35),
    seek_todo: rgb(0x5f564c),
    seek_dur: rgb(0x8a7f70),
    seek_live: rgb(0xff5c2e),
    thumb_playing_bar: rgb(0xff6b35),
    thumb_selected_bar: rgb(0xe09b2a),
    thumb_selected_bg: rgb(0x5a3a12),
    thumb_selected_marker: rgb(0xffb454),
    thumb_empty: rgb(0x6b6055),
    clip_bg: rgb(0x000000),
    toast_bg: rgb(0x221c18),
    toast_border: rgb(0xff5c2e),
    toast_fg: rgb(0xf0e6d8),
};

pub const DARK: Theme = Theme {
    name: "dark",
    screen_bg: rgb(0x0e1014),
    screen_fg: rgb(0xe8eaee),
    bar_bg: rgb(0x15181e),
    brand: rgb(0x7dd3fc),
    status: rgb(0x8a93a3),
    mem: rgb(0x5b6370),
    footer_key: rgb(0x7dd3fc),
    footer_desc: rgb(0x8a93a3),
    border: rgb(0x2a303b),
    border_focus: rgb(0x7dd3fc),
    border_title: rgb(0x8a93a3),
    border_title_focus: rgb(0xe0f2fe),
    panel_bg: rgb(0x13161b),
    scrollbar: rgb(0x2f3642),
    search_bg: rgb(0x13161b),
    search_bg_focus: rgb(0x171b22),
    search_fg: rgb(0xffffff),
    placeholder: rgb(0x5b6370),
    cursor_bg: rgb(0x7dd3fc),
    cursor_fg: rgb(0x0e1014),
    row_highlight_bg: rgb(0x1e2633),
    row_hover_bg: rgb(0x181c23),
    row_playing_bg: rgb(0x2a1f33),
    row_marker: rgb(0xf472b6),
    row_dur: rgb(0x6b7383),
    row_dur_hl: rgb(0x7dd3fc),
    row_dur_playing: rgb(0xf472b6),
    row_title: rgb(0xd6dae1),
    row_title_hl: rgb(0xffffff),
    row_uploader: rgb(0x6b7383),
    row_uploader_hl: rgb(0xa5b4c8),
    spinner: rgb(0x7dd3fc),
    spinner_label: rgb(0x8a93a3),
    now_title: rgb(0xffffff),
    now_sub: rgb(0x8a93a3),
    // Deep teal, cyan, then violet and a pink "clipping" zone.
    ramp: [
        (0x0E, 0x4D, 0x64),
        (0x0F, 0x76, 0x8E),
        (0x14, 0xA3, 0xB8),
        (0x38, 0xC6, 0xD9),
        (0x7D, 0xD3, 0xFC),
        (0xA5, 0xB4, 0xFC),
        (0xC0, 0x84, 0xFC),
        (0xE8, 0x79, 0xF9),
        (0xF4, 0x72, 0xB6),
    ],
    chassis: rgb(0x232831),
    ember: rgb(0xf472b6),
    seek_play: rgb(0x7dd3fc),
    seek_paused: rgb(0x6b7383),
    seek_pos: rgb(0xe0f2fe),
    seek_done: rgb(0x38bdf8),
    seek_head: rgb(0xf472b6),
    seek_todo: rgb(0x2f3642),
    seek_dur: rgb(0x8a93a3),
    seek_live: rgb(0xf472b6),
    thumb_playing_bar: rgb(0xf472b6),
    thumb_selected_bar: rgb(0x7dd3fc),
    thumb_selected_bg: rgb(0x1e2633),
    thumb_selected_marker: rgb(0x7dd3fc),
    thumb_empty: rgb(0x5b6370),
    clip_bg: rgb(0x000000),
    toast_bg: rgb(0x171b22),
    toast_border: rgb(0x7dd3fc),
    toast_fg: rgb(0xffffff),
};

pub const WHITE: Theme = Theme {
    name: "white",
    screen_bg: rgb(0xf4f2ee),
    screen_fg: rgb(0x1a1a1a),
    bar_bg: rgb(0xe8e5df),
    brand: rgb(0xd9480f),
    status: rgb(0x6b665e),
    mem: rgb(0x8c867c),
    footer_key: rgb(0x1f5fbf),
    footer_desc: rgb(0x5f5a52),
    border: rgb(0xcfc9bf),
    border_focus: rgb(0x1f5fbf),
    border_title: rgb(0x7a746a),
    border_title_focus: rgb(0x1f5fbf),
    panel_bg: rgb(0xffffff),
    scrollbar: rgb(0xd6d1c8),
    search_bg: rgb(0xffffff),
    search_bg_focus: rgb(0xffffff),
    search_fg: rgb(0x111111),
    placeholder: rgb(0xa39d92),
    cursor_bg: rgb(0x1f5fbf),
    cursor_fg: rgb(0xffffff),
    row_highlight_bg: rgb(0xe4ecf9),
    row_hover_bg: rgb(0xf3f1ec),
    row_playing_bg: rgb(0xfde8dd),
    row_marker: rgb(0xd9480f),
    row_dur: rgb(0x8c867c),
    row_dur_hl: rgb(0x1f5fbf),
    row_dur_playing: rgb(0xd9480f),
    row_title: rgb(0x2a2a2a),
    row_title_hl: rgb(0x000000),
    row_uploader: rgb(0x8c867c),
    row_uploader_hl: rgb(0x5f5a52),
    spinner: rgb(0x1f5fbf),
    spinner_label: rgb(0x6b665e),
    now_title: rgb(0x000000),
    now_sub: rgb(0x6b665e),
    // Saturated end to end so even the lowest step reads on white: navy,
    // blue, teal, green, then amber to a red "clipping" zone.
    ramp: [
        (0x1E, 0x3A, 0x8A),
        (0x1F, 0x5F, 0xBF),
        (0x0E, 0x7C, 0xA8),
        (0x0F, 0x95, 0x8F),
        (0x2F, 0x9E, 0x44),
        (0xCA, 0x8A, 0x04),
        (0xEA, 0x58, 0x0C),
        (0xDC, 0x26, 0x26),
        (0xB9, 0x1C, 0x1C),
    ],
    chassis: rgb(0xe3dfd8),
    ember: rgb(0xd9480f),
    seek_play: rgb(0xd9480f),
    seek_paused: rgb(0x8c867c),
    seek_pos: rgb(0x1a1a1a),
    seek_done: rgb(0x1f5fbf),
    seek_head: rgb(0xd9480f),
    seek_todo: rgb(0xd6d1c8),
    seek_dur: rgb(0x6b665e),
    seek_live: rgb(0xdc2626),
    thumb_playing_bar: rgb(0xd9480f),
    thumb_selected_bar: rgb(0x1f5fbf),
    thumb_selected_bg: rgb(0xe4ecf9),
    thumb_selected_marker: rgb(0x1f5fbf),
    thumb_empty: rgb(0x8c867c),
    clip_bg: rgb(0x000000),
    toast_bg: rgb(0xffffff),
    toast_border: rgb(0xd9480f),
    toast_fg: rgb(0x1a1a1a),
};

pub const ALL: [&Theme; 3] = [&DEFAULT, &DARK, &WHITE];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// The active theme.
pub fn get() -> &'static Theme {
    ALL[CURRENT.load(Ordering::Relaxed) % ALL.len()]
}

pub fn index() -> usize {
    CURRENT.load(Ordering::Relaxed) % ALL.len()
}

pub fn set(index: usize) {
    CURRENT.store(index % ALL.len(), Ordering::Relaxed);
}

/// ~/.local/share/ytui/theme: the name of the theme picked last.
fn saved_path() -> Option<std::path::PathBuf> {
    Some(crate::tools::data_dir()?.join("theme"))
}

pub fn load_saved() {
    let Some(name) = saved_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return };
    if let Some(i) = ALL.iter().position(|t| t.name == name.trim()) {
        set(i);
    }
}

pub fn save() {
    let Some(path) = saved_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, get().name); // a read-only home must not take the app down
}
