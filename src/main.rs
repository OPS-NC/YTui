//! Entry point: terminal setup, the input thread and the main loop.
//!
//! The loop sleeps on one channel until either a message arrives (a key, a
//! worker's reply, a player event) or the next deadline the app asked for:
//! 20 Hz while something is moving on screen, once a second otherwise.

mod app;
mod exec;
mod history;
mod meminfo;
mod player;
mod sources;
mod theme;
mod tools;
mod widgets;

use std::io::{self, BufWriter, Write};
use std::sync::mpsc;
use std::time::Instant;

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::{execute, queue};
use crossterm::terminal::{
    BeginSynchronizedUpdate, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
    disable_raw_mode, enable_raw_mode,
};
use ratatui_core::backend::Backend;
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_crossterm::CrosstermBackend;

use app::{App, Msg};

fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut out = io::stdout();
    let _ = execute!(out, DisableBracketedPaste, DisableMouseCapture, LeaveAlternateScreen);
    let _ = execute!(out, crossterm::cursor::Show);
    let _ = out.flush();
}

/// Double-buffered screen: draw into one buffer, send only the cells that
/// differ from the previous frame.
///
/// Deliberately not `ratatui::Terminal`: on resize it snapshots the cursor
/// with a `ESC[6n` query, which needs crossterm's event reader — held by the
/// input thread blocked in `read()`. The main loop then froze until the next
/// key or mouse event, on every resize (tiling window managers resize right
/// after launch). A fullscreen app has no cursor worth restoring, so a plain
/// clear does the job.
const OUT_BUFFER: usize = 64 * 1024;

struct Screen {
    backend: CrosstermBackend<BufWriter<io::Stdout>>,
    buffers: [Buffer; 2],
    current: usize,
}

impl Screen {
    fn new() -> io::Result<Self> {
        let empty = Buffer::empty(Rect::default());
        // Stdout's own buffer is 1 KiB: a full-screen clip frame (two RGB
        // escapes per cell, ~400 kB) went out in hundreds of write(2)s.
        let out = BufWriter::with_capacity(OUT_BUFFER, io::stdout());
        Ok(Self { backend: CrosstermBackend::new(out), buffers: [empty.clone(), empty], current: 0 })
    }

    fn draw(&mut self, render: impl FnOnce(&mut Buffer)) -> io::Result<()> {
        let size = self.backend.size()?;
        let area = Rect::new(0, 0, size.width, size.height);
        if area != self.buffers[self.current].area {
            for buf in &mut self.buffers {
                buf.resize(area);
                buf.reset();
            }
            self.backend.clear()?;
        }
        // Synchronized output (DEC 2026): the terminal shows the frame once
        // it is complete instead of painting it as it streams in — no
        // tearing in the clip. Terminals without it ignore the sequence.
        queue!(self.backend, BeginSynchronizedUpdate)?;
        let (cur, prev) = (self.current, 1 - self.current);
        self.buffers[cur].reset();
        render(&mut self.buffers[cur]);
        let [a, b] = &self.buffers;
        let (prev_buf, cur_buf) = if prev == 0 { (a, b) } else { (b, a) };
        self.backend.draw(prev_buf.diff_iter(cur_buf))?;
        self.backend.hide_cursor()?;
        queue!(self.backend, EndSynchronizedUpdate)?;
        Backend::flush(&mut self.backend)?;
        self.current = prev;
        Ok(())
    }
}

fn main() -> io::Result<()> {
    sources::init_cookie_browser_from_env();
    // `--firefox`, `--chrome`… : equivalent to YTUI_COOKIES_FROM_BROWSER,
    // shorter to type.
    for arg in std::env::args().skip(1) {
        let name = arg.strip_prefix("--").unwrap_or("");
        if sources::COOKIE_BROWSERS.iter().flatten().any(|b| *b == name) {
            sources::set_cookie_browser(Some(name.to_string()));
        }
    }

    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        hook(info);
    }));

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
    let mut screen = Screen::new()?;

    let (tx, rx) = mpsc::channel::<Msg>();
    {
        let tx = tx.clone();
        exec::spawn("input", move || {
            while let Ok(ev) = crossterm::event::read() {
                if tx.send(Msg::Term(ev)).is_err() {
                    break;
                }
            }
        });
    }

    let mut app = App::new(tx);
    let result = (|| -> io::Result<()> {
        while !app.quit {
            if app.needs_draw() {
                screen.draw(|buf| app.draw(buf))?;
                app.after_draw();
            }
            let timeout = app.next_deadline().saturating_duration_since(Instant::now());
            match rx.recv_timeout(timeout) {
                Ok(msg) => {
                    app.handle(msg);
                    // Drain the burst (mouse moves, paste) before redrawing once.
                    while let Ok(msg) = rx.try_recv() {
                        app.handle(msg);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            app.tick();
        }
        Ok(())
    })();

    app.shutdown();
    restore_terminal();
    result
}
