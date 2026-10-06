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
mod widgets;

use std::io::{self, Write};
use std::sync::mpsc;
use std::time::Instant;

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui_core::terminal::Terminal;
use ratatui_crossterm::CrosstermBackend;

use app::{App, Msg};

fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut out = io::stdout();
    let _ = execute!(out, DisableBracketedPaste, DisableMouseCapture, LeaveAlternateScreen);
    let _ = execute!(out, crossterm::cursor::Show);
    let _ = out.flush();
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
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.hide_cursor()?;

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
                terminal.draw(|frame| app.draw(frame.buffer_mut()))?;
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
