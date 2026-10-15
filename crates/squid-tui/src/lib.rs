//! The terminal front end.
//!
//! [`run`] owns the whole screen: it installs raw mode plus the alternate
//! buffer, drives [`App::tick`] on a fixed beat, and always puts the terminal
//! back the way it found it — including when a render panics.
//!
//! It also decides what "the whole screen" means, once per beat. That sounds
//! like a detail and is the difference between an interface and a pile of
//! previous frames: see [`drawable`] and [`repaint`].
//!
//! Then it asks what a column costs, because on a console that charges two for
//! the characters the layout measured as one, every row of the interface is
//! longer than the screen and the pile of previous frames comes back on its
//! own: see [`glyphs`].

pub mod app;
pub mod glyphs;
pub mod job;
pub mod theme;
pub mod views;

use app::{Action, App};
use crossterm::{
    event::{poll, read, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal, TerminalOptions, Viewport};
use squid_platform::console::Geometry;
use std::io::{stdout, Stdout, Write};
use std::time::{Duration, Instant};

/// How often the UI drains progress events and repaints.
const TICK: Duration = Duration::from_millis(250);

/// The shape to draw into when a terminal cannot be asked for one.
const FALLBACK: (u16, u16) = (80, 24);

type Screen = Terminal<CrosstermBackend<Stdout>>;

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
    squid_platform::console::note(&format!("start {}", squid_platform::console::report()));
    let guard = TerminalGuard::install()?;
    // The line that settles the argument about which buffer the interface is
    // on: this is measured after the alternate screen went up, and it is the
    // last moment at which the screen can still be recognised as one the
    // console made for us. The `vt=` on both lines says which way the words are
    // reaching the glass, and the difference between the two lines says whether
    // the terminal library managed to turn on escape-sequence output at all.
    squid_platform::console::adopt();
    squid_platform::console::note(&format!("alternate {}", squid_platform::console::report()));
    let result = run_inner();
    squid_platform::console::note("leaving");
    drop(guard);
    squid_platform::console::note("restored");
    result
}

/// The rectangle the interface may paint.
///
/// A window that reports 180 columns is only 180 columns wide if something
/// exists to hold them. Modern terminals own both numbers and answer
/// [`Geometry::None`](Geometry), so the window is the answer; a legacy Windows
/// console reports a buffer as well, and the smaller of the two is the only
/// area where writing cannot spill onto the following row.
fn drawable(window: (u16, u16), console: Option<Geometry>) -> Rect {
    let (columns, rows) = match console {
        Some(geometry) => geometry.drawable(),
        None => window,
    };
    Rect::new(0, 0, columns, rows)
}

/// Everything the screen reports about its own shape, right now.
///
/// Three numbers, because one is not enough to notice trouble. The window is
/// what the user is looking at; the area is what may be painted without
/// spilling into the next row; the console geometry is the pair of measurements
/// behind both, and it is the only one that sees a buffer being resized
/// underneath an interface whose drawable area happens not to move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Reported {
    window: (u16, u16),
    area: Rect,
    console: Option<Geometry>,
}

/// Ask the terminal, then the console behind it, what it can hold right now.
fn current_shape() -> Reported {
    let window = crossterm::terminal::size().unwrap_or(FALLBACK);
    let console = squid_platform::console::geometry();
    Reported {
        window,
        area: drawable(window, console),
        console,
    }
}

/// The screen as it was when the interface last painted it, so that a wipe
/// happens when the shape changes and not every beat. Repainting on an
/// unchanged size would cost a flicker four times a second for the session.
///
/// What it remembers is the whole measurement on purpose. A window can grow
/// while the area stays clamped to a buffer that refused to grow with it, and
/// then the columns that just came into view are full of whatever the console
/// remembers; a buffer can also be resized by the console host while both the
/// window and the area stay put, which moves the frame without asking. Either
/// way the diff is now comparing coordinates that no longer mean the same cell.
#[derive(Debug)]
struct Drawn {
    reported: Reported,
}

impl Drawn {
    fn new(reported: Reported) -> Drawn {
        Drawn { reported }
    }

    /// `Some` when the screen is no longer the shape last drawn into.
    fn observe(&mut self, seen: Reported) -> Option<Rect> {
        (self.reported != seen).then(|| {
            self.reported = seen;
            seen.area
        })
    }
}

/// The screen changed shape, which no diff renderer survives on its own.
///
/// The frame is drawn by writing the cells that differ from the previous one.
/// Resizing moves content around inside the console without asking, so cells
/// this renderer believes are empty are holding an older frame, and cells it
/// believes are filled are somewhere else entirely. Growing the console's
/// buffer is tried first — it is the actual fault on Windows 10 — and then the
/// whole buffer is wiped so the next frame starts from a screen that matches
/// what the diff assumes.
fn repaint(terminal: &mut Screen, drawn: &mut Drawn) -> anyhow::Result<()> {
    // A console that accepted a bigger buffer has just handed back columns, so
    // the answer worth drawing into is asked for again.
    let repaired = squid_platform::console::repair();
    if repaired {
        drawn.reported = current_shape();
    }
    let area = drawn.reported.area;
    squid_platform::console::note(&format!(
        "repaint {}x{} repair={repaired}",
        area.width, area.height
    ));
    let mut out = stdout();
    execute!(out, Clear(ClearType::All))?;
    out.flush()?;
    terminal.resize(area)?;
    Ok(())
}

fn run_inner() -> anyhow::Result<()> {
    let mut app = App::new()?;
    // The viewport is fixed rather than full-screen because the size of a full
    // screen is whatever the terminal says it is, and on a legacy console that
    // answer is a lie the renderer would then paint into.
    let seen = current_shape();
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(stdout()),
        TerminalOptions {
            viewport: Viewport::Fixed(seen.area),
        },
    )?;
    let mut drawn = Drawn::new(seen);
    terminal.hide_cursor()?;
    terminal.clear()?;
    // Priced against the screen, not against the tables the layout was drawn
    // with, and only now: measuring spends one character on whatever the console
    // is showing, and the screen is not ours to spend until the alternate one is
    // up. The character it writes is covered by the first frame.
    let fit = glyphs::Fit::measure();

    let mut next_tick = Instant::now() + TICK;
    while !app.should_quit {
        app.tick();
        let wanted = current_shape();
        if drawn.observe(wanted).is_some() {
            repaint(&mut terminal, &mut drawn)?;
        }
        terminal.draw(|frame| {
            views::draw(frame, &app);
            // The last moment at which a row that is too long for this console
            // can still be made to fit is after the widgets and before the wire.
            fit.carry(frame.buffer_mut());
        })?;
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
                // Nothing to do: the beat above re-reads the geometry from the
                // console instead of trusting this event, which also covers the
                // resizes a legacy console never reports.
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

#[cfg(test)]
mod tests {
    use super::{drawable, Drawn, Reported};
    use ratatui::layout::Rect;
    use squid_platform::console::Geometry;

    /// A console that can display whatever it is showing, so only the window
    /// and the buffer are in disagreement here.
    fn console(window: (u16, u16), buffer: (u16, u16)) -> Option<Geometry> {
        Some(Geometry {
            window,
            origin: (0, 0),
            buffer,
            limit: (window.0.max(buffer.0), window.1.max(buffer.1).max(9_999)),
        })
    }

    fn reported(window: (u16, u16), area: (u16, u16), buffer: (u16, u16)) -> Reported {
        Reported {
            window,
            area: Rect::new(0, 0, area.0, area.1),
            console: console(window, buffer),
        }
    }

    fn painted(window: (u16, u16), area: (u16, u16), buffer: (u16, u16)) -> Drawn {
        Drawn::new(reported(window, area, buffer))
    }

    #[test]
    fn a_terminal_that_owns_its_buffer_keeps_the_size_it_reported() {
        // Every terminal drawn on so far — Terminal.app, Windows Terminal,
        // konsole, a bare ssh session — owns both numbers, so its word is the
        // layout's word.
        assert_eq!(drawable((140, 45), None), Rect::new(0, 0, 140, 45));
    }

    #[test]
    fn a_window_wider_than_the_cells_that_exist_is_not_a_place_to_draw() {
        // The maximized-on-Windows-10 frame: 180 columns of window over 100
        // columns of buffer. Column 101 is column 0 of the next row, and every
        // line under it slides.
        assert_eq!(
            drawable((180, 50), console((180, 50), (100, 30))),
            Rect::new(0, 0, 100, 30)
        );
    }

    #[test]
    fn a_consoles_history_is_not_extra_interface() {
        // Rows of scrollback are the shell's, not the interface's.
        assert_eq!(
            drawable((120, 40), console((120, 40), (120, 900))),
            Rect::new(0, 0, 120, 40)
        );
    }

    #[test]
    fn the_screen_is_wiped_when_it_changes_and_only_then() {
        let mut drawn = painted((100, 30), (100, 30), (100, 900));
        assert_eq!(
            drawn.observe(reported((100, 30), (100, 30), (100, 900))),
            None,
            "an unchanged screen must not be wiped again"
        );
        assert_eq!(
            drawn.observe(reported((140, 40), (140, 40), (140, 900))),
            Some(Rect::new(0, 0, 140, 40))
        );
        assert_eq!(
            drawn.observe(reported((140, 40), (140, 40), (140, 900))),
            None,
            "the wipe is once per change, not once per beat"
        );
    }

    #[test]
    fn a_console_that_scrolled_its_own_buffer_moves_the_picture_without_moving_the_size() {
        // Every number the layout reads is unchanged here, and the frame is
        // still on the wrong cells: the console slid its window down the buffer
        // underneath the interface. The measurement carries the origin for this
        // case and nobody has to explain why the screen looks frozen.
        let mut drawn = painted((120, 30), (120, 30), (120, 9_000));
        let scrolled = Reported {
            window: (120, 30),
            area: Rect::new(0, 0, 120, 30),
            console: Some(Geometry {
                window: (120, 30),
                origin: (0, 897),
                buffer: (120, 9_000),
                limit: (120, 30),
            }),
        };
        assert_eq!(drawn.observe(scrolled), Some(Rect::new(0, 0, 120, 30)));
    }

    #[test]
    fn a_window_that_grew_is_repainted_even_when_the_buffer_stayed_behind() {
        // The clamped case: 180 columns of window over the 100 columns of
        // buffer a console refused to grow. The drawing area has not moved,
        // but eighty columns of that console's memory just came into view.
        let mut drawn = painted((100, 30), (100, 30), (100, 30));
        assert_eq!(
            drawn.observe(reported((180, 30), (100, 30), (100, 30))),
            Some(Rect::new(0, 0, 100, 30)),
            "newly visible columns must not keep what the console remembered"
        );
    }

    #[test]
    fn a_buffer_moved_under_an_unchanged_window_is_repainted_too() {
        // Nothing the interface lays out has changed: the window is the same
        // and the drawable area is still clamped by it. The console host has
        // resized the buffer behind the frame, which is exactly the move a
        // coordinate-based diff cannot see, so the numbers behind the layout
        // have to be watched, not only the layout.
        let mut drawn = painted((120, 30), (120, 30), (120, 30));
        assert_eq!(
            drawn.observe(reported((120, 30), (120, 30), (120, 60))),
            Some(Rect::new(0, 0, 120, 30)),
            "a buffer that moved is a screen that has to be wiped"
        );
    }

    #[test]
    fn scrollback_that_grew_past_the_frame_is_not_a_change_of_pace() {
        // Height the console offers as history is not height the interface
        // paints into, so growth entirely inside scrollback must not buy a
        // flicker. A buffer that only grew taller than a window that is the
        // limit either way is the one case ignored on purpose.
        let mut drawn = Drawn::new(Reported {
            window: (120, 30),
            area: Rect::new(0, 0, 120, 30),
            console: console((120, 30), (120, 900)),
        });
        assert_eq!(
            drawn.observe(Reported {
                window: (120, 30),
                area: Rect::new(0, 0, 120, 30),
                console: console((120, 30), (120, 1_800)),
            }),
            Some(Rect::new(0, 0, 120, 30)),
            "documented cost: the console changed, so the frame is redrawn"
        );
    }
}
