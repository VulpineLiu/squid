//! The terminal front end.
//!
//! [`run`] owns the whole screen: it installs raw mode plus the alternate
//! buffer, drives [`App::tick`] on a fixed beat, and always puts the terminal
//! back the way it found it — including when a render panics.

pub mod app;
pub mod job;
pub mod theme;
pub mod views;

use app::{Action, App};
use crossterm::{
    event::{poll, read, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{stdout, Write};
use std::time::{Duration, Instant};

/// How often the UI drains progress events and repaints.
const TICK: Duration = Duration::from_millis(250);

/// Restores the terminal on the way out, panic included. Without the panic
/// hook a bug would leave the user's shell in raw mode.
struct TerminalGuard;

impl TerminalGuard {
    fn install() -> anyhow::Result<TerminalGuard> {
        enable_raw_mode()?;
        let mut out = stdout();
        execute!(out, EnterAlternateScreen)?;
        out.flush()?;
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = TerminalGuard::restore();
            previous(info);
        }));
        Ok(TerminalGuard)
    }

    fn restore() -> std::io::Result<()> {
        let mut out = stdout();
        execute!(out, LeaveAlternateScreen)?;
        out.flush()?;
        disable_raw_mode()?;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = TerminalGuard::restore();
    }
}

/// Open the interface. Returns once the user quits.
pub fn run() -> anyhow::Result<()> {
    // Settle the colour depth before the terminal is touched at all. The answer
    // comes from the environment of the shell that launched squid, and every
    // colour the interface later emits is decided by it: a terminal that cannot
    // parse a 24-bit escape sequence prints the numbers as text, so asking
    // first and defaulting downwards is what keeps Apple Terminal and a bare
    // ssh session readable instead of garbled.
    crate::theme::init();
    let guard = TerminalGuard::install()?;
    let result = run_inner();
    drop(guard);
    result
}

fn run_inner() -> anyhow::Result<()> {
    let mut app = App::new()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.hide_cursor()?;
    terminal.clear()?;

    let mut next_tick = Instant::now() + TICK;
    while !app.should_quit {
        app.tick();
        terminal.draw(|frame| views::draw(frame, &app))?;
        if poll(next_tick.saturating_duration_since(Instant::now()))? {
            match read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if app.handle_key(key.code, key.modifiers) == Action::Quit {
                        if app.busy() && !app.confirm.is_some() {
                            app.cancel_jobs();
                            app.should_quit = true;
                        } else {
                            app.should_quit = true;
                        }
                    }
                }
                Event::Key(key)
                    if key.kind == KeyEventKind::Repeat
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c') =>
                {
                    app.cancel_jobs();
                    app.should_quit = true;
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        if Instant::now() >= next_tick {
            next_tick = Instant::now() + TICK;
        }
    }
    terminal.show_cursor()?;
    Ok(())
}
