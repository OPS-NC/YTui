//! Rendering-only widgets, painted straight into the cell buffer.
//!
//! Visual language: a late-70s hi-fi separate. Warm near-black chassis, amber
//! legends, a peak-hold VU meter that runs cold amber at the bottom and burns
//! ember at the top. Everything is drawn with box-drawing glyphs — no images,
//! no fonts to load, no allocation per frame beyond the cell buffer itself.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Color, Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::player::{Analyser, Frame, NBANDS};
use crate::sources::{Video, fmt_time};
use crate::theme as t;

// ------------------------------------------------------------------ helpers

pub fn fg(c: Color) -> Style {
    Style::new().fg(c)
}

pub fn bold(c: Color) -> Style {
    Style::new().fg(c).add_modifier(Modifier::BOLD)
}

pub fn fill(buf: &mut Buffer, area: Rect, bg: Color) {
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            cell.reset();
            cell.set_bg(bg).set_fg(t::SCREEN_FG);
        }
    }
}

/// Writes `spans` from (x, y) within `width` cells; a cut line ends in `…`,
/// the way a Rich `Text(no_wrap=True, overflow="ellipsis")` would. Returns
/// the number of columns used.
pub fn spans(buf: &mut Buffer, x: u16, y: u16, width: u16, parts: &[(&str, Style)]) -> u16 {
    if width == 0 || y >= buf.area.bottom() {
        return 0;
    }
    let total: usize = parts.iter().map(|(s, _)| s.width()).sum();
    let cut = total > width as usize;
    let limit = if cut { width as usize - 1 } else { width as usize };
    let mut col = 0usize;
    let mut last = Style::new();
    for (text, style) in parts {
        if col >= limit {
            break;
        }
        last = *style;
        let mut end = 0;
        let mut w = 0;
        for (i, ch) in text.char_indices() {
            let cw = ch.width().unwrap_or(0);
            if col + w + cw > limit {
                break;
            }
            w += cw;
            end = i + ch.len_utf8();
        }
        buf.set_stringn(x + col as u16, y, &text[..end], w, *style);
        col += w;
        if end < text.len() {
            break;
        }
    }
    if cut {
        buf.set_stringn(x + col as u16, y, "…", 1, last);
        col += 1;
    }
    col as u16
}

/// Textual's `tall` border: thin inset lines left and right over the outer
/// background, hairlines top and bottom over the panel's own, so the frame
/// reads taller than it is wide. Paints the panel background too.
pub fn tall_box(
    buf: &mut Buffer,
    area: Rect,
    border: Color,
    inner_bg: Color,
    outer_bg: Color,
    title: Option<(&str, Color)>,
) {
    let area = area.intersection(buf.area);
    if area.width < 2 || area.height < 2 {
        return;
    }
    fill(buf, area, inner_bg);
    let (l, r, top, bot) = (area.left(), area.right() - 1, area.top(), area.bottom() - 1);
    // Left edge is reversed: the glyph's ink is the outer background and the
    // border colour shows through as the cell background.
    let left = Style::new().fg(outer_bg).bg(border);
    let right = Style::new().fg(border).bg(outer_bg);
    let line = Style::new().fg(border).bg(inner_bg);
    for y in top..=bot {
        buf[(l, y)].set_symbol("▊").set_style(left);
        buf[(r, y)].set_symbol("▎").set_style(right);
    }
    for x in l + 1..r {
        buf[(x, top)].set_symbol("▔").set_style(line);
        buf[(x, bot)].set_symbol("▁").set_style(line);
    }
    if let Some((title, color)) = title {
        let room = area.width.saturating_sub(4);
        let label = format!(" {title} ");
        spans(buf, l + 1, top, room.min(label.width() as u16), &[(&label, fg(color).bg(inner_bg))]);
    }
}

pub fn inner(area: Rect) -> Rect {
    Rect::new(area.x + 1, area.y + 1, area.width.saturating_sub(2), area.height.saturating_sub(2))
}

/// One-column scrollbar; returns the content width left of it.
fn scrollbar(buf: &mut Buffer, area: Rect, total: usize, offset: usize) -> u16 {
    let h = area.height as usize;
    if total <= h || area.width < 2 {
        return area.width;
    }
    let x = area.right() - 1;
    let thumb = (h * h / total).max(1);
    let start = (offset * h / total).min(h - thumb);
    for row in 0..h {
        let cell = &mut buf[(x, area.y + row as u16)];
        cell.set_symbol(" ");
        cell.set_bg(if (start..start + thumb).contains(&row) { t::SCROLLBAR } else { t::PANEL_BG });
    }
    area.width - 1
}

// -------------------------------------------------------------- text input

/// The search field: a one-line editor with horizontal scrolling. The
/// cursor does not blink — a blinking cursor is a timer firing forever for
/// nothing.
#[derive(Default)]
pub struct TextInput {
    pub chars: Vec<char>,
    pub cursor: usize,
    scroll: usize,
    view: Rect,
}

impl TextInput {
    pub fn value(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn set(&mut self, value: &str) {
        self.chars = value.chars().collect();
        self.cursor = self.chars.len();
    }

    pub fn insert(&mut self, text: &str) {
        for ch in text.chars().filter(|c| !c.is_control()) {
            self.chars.insert(self.cursor, ch);
            self.cursor += 1;
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.chars.len());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.chars.len();
    }

    fn word_start(&self) -> usize {
        let mut i = self.cursor;
        while i > 0 && self.chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !self.chars[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start();
    }

    pub fn word_right(&mut self) {
        let n = self.chars.len();
        let mut i = self.cursor;
        while i < n && self.chars[i].is_whitespace() {
            i += 1;
        }
        while i < n && !self.chars[i].is_whitespace() {
            i += 1;
        }
        self.cursor = i;
    }

    pub fn delete_word_left(&mut self) {
        let start = self.word_start();
        self.chars.drain(start..self.cursor);
        self.cursor = start;
    }

    pub fn delete_to_start(&mut self) {
        self.chars.drain(..self.cursor);
        self.cursor = 0;
    }

    pub fn delete_to_end(&mut self) {
        self.chars.truncate(self.cursor);
    }

    /// Places the cursor under a mouse click.
    pub fn click(&mut self, x: u16) {
        let target = self.scroll + x.saturating_sub(self.view.x) as usize;
        let mut col = 0;
        self.cursor = self.chars.len();
        for (i, ch) in self.chars.iter().enumerate() {
            let w = ch.width().unwrap_or(0);
            if col + w > target {
                self.cursor = i;
                break;
            }
            col += w;
        }
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, focused: bool, placeholder: &str) {
        let (border, bg, title) = if focused {
            (t::BORDER_FOCUS, t::SEARCH_BG_FOCUS, t::BORDER_TITLE_FOCUS)
        } else {
            (t::BORDER, t::SEARCH_BG, t::BORDER_TITLE)
        };
        tall_box(buf, area, border, bg, t::SCREEN_BG, Some(("R E C H E R C H E", title)));
        let mut view = inner(area);
        view.x += 1; // Input's own `padding: 0 1`
        view.width = view.width.saturating_sub(2);
        self.view = view;
        let w = view.width as usize;
        if w == 0 || view.height == 0 {
            return;
        }
        let cursor_style = Style::new().fg(t::CURSOR_FG).bg(t::CURSOR_BG);
        if self.chars.is_empty() {
            spans(buf, view.x, view.y, view.width, &[(placeholder, fg(t::PLACEHOLDER))]);
            if focused {
                buf[(view.x, view.y)].set_style(cursor_style);
            }
            self.scroll = 0;
            return;
        }
        let widths: Vec<usize> = self.chars.iter().map(|c| c.width().unwrap_or(0)).collect();
        let cursor_col: usize = widths[..self.cursor].iter().sum();
        if cursor_col < self.scroll {
            self.scroll = cursor_col;
        } else if cursor_col >= self.scroll + w {
            self.scroll = cursor_col + 1 - w;
        }
        let mut col = 0usize;
        let mut ch_buf = [0u8; 4];
        for (ch, cw) in self.chars.iter().zip(&widths) {
            if col >= self.scroll && col + cw <= self.scroll + w {
                let x = view.x + (col - self.scroll) as u16;
                buf.set_stringn(x, view.y, ch.encode_utf8(&mut ch_buf), *cw, fg(t::SEARCH_FG));
            }
            col += cw;
        }
        if focused && cursor_col >= self.scroll && cursor_col < self.scroll + w {
            buf[(view.x + (cursor_col - self.scroll) as u16, view.y)].set_style(cursor_style);
        }
    }
}

// --------------------------------------------------------------- track list

/// A ListView of videos: `index` is the list's own cursor (keyboard or
/// mouse) and `offset` the viewport, scrolled independently by the wheel.
pub struct VList {
    pub items: Vec<Video>,
    pub index: Option<usize>,
    pub offset: usize,
    pub title: String,
    pub view: Rect,
}

impl VList {
    pub fn new(title: &str) -> Self {
        Self { items: Vec::new(), index: None, offset: 0, title: title.into(), view: Rect::default() }
    }

    pub fn set_items(&mut self, items: Vec<Video>) {
        self.items = items;
        self.index = None;
        self.offset = 0;
    }

    fn page(&self) -> usize {
        (self.view.height as usize).max(1)
    }

    pub fn select(&mut self, index: usize) {
        if self.items.is_empty() {
            return;
        }
        let index = index.min(self.items.len() - 1);
        self.index = Some(index);
        let page = self.page();
        if index < self.offset {
            self.offset = index;
        } else if index >= self.offset + page {
            self.offset = index + 1 - page;
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        let target = match self.index {
            None => 0,
            Some(i) => (i as isize + delta).max(0) as usize,
        };
        self.select(target);
    }

    pub fn page_by(&mut self, pages: isize) {
        self.move_by(pages * self.page() as isize);
    }

    pub fn scroll(&mut self, delta: isize) {
        let max = self.items.len().saturating_sub(self.page());
        self.offset = (self.offset as isize + delta).clamp(0, max as isize) as usize;
    }

    /// Row index under a screen position, if any.
    pub fn hit(&self, x: u16, y: u16) -> Option<usize> {
        let v = self.view;
        if x < v.x || x >= v.right() || y < v.y || y >= v.bottom() {
            return None;
        }
        let i = self.offset + (y - v.y) as usize;
        (i < self.items.len()).then_some(i)
    }

    pub fn render(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        focused: bool,
        playing: Option<&str>,
        hover: Option<usize>,
    ) {
        let (border, title) =
            if focused { (t::BORDER_FOCUS, t::BORDER_TITLE_FOCUS) } else { (t::BORDER, t::BORDER_TITLE) };
        tall_box(buf, area, border, t::PANEL_BG, t::SCREEN_BG, Some((&self.title, title)));
        let view = inner(area);
        self.view = view;
        let max = self.items.len().saturating_sub(view.height as usize);
        self.offset = self.offset.min(max);
        let width = scrollbar(buf, view, self.items.len(), self.offset);
        for row in 0..view.height as usize {
            let i = self.offset + row;
            let Some(video) = self.items.get(i) else { break };
            let line = Rect::new(view.x, view.y + row as u16, width, 1);
            let is_playing = playing == Some(video.id.as_str());
            video_row(buf, line, video, is_playing, self.index == Some(i), hover == Some(i));
        }
    }
}

/// One entry of a track listing. `playing` marks the track actually coming
/// out of the speakers and takes priority over `highlighted`, the list's own
/// cursor (keyboard or mouse) — the two are independent and often disagree.
fn video_row(buf: &mut Buffer, line: Rect, video: &Video, playing: bool, highlighted: bool, hovered: bool) {
    let (bg, marker, dur, title, uploader) = if playing {
        (t::ROW_PLAYING_BG, "▶ ", t::ROW_DUR_PLAYING, bold(t::ROW_TITLE_HL), t::ROW_UPLOADER_HL)
    } else if highlighted {
        (t::ROW_HIGHLIGHT_BG, "▌ ", t::ROW_DUR_HL, bold(t::ROW_TITLE_HL), t::ROW_UPLOADER_HL)
    } else {
        let bg = if hovered { t::ROW_HOVER_BG } else { t::PANEL_BG };
        (bg, "  ", t::ROW_DUR, fg(t::ROW_TITLE), t::ROW_UPLOADER)
    };
    fill(buf, line, bg);
    let duration = format!("{:>7}  ", video.duration_str());
    let by = if video.uploader.is_empty() { String::new() } else { format!("  {}", video.uploader) };
    spans(
        buf,
        line.x,
        line.y,
        line.width,
        &[(marker, fg(t::ROW_MARKER)), (&duration, fg(dur)), (&video.title, title), (&by, fg(uploader))],
    );
}

// ----------------------------------------------------------------- spectrum

// Vertical eighths, index 0 == empty.
const EIGHTHS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

fn ramp(v: f32) -> Color {
    let v = v.clamp(0.0, 0.999);
    let pos = v * (t::RAMP.len() - 1) as f32;
    let i = pos as usize;
    let f = pos - i as f32;
    let (a, b) = (t::RAMP[i], t::RAMP[(i + 1).min(t::RAMP.len() - 1)]);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f) as u8;
    Color::Rgb(mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}

/// Peak-hold analyser, one row at a time from the top.
pub fn spectrum(buf: &mut Buffer, area: Rect, an: &Analyser) {
    let (w, h) = (area.width as usize, area.height as usize);
    if w == 0 || h == 0 {
        return;
    }
    let bar = (w / NBANDS).max(1);
    let gap = usize::from(bar > 1);
    let glyph_w = bar - gap;
    let unlit = fg(t::CHASSIS);
    let peak_style = bold(t::EMBER);
    for y in 0..h {
        let row = h - 1 - y; // counted from the baseline
        let lit = fg(ramp((row as f32 + 0.5) / h as f32));
        let mut x = area.x as usize;
        for (value, peak) in an.bands.iter().zip(&an.peaks) {
            if x + glyph_w > area.right() as usize {
                break;
            }
            let cell = value * h as f32 - row as f32;
            let (glyph, style) = if cell >= 1.0 {
                ("█", lit)
            } else if cell > 0.0 {
                (EIGHTHS[((cell * 8.0) as usize).max(1)], lit)
            } else if *peak > 0.02 && (peak * h as f32) as isize - 1 == row as isize {
                ("▔", peak_style) // peak hold: the segment that lingers above the bar
            } else if row == 0 {
                ("▁", unlit) // the meter's resting floor
            } else {
                (" ", unlit)
            };
            for dx in 0..glyph_w {
                buf[((x + dx) as u16, area.y + y as u16)].set_symbol(glyph).set_style(style);
            }
            x += bar;
        }
    }
}

// ----------------------------------------------------------------- seek bar

/// Transport line: `▶  1:23  ━━━━━◆┄┄┄┄┄  4:56`.
pub fn seek_bar(buf: &mut Buffer, area: Rect, pos: f64, dur: f64, paused: bool) {
    if area.width == 0 {
        return;
    }
    let width = (area.width as usize).max(24);
    let ratio = if dur > 0.0 { (pos / dur).min(1.0) } else { 0.0 };
    let head = if paused { "❙❙" } else { "▶ " };
    let pos_s = fmt_time(pos);
    let right = format!("  {}", if dur > 0.0 { fmt_time(dur) } else { "DIRECT".into() });
    let track = width.saturating_sub(2 + pos_s.len() + right.len() + 4).max(4);
    let done = (ratio * track as f64) as usize;
    let pos_txt = format!("  {pos_s}  ");
    let done_txt = "━".repeat(done);
    let todo_txt = "┄".repeat(track.saturating_sub(done + 1));
    let head_style = if paused { bold(t::SEEK_PAUSED) } else { bold(t::SEEK_PLAY) };
    let right_style = if dur > 0.0 { fg(t::SEEK_DUR) } else { bold(t::SEEK_LIVE) };
    spans(
        buf,
        area.x,
        area.y,
        area.width,
        &[
            (head, head_style),
            (&pos_txt, bold(t::SEEK_POS)),
            (&done_txt, fg(t::SEEK_DONE)),
            ("◆", bold(t::SEEK_HEAD)),
            (&todo_txt, fg(t::SEEK_TODO)),
            (&right, right_style),
        ],
    );
}

// --------------------------------------------------------------------- clip

/// Nearest-neighbour half-block sampling: one cell carries two source pixels
/// via `▀` (top pixel as foreground, bottom pixel as background), so the
/// picture keeps the terminal's full colour depth at twice the vertical
/// resolution. Tables are rebuilt only when the target size changes, so a
/// window resize costs a table rebuild instead of an ffmpeg restart.
/// 5 bits per channel instead of 8: invisible at half-block resolution, but
/// neighbouring cells then often share a colour (its escape is not repeated)
/// and still areas stop flickering with compression noise (the cell diff
/// skips them) — far fewer bytes for the terminal to chew through per frame.
fn quantise(c: u8) -> u8 {
    (c & 0xF8) | 0x04
}

#[derive(Default)]
pub struct HalfBlock {
    grid: (u16, u16, usize, usize),
    cols: Vec<i32>,
    rows: Vec<i32>,
}

impl HalfBlock {
    /// Letterboxed to keep the aspect ratio; -1 marks a padding column or row.
    fn tables(&mut self, w: u16, h: u16, src_w: usize, src_h: usize) {
        if self.grid == (w, h, src_w, src_h) {
            return;
        }
        self.grid = (w, h, src_w, src_h);
        let (out_w, out_h) = (w as usize, h as usize * 2); // half-blocks: square-ish pixels
        let draw_w = out_w.min(((out_h * src_w) as f64 / src_h as f64).round().max(1.0) as usize);
        let draw_h = out_h.min(((out_w * src_h) as f64 / src_w as f64).round().max(1.0) as usize);
        let (off_x, off_y) = ((out_w - draw_w) / 2, (out_h - draw_h) / 2);
        self.cols = (0..out_w)
            .map(|x| {
                if x < off_x || x >= off_x + draw_w {
                    -1
                } else {
                    ((x - off_x) * src_w / draw_w).min(src_w - 1) as i32
                }
            })
            .collect();
        self.rows = (0..out_h)
            .map(|y| {
                if y < off_y || y >= off_y + draw_h {
                    -1
                } else {
                    ((y - off_y) * src_h / draw_h).min(src_h - 1) as i32
                }
            })
            .collect();
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, frame: &Frame, src_w: usize, src_h: usize) {
        if area.width == 0 || area.height == 0 || !frame.valid || frame.data.len() < src_w * src_h * 3 {
            return;
        }
        self.tables(area.width, area.height, src_w, src_h);
        let data = &frame.data;
        let px = |row: i32, col: i32| -> Color {
            if row < 0 {
                return Color::Rgb(0, 0, 0);
            }
            let i = (row as usize * src_w + col as usize) * 3;
            Color::Rgb(quantise(data[i]), quantise(data[i + 1]), quantise(data[i + 2]))
        };
        for y in 0..area.height {
            let (top, bottom) = (self.rows[2 * y as usize], self.rows[2 * y as usize + 1]);
            if top < 0 && bottom < 0 {
                continue;
            }
            for (x, &col) in self.cols.iter().enumerate() {
                if col < 0 {
                    continue;
                }
                buf[(area.x + x as u16, area.y + y)]
                    .set_symbol("▀")
                    .set_fg(px(top, col))
                    .set_bg(px(bottom, col));
            }
        }
    }
}

// --------------------------------------------------------------- thumb grid

pub const CELL_W: usize = 24;
pub const IMAGE_H: usize = 7;
const TITLE_ROW: usize = IMAGE_H + 1;
pub const CELL_H: usize = TITLE_ROW + 1; // highlight bar + image + title line
const COL_GAP: usize = 1;

/// Alternative "SUITE" display for playlist mode: the *whole* queue as a
/// scrollable grid of thumbnails instead of a text list. The list is the
/// queue itself and never re-sliced as playback advances — nothing reflows
/// under the user while they're browsing it, only the thumbnails on screen
/// get decoded. `playing` (ember) and `selected` (amber) are tracked
/// independently, same as the text list.
///
/// Each thumbnail is decoded by ffmpeg directly at the cell's pixel size
/// (CELL_W x 2*IMAGE_H), so a frame is ~1 kB and drawing it is a straight
/// copy, no resampling.
#[derive(Default)]
pub struct ThumbGrid {
    pub selected: usize,
    pub scroll_y: usize,
    pub frames: std::collections::HashMap<String, Vec<u8>>,
    /// Bring `selected` into view at the next render, once the geometry is
    /// known — set when grid mode is entered.
    pub follow: bool,
    columns: usize,
    view: Rect,
}

impl ThumbGrid {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    fn columns_for(width: u16) -> usize {
        ((width as usize + COL_GAP) / (CELL_W + COL_GAP)).max(1)
    }

    fn rows(&self, n: usize) -> usize {
        n.div_ceil(self.columns.max(1))
    }

    fn max_scroll(&self, n: usize) -> usize {
        (self.rows(n) * CELL_H).saturating_sub(self.view.height as usize)
    }

    pub fn scroll(&mut self, delta: isize, n: usize) {
        self.scroll_y = (self.scroll_y as isize + delta).clamp(0, self.max_scroll(n) as isize) as usize;
    }

    pub fn scroll_home(&mut self) {
        self.scroll_y = 0;
    }

    pub fn scroll_end(&mut self, n: usize) {
        self.scroll_y = self.max_scroll(n);
    }

    pub fn page(&self) -> isize {
        self.view.height.max(1) as isize
    }

    /// Scrolls just enough to bring the selected cell into view.
    pub fn ensure_visible(&mut self) {
        let h = self.view.height as usize;
        if h == 0 {
            return;
        }
        let row = self.selected / self.columns.max(1);
        let (top, bottom) = (row * CELL_H, (row + 1) * CELL_H);
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if bottom > self.scroll_y + h {
            self.scroll_y = bottom.saturating_sub(h);
        }
    }

    pub fn move_sel(&mut self, key: char, n: usize) {
        if n == 0 {
            return;
        }
        let c = self.columns.max(1);
        self.selected = match key {
            'l' => self.selected.saturating_sub(1),
            'r' => (self.selected + 1).min(n - 1),
            'u' => self.selected.saturating_sub(c),
            _ => (self.selected + c).min(n - 1),
        };
        self.ensure_visible();
    }

    pub fn hit(&self, x: u16, y: u16, n: usize) -> Option<usize> {
        let v = self.view;
        if x < v.x || x >= v.right() || y < v.y || y >= v.bottom() {
            return None;
        }
        let col = (x - v.x) as usize / (CELL_W + COL_GAP);
        if col >= self.columns {
            return None;
        }
        let row = (self.scroll_y + (y - v.y) as usize) / CELL_H;
        let i = row * self.columns + col;
        (i < n).then_some(i)
    }

    /// Ids on screen that still lack a frame — the point of a real scrolled
    /// grid instead of a preloaded window: a 300-track playlist never
    /// decodes more than a screenful.
    pub fn missing<'a>(&self, videos: &'a [Video]) -> Vec<&'a Video> {
        let h = self.view.height as usize;
        if h == 0 || videos.is_empty() {
            return Vec::new();
        }
        let first = self.scroll_y / CELL_H;
        let last = (self.scroll_y + h) / CELL_H;
        let start = (first * self.columns).min(videos.len());
        let end = ((last + 1) * self.columns).min(videos.len());
        videos[start..end].iter().filter(|v| !self.frames.contains_key(&v.id)).collect()
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, focused: bool, videos: &[Video], playing: Option<&str>) {
        let (border, title) =
            if focused { (t::BORDER_FOCUS, t::BORDER_TITLE_FOCUS) } else { (t::BORDER, t::BORDER_TITLE) };
        tall_box(buf, area, border, t::PANEL_BG, t::SCREEN_BG, Some(("S U I T E   ·   miniatures", title)));
        let mut view = inner(area);
        if view.width == 0 || view.height == 0 {
            return;
        }
        // Reserve the scrollbar column only when the content overflows.
        self.columns = Self::columns_for(view.width);
        let total = self.rows(videos.len()) * CELL_H;
        if total > view.height as usize {
            view.width = scrollbar(buf, view, total, self.scroll_y);
            self.columns = Self::columns_for(view.width);
        }
        self.view = view;
        self.selected = self.selected.min(videos.len().saturating_sub(1));
        if std::mem::take(&mut self.follow) {
            self.ensure_visible();
        }
        self.scroll_y = self.scroll_y.min(self.max_scroll(videos.len()));

        if videos.is_empty() {
            let text = "aucune piste suivante";
            let pad = (view.width as usize).saturating_sub(text.len()) / 2;
            spans(buf, view.x + pad as u16, view.y + view.height / 2, view.width, &[(text, fg(t::THUMB_EMPTY))]);
            return;
        }
        for y in 0..view.height as usize {
            let content_y = self.scroll_y + y;
            let (grid_row, row_in_cell) = (content_y / CELL_H, content_y % CELL_H);
            for col in 0..self.columns {
                let index = grid_row * self.columns + col;
                let Some(video) = videos.get(index) else { break };
                let x = view.x + (col * (CELL_W + COL_GAP)) as u16;
                let line = Rect::new(x, view.y + y as u16, CELL_W as u16, 1);
                self.cell_row(buf, line, video, index, row_in_cell, playing == Some(video.id.as_str()));
            }
        }
    }

    fn cell_row(&self, buf: &mut Buffer, line: Rect, video: &Video, index: usize, row: usize, playing: bool) {
        let selected = index == self.selected;
        if row == 0 {
            // A solid bright bar reads as "selected"/"playing" regardless of
            // how busy the thumbnail under it is — a tinted background alone
            // got lost next to a colourful image. Playing (ember, matches the
            // "▶" marker elsewhere) wins over the cursor (amber).
            if playing {
                fill(buf, line, t::THUMB_PLAYING_BAR);
            } else if selected {
                fill(buf, line, t::THUMB_SELECTED_BAR);
            }
        } else if row == TITLE_ROW {
            let (bg, marker, marker_c, title) = if playing {
                (t::ROW_PLAYING_BG, "▶ ", t::ROW_MARKER, bold(t::ROW_TITLE_HL))
            } else if selected {
                (t::THUMB_SELECTED_BG, "▌ ", t::THUMB_SELECTED_MARKER, bold(t::ROW_TITLE_HL))
            } else {
                (t::PANEL_BG, "  ", t::PANEL_BG, fg(t::ROW_TITLE))
            };
            fill(buf, line, bg);
            spans(buf, line.x, line.y, line.width, &[(marker, fg(marker_c)), (&video.title, title)]);
        } else if let Some(frame) = self.frames.get(&video.id) {
            let r = row - 1; // image rows 0..IMAGE_H, two pixel rows each
            for x in 0..CELL_W.min(line.width as usize) {
                let at = |py: usize| {
                    let i = (py * CELL_W + x) * 3;
                    Color::Rgb(frame[i], frame[i + 1], frame[i + 2])
                };
                buf[(line.x + x as u16, line.y)].set_symbol("▀").set_fg(at(2 * r)).set_bg(at(2 * r + 1));
            }
        }
    }
}
