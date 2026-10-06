//! ytui — chassis of a late-70s hi-fi separate.
//! Warm near-black, brushed-metal hairlines, amber legends, ember accents.
//!
//! **The only theme.** Every colour of the app lives in this file; widgets
//! never spell out an RGB value of their own.

use ratatui_core::style::Color;

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

// ------------------------------------------------------------------- screen
pub const SCREEN_BG: Color = rgb(0x151312);
pub const SCREEN_FG: Color = rgb(0xcbc0b2);

// ------------------------------------------------------------ top / bottom bars
pub const BAR_BG: Color = rgb(0x1e1a17);
pub const BRAND: Color = rgb(0xff6b35);
pub const STATUS: Color = rgb(0x6b6055);
pub const MEM: Color = rgb(0x4f473e);
pub const FOOTER_KEY: Color = rgb(0xe09b2a);
pub const FOOTER_DESC: Color = rgb(0x6b6055);

// ------------------------------------------------------------------- panels
pub const BORDER: Color = rgb(0x6b4a1e);
pub const BORDER_FOCUS: Color = rgb(0xe09b2a);
pub const BORDER_TITLE: Color = rgb(0x6b6055);
pub const BORDER_TITLE_FOCUS: Color = rgb(0xf2b23c);
pub const PANEL_BG: Color = rgb(0x191615);
pub const SCROLLBAR: Color = rgb(0x443b32);

// ------------------------------------------------------------------- search
pub const SEARCH_BG: Color = rgb(0x1a1716);
pub const SEARCH_BG_FOCUS: Color = rgb(0x211b15);
pub const SEARCH_FG: Color = rgb(0xf0e6d8);
pub const PLACEHOLDER: Color = rgb(0x554c43);
pub const CURSOR_BG: Color = rgb(0xff6b35);
pub const CURSOR_FG: Color = rgb(0x151312);

// ---------------------------------------------------------------- list rows
pub const ROW_HIGHLIGHT_BG: Color = rgb(0x3b2b14);
pub const ROW_HOVER_BG: Color = rgb(0x221c18);
pub const ROW_PLAYING_BG: Color = rgb(0x3d2015);
pub const ROW_MARKER: Color = rgb(0xff6b35);
pub const ROW_DUR: Color = rgb(0x7d7266);
pub const ROW_DUR_HL: Color = rgb(0xf2b23c);
pub const ROW_DUR_PLAYING: Color = rgb(0xff8a3d);
pub const ROW_TITLE: Color = rgb(0xcbc0b2);
pub const ROW_TITLE_HL: Color = rgb(0xfff6e6);
pub const ROW_UPLOADER: Color = rgb(0x6b6055);
pub const ROW_UPLOADER_HL: Color = rgb(0xe0b98a);

// --------------------------------------------------------------------- deck
pub const NOW_TITLE: Color = rgb(0xfff6e6);
pub const NOW_SUB: Color = rgb(0x6b6055);

// Bottom -> top: the ramp of a VU meter. Cold amber, then ember, then a red
// "clipping" zone that only the loudest transients ever reach.
pub const RAMP: [(u8, u8, u8); 9] = [
    (0x6B, 0x45, 0x14),
    (0x9A, 0x62, 0x1B),
    (0xC4, 0x7F, 0x1E),
    (0xE0, 0x9B, 0x2A),
    (0xF2, 0xB2, 0x3C),
    (0xFF, 0xC4, 0x59),
    (0xFF, 0x8A, 0x3D),
    (0xFF, 0x5C, 0x2E),
    (0xE0, 0x32, 0x1F),
];
pub const CHASSIS: Color = rgb(0x332c24); // unlit segment
pub const EMBER: Color = rgb(0xff6b35);

// ----------------------------------------------------------------- seek bar
pub const SEEK_PLAY: Color = rgb(0xff6b35);
pub const SEEK_PAUSED: Color = rgb(0x8a7f70);
pub const SEEK_POS: Color = rgb(0xf2b23c);
pub const SEEK_DONE: Color = rgb(0xe09b2a);
pub const SEEK_HEAD: Color = rgb(0xff6b35);
pub const SEEK_TODO: Color = rgb(0x5f564c);
pub const SEEK_DUR: Color = rgb(0x8a7f70);
pub const SEEK_LIVE: Color = rgb(0xff5c2e);

// --------------------------------------------------------------- thumb grid
pub const THUMB_PLAYING_BAR: Color = rgb(0xff6b35);
pub const THUMB_SELECTED_BAR: Color = rgb(0xe09b2a);
pub const THUMB_SELECTED_BG: Color = rgb(0x5a3a12);
pub const THUMB_SELECTED_MARKER: Color = rgb(0xffb454);
pub const THUMB_EMPTY: Color = rgb(0x6b6055);

// --------------------------------------------------------------------- clip
pub const CLIP_BG: Color = rgb(0x000000);

// ------------------------------------------------------------------- toasts
pub const TOAST_BG: Color = rgb(0x221c18);
pub const TOAST_BORDER: Color = rgb(0xff5c2e);
pub const TOAST_FG: Color = rgb(0xf0e6d8);
