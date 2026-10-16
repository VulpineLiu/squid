//! The console this program paints on, and the rectangle it can really hold.
//!
//! A terminal user interface runs on one assumption: the size it was told is
//! the size it may write to. Legacy Windows consoles break that assumption in
//! a way that looks like a broken renderer. The screen buffer behind the
//! alternate screen keeps the size it had when the program entered it, while
//! the window reports the size the user just dragged it to, so the console now
//! claims to be *wider* than the cells that exist. Cells written past the end
//! of a row wrap to the beginning of the next one, every line below the drag
//! shifts right, and cells that the renderer believes are empty show the
//! previous frame. A diff-based renderer cannot undo that: it compares frames
//! by coordinate, and the console moved the content between them.
//!
//! Reading the geometry is how the interface notices the lie. Making the
//! buffer agree with the window is how it repairs itself, and drawing inside
//! what is left is how it degrades when the console refuses.
//!
//! Whether the rectangle is honest is not the only question a console gets to
//! answer, and the second one is what makes a whole screen look broken at once.
//! A character that measures one column is supposed to spend one cell, and a
//! Far-Eastern console spends two on the box-drawing rules, block shading,
//! arrows, ticks and middots an interface is built out of. The window is not
//! lying any more: it is simply too small for what was written into it, every
//! row overflows by the characters it paid one cell for, and the overflow lands
//! at the start of the row below, which is the same pile of previous frames a
//! wrong size produces.
//!
//! [`prices`] asks the cells what they cost instead of guessing from the
//! locale, and it asks *character by character*. The rules, the shading and the
//! punctuation an interface is drawn from are priced one at a time by whatever
//! code page and font the console was left on, and nothing promises they will
//! all land on the same answer: the first Chinese window this was chased through
//! had room for the rounded cards and no room for the gauges inside them, which
//! is one console pricing the two sets differently. One question for the whole
//! screen is one answer for the wrong character, and the frame overflows by
//! whichever set the answer was taken from. So the answer is a price list, and
//! the interface pays ASCII only for the characters this console actually
//! overcharges, keeping the rounded card it can afford and losing only the
//! block that it cannot.
//!
//! A second question has to be answered before the first one means anything:
//! *which* buffer is being measured. The alternate screen on a legacy console is
//! a second buffer made active, and a process can still be holding the handle it
//! was started with, which then describes a screen nobody is looking at. Every
//! measurement here is taken from the buffer the console is showing, and
//! [`note`] writes both answers down side by side so the disagreement is
//! visible instead of being guessed at.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

/// What a console reports about itself, in cells.
///
/// Three numbers, because a console has three ways to be too optimistic. The
/// window is what it says it shows; the buffer is what it says it holds; the
/// limit is the largest window it could show at all, which is where the console
/// itself admits how much the window on the desk can carry. Reading only the
/// first of the three is how an interface ends up painting columns that have no
/// cells, which on a legacy console is where the wrapping starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    /// The part the user can see.
    pub window: (u16, u16),
    /// Where in the buffer that part begins.
    ///
    /// Zero, usually, and not always: a window dragged taller than a buffer
    /// with room to spare slides down its own history, and so does one the user
    /// scrolled. Nothing about the *size* changes, which is why a renderer that
    /// only watches sizes keeps painting at row zero of a buffer whose first
    /// visible row is somewhere else — an interface that draws where nobody is
    /// looking, over a screen of cells it believes it already drew.
    pub origin: (u16, u16),
    /// The part that actually exists.
    pub buffer: (u16, u16),
    /// The largest window this console could display at its current size.
    pub limit: (u16, u16),
}

impl Geometry {
    /// The largest rectangle the console can show, hold, and display at once.
    ///
    /// This is the only area it is safe to paint: anything wider wraps, and
    /// anything taller is discarded.
    pub fn drawable(&self) -> (u16, u16) {
        (
            self.smallest(|edges| edges.0),
            self.smallest(|edges| edges.1),
        )
    }

    /// The narrowest of the three claims along one axis.
    fn smallest(&self, axis: fn((u16, u16)) -> u16) -> u16 {
        axis(self.window)
            .min(axis(self.buffer))
            .min(axis(self.limit))
    }

    /// True when the window asks for cells the buffer does not contain.
    pub fn outlived_its_buffer(&self) -> bool {
        self.window.0 > self.buffer.0 || self.window.1 > self.buffer.1
    }

    /// True when the window is not sitting on the first cell of its buffer.
    ///
    /// The interface paints from (0, 0) of its own area, so anything else means
    /// the frame and the screen are two different rectangles.
    pub fn misplaced(&self) -> bool {
        self.origin != (0, 0)
    }

    /// True when this looks like a buffer the console made for this program
    /// rather than the screen a person types into.
    ///
    /// Measured on Windows 10 18363: the alternate screen arrives with no
    /// history at all, its buffer exactly the size of its window. A screen with
    /// somebody's scrollback in it is a screen that belongs to them.
    pub fn is_own_screen(&self) -> bool {
        self.buffer == self.window
    }

    /// The buffer size that would leave the window nowhere to hide.
    ///
    /// When the screen is ours the answer is the window, in both directions: a
    /// buffer the size of its window has exactly one place it can sit, so
    /// agreeing about size is also what puts the picture back underneath it.
    /// When the screen is not ours, growth is the only change on offer, because
    /// history that belongs to somebody else is not ours to cut down.
    pub fn repair_target(&self, ours: bool) -> (u16, u16) {
        if ours {
            self.window
        } else {
            (
                self.window.0.max(self.buffer.0),
                self.window.1.max(self.buffer.1),
            )
        }
    }

    /// True when the buffer is not the shape a repair would leave it in.
    pub fn needs_repair(&self, ours: bool) -> bool {
        self.repair_target(ours) != self.buffer
    }

    /// `window+origin in buffer of limit`, the shape a trace line carries, in
    /// the order a person reads a window manager's geometry.
    fn text(&self) -> String {
        format!(
            "{}x{}+{}+{} in {}x{} of {}x{}",
            self.window.0,
            self.window.1,
            self.origin.0,
            self.origin.1,
            self.buffer.0,
            self.buffer.1,
            self.limit.0,
            self.limit.1
        )
    }
}

/// Ask the console how big its window and its buffer are.
///
/// `None` means the question does not apply: standard output is a pipe, or the
/// platform's terminal is an emulator that owns both sizes itself. Callers then
/// trust the size the terminal library reported.
pub fn geometry() -> Option<Geometry> {
    #[cfg(windows)]
    {
        windows::shown()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Claim the screen being shown, if it arrived as one this program made.
///
/// The promise has to be taken from the console at the moment it is still true:
/// once something has moved underneath the interface, there is no way left to
/// tell a buffer the console made for us from a main screen the user happens to
/// keep one row of history in. Call this once, as the alternate screen goes up.
pub fn adopt() {
    #[cfg(windows)]
    {
        windows::adopt();
    }
    #[cfg(not(windows))]
    {}
}

/// One line about what kind of screen this is, for the trace.
///
/// The output code page and the virtual-terminal flag together decide how this
/// program's words reach the glass. With the flag the interface addresses the
/// window, so a window the console moves takes the picture with it; without it
/// the interface addresses absolute cells of a buffer, and a window that slides
/// down its history leaves the interface painting rows nobody is looking at.
/// Which of the two a machine is doing is not written in its version number —
/// the same build of conhost answers differently through a pseudo-console and
/// through a window on a desk — so it is measured and written down.
pub fn report() -> String {
    #[cfg(windows)]
    {
        windows::report()
    }
    #[cfg(not(windows))]
    {
        "not-a-console".to_string()
    }
}

/// Put the console back into a shape the renderer's coordinates agree with.
///
/// The buffer is made to equal the window; the window itself is left alone.
/// Every half is an attempt, and `false` means nothing changed, which is not an
/// error worth reporting to someone watching their own window resize. The
/// caller keeps drawing inside [`Geometry::drawable`] either way.
pub fn repair() -> bool {
    #[cfg(windows)]
    {
        windows::repair()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// What the console charges for a character that measures one column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolCells {
    /// One cell, which is what the character measures and what a layout
    /// measured in columns paid for it.
    Single,
    /// Two cells, so every one-column symbol on the screen spends a cell it was
    /// not bought, and every row holding one is that much longer than the glass.
    Double,
}

/// What a console charges for the characters whose price is not written down.
///
/// Held as a list rather than as one verdict, because the characters an
/// interface is drawn from do not all belong to one class: the box-drawing set
/// and the block-shading set are priced separately, by a code page and a font
/// together, and nothing binds the two to the same answer. A single verdict can
/// only be right for one of them, and whichever it gets wrong is the row that
/// overflows — which is also why the list is measured again for every console
/// rather than remembered from the last one, since the same box answers one cell
/// for the lot once it is moved to UTF-8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prices {
    /// Where the answers came from, so a log says so.
    source: &'static str,
    /// The price of every character that was asked, and no others.
    measured: Vec<(char, SymbolCells)>,
    /// The price of a character nobody asked about. This is the pessimistic
    /// answer on a code page that reprices things, which is the only safe one:
    /// a character this program forgot to ask about is a character it should
    /// not have drawn without asking.
    assumed: SymbolCells,
}

impl Prices {
    /// The answer a person named, or a test asked for: one price for every
    /// character, asked or not.
    pub fn flat(cells: SymbolCells) -> Prices {
        Prices {
            source: "asked",
            measured: Vec::new(),
            assumed: cells,
        }
    }

    /// The answer for a screen that cannot be asked, which is the one a code
    /// page implies. The function that guesses it, `charged_by_code_page`, says
    /// for itself what that answer is worth.
    pub fn implied(code_page: u32) -> Prices {
        Prices {
            source: "implied",
            measured: Vec::new(),
            assumed: charged_by_code_page(code_page),
        }
    }

    /// What the cells said, character by character, with the price a code page
    /// implies standing behind the ones they could not read.
    pub fn measured(measured: Vec<(char, SymbolCells)>, assumed: SymbolCells) -> Prices {
        let source = if measured.is_empty() {
            // Not one character landed where it could be read back, so there is
            // nothing here but the guess, and the log should say so.
            "implied"
        } else {
            "measured"
        };
        Prices {
            source,
            measured,
            assumed,
        }
    }

    /// What this console charges for one character.
    pub fn price(&self, ch: char) -> SymbolCells {
        self.measured
            .iter()
            .find(|(asked, _)| *asked == ch)
            .map(|(_, cells)| *cells)
            .unwrap_or(self.assumed)
    }

    /// Whether anything on this screen costs more than it measures, which is the
    /// one question a frame can ask before it walks itself.
    pub fn overcharged(&self) -> bool {
        self.assumed == SymbolCells::Double
            || self
                .measured
                .iter()
                .any(|(_, cells)| *cells == SymbolCells::Double)
    }

    /// One line for the trace: where the answer came from, what an unasked
    /// character is assumed to cost, and which characters cost two. The last of
    /// those is the whole reason the line exists — a person working out why a
    /// gauge came out as `#` needs to be told what the glass charged for `░`.
    pub fn text(&self) -> String {
        let double: String = self
            .measured
            .iter()
            .filter(|(_, cells)| *cells == SymbolCells::Double)
            .map(|(ch, _)| *ch)
            .collect();
        format!(
            "{} of {} characters, unasked={}, two cells={}",
            self.source,
            self.measured.len(),
            word(self.assumed),
            if double.is_empty() {
                "none".to_string()
            } else {
                double
            }
        )
    }
}

/// The word for a price, because `{:?}` of an enum variant is a Rust name and a
/// log is read by a person.
fn word(cells: SymbolCells) -> &'static str {
    match cells {
        SymbolCells::Single => "one",
        SymbolCells::Double => "two",
    }
}

/// The price a cursor's step describes, or no price at all.
///
/// One cell and two cells are the only answers a character can give. Anything
/// else is the console saying something different: nothing at all is a write
/// that did not land, a step backwards is a cursor that was moved by somebody
/// else, and a step past two is a reading that has lost the character it was
/// measuring. Sixty looked like a price once, and was not one — it was the
/// distance from a cell the console had never agreed to move the cursor to, read
/// off the sixtieth question in a row, and it is the reason this function
/// exists: an answer that cannot be a price has to become no answer, so the code
/// page answers instead of the layout believing a number with no cells behind it.
// Read by the probe on Windows and by a test anywhere; a non-Windows library
// build has no console to read a step off, so nothing here calls it.
#[cfg_attr(not(windows), allow(dead_code))]
fn price_from_step(step: i32) -> Option<SymbolCells> {
    match step {
        1 => Some(SymbolCells::Single),
        2 => Some(SymbolCells::Double),
        _ => None,
    }
}

/// Ask the console, once per character, what it charges for a one-column
/// character.
///
/// The answers are measured rather than looked up, because the two things that
/// decide them are both set outside this program: the code page the console was
/// left on and the font it was left with. They are not even the same question,
/// since `chcp 65001` moves a box-drawing bar back to one cell on the box that
/// charges two, and the same console shows that bar as a blank when the font
/// carries no glyph for it. The only answers worth laying a screen out against
/// are the ones taken from the cells themselves.
///
/// The caller names the characters worth asking about, which are the ones it
/// means to draw. The price of one of them is not evidence about another: the
/// list is as long as the question because a frame laid out against a character
/// it never asked about is laid out against a guess, and it is the guess that the
/// overflow costs.
///
/// Measuring spends characters on whatever screen the console is showing, so
/// call this when the interface owns the screen rather than while somebody's
/// shell is still on it. Each character is written, read, and taken back out
/// again, and the first frame covers the row that is left.
pub fn prices(witnesses: &[char]) -> Prices {
    if let Some(asked) = asked_for() {
        note(&format!("symbols={} asked", word(asked)));
        return Prices::flat(asked);
    }
    #[cfg(windows)]
    {
        windows::prices(witnesses)
    }
    #[cfg(not(windows))]
    {
        // No console to ask. A terminal on a writing system that charges two
        // for these characters also charges two to the layout that measures
        // them, because both read the same tables, so there is nothing to buy
        // back and the frame keeps the glyphs it was drawn with.
        let _ = witnesses;
        Prices::flat(SymbolCells::Single)
    }
}

/// The price somebody named, rather than the one the console charges.
///
/// `SQUID_SYMBOL_CELLS=2` charges two cells for every ambiguous character and
/// `=1` charges one, whatever the console says. The measurement is the honest
/// answer and this is the exception to it, for the two cases a measurement
/// cannot reach: a person looking at a screen this program cannot see — over
/// ssh, the program measures the console sshd made for it, not the one on the
/// glass — and a person who wants to see the plain frame on a terminal that
/// could afford the drawn one, to compare the two.
///
/// Anything the variable says is obeyed without argument, including on a
/// terminal that would have been drawn better. It is a person looking at their
/// own screen, and they can see what this code cannot.
fn asked_for() -> Option<SymbolCells> {
    spend(&std::env::var("SQUID_SYMBOL_CELLS").ok()?)
}

/// The two spellings of an answer: how many cells, or the word for it. A price
/// with spaces round it is still that price, because it came from an environment
/// variable somebody typed between the equals sign and the end of the line.
fn spend(asked: &str) -> Option<SymbolCells> {
    match asked.trim().to_ascii_lowercase().as_str() {
        "2" | "two" | "double" => Some(SymbolCells::Double),
        "1" | "one" | "single" => Some(SymbolCells::Single),
        _ => None,
    }
}

/// The price a code page implies, for when the cells cannot be measured.
///
/// A Far-Eastern code page has no one-byte spelling for any of the punctuation,
/// rules and shading an interface is drawn from: those characters are two bytes
/// there, and a two-byte character is two cells to the console reading it.
/// UTF-8 has no two-byte character at all, and neither has the code page a
/// person chooses with `chcp 437`, so both charge one.
#[cfg_attr(not(windows), allow(dead_code))]
fn charged_by_code_page(code_page: u32) -> SymbolCells {
    match code_page {
        932 | 936 | 949 | 950 => SymbolCells::Double,
        _ => SymbolCells::Single,
    }
}

/// Write down what the console is claiming right now, when it was asked to.
///
/// `SQUID_CONSOLE_TRACE=a\path\file.log` switches this on; with the variable
/// unset the call reads one environment entry and writes nothing. The variable
/// exists because the console that misreports its size is a console somebody
/// has to sit in front of and drag: a hand-made resize leaves no evidence
/// behind, and the whole diagnosis is about which of several numbers the
/// console changed. The file says, per line, what stdout described, what the
/// screen being shown described, and what the interface did about it.
pub fn note(what: &str) {
    #[cfg(windows)]
    {
        let Some(path) = trace_path() else { return };
        let line = measurement(
            elapsed(),
            what,
            &[("stdout", windows::stdout()), ("shown", windows::shown())],
        );
        append(path, &line);
    }
    #[cfg(not(windows))]
    {
        let _ = what;
    }
}

/// Where the trace goes, decided once so a beat that measures four times a
/// second does not re-read the environment forever.
#[cfg_attr(not(windows), allow(dead_code))]
fn trace_path() -> Option<&'static PathBuf> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| std::env::var_os("SQUID_CONSOLE_TRACE").map(PathBuf::from))
        .as_ref()
}

/// Milliseconds since the first measurement, so lines can be read in order
/// against a resize someone performed by hand.
#[cfg_attr(not(windows), allow(dead_code))]
fn elapsed() -> u128 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis()
}

/// One line of the trace: when, what happened, and what each handle said.
#[cfg_attr(not(windows), allow(dead_code))]
fn measurement(elapsed: u128, what: &str, cells: &[(&str, Option<Geometry>)]) -> String {
    let mut line = format!("+{elapsed:>6}ms {what}");
    for (name, geometry) in cells {
        let claim = geometry
            .as_ref()
            .map(Geometry::text)
            .unwrap_or_else(|| "none".to_string());
        line.push_str(&format!("  {name}={claim}"));
    }
    line
}

#[cfg_attr(not(windows), allow(dead_code))]
fn append(path: &Path, line: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

#[cfg(windows)]
mod windows {
    use super::{charged_by_code_page, price_from_step, Geometry, Prices, SymbolCells};
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock};
    use windows_sys::Win32::Foundation::{
        CloseHandle, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetFileType, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        FillConsoleOutputCharacterW, GetConsoleMode, GetConsoleOutputCP,
        GetConsoleScreenBufferInfo, GetStdHandle, SetConsoleCursorPosition,
        SetConsoleScreenBufferSize, WriteConsoleW, CONSOLE_SCREEN_BUFFER_INFO, COORD,
        ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_OUTPUT_HANDLE,
    };

    type Handle = *mut c_void;

    /// A console refuses a buffer larger than this, and `srWindow` is measured
    /// in buffer coordinates, so a window wider than it cannot be satisfied by
    /// growing anything.
    const SPAN: i32 = 9_999;

    /// A console handle that closes itself, since `CONOUT$` is opened fresh
    /// every time it is asked about.
    struct Owned(Handle);

    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// The buffer this process' bytes go to.
    fn stdout_handle() -> Option<Handle> {
        let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return None;
        }
        Some(handle)
    }

    /// True when standard output really is a console rather than a pipe or a
    /// redirected file. Without this test a `squid > log.txt` would happily
    /// resize the console sitting behind the file it is no longer writing to.
    fn stdout_is_console() -> bool {
        let Some(handle) = stdout_handle() else {
            return false;
        };
        let mut mode = 0u32;
        unsafe { GetConsoleMode(handle, &mut mode) != 0 }
    }

    /// The buffer the console is displaying, opened fresh because the console
    /// can switch it at any moment and a cached handle would keep describing
    /// the screen that was there when we looked.
    fn shown_handle() -> Option<Owned> {
        if !stdout_is_console() {
            return None;
        }
        let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return None;
        }
        Some(Owned(handle))
    }

    fn info(handle: Handle) -> Option<CONSOLE_SCREEN_BUFFER_INFO> {
        let mut info = std::mem::MaybeUninit::<CONSOLE_SCREEN_BUFFER_INFO>::uninit();
        let ok = unsafe { GetConsoleScreenBufferInfo(handle, info.as_mut_ptr()) };
        if ok == 0 {
            return None;
        }
        Some(unsafe { info.assume_init() })
    }

    /// `srWindow` is a rectangle, not a size, so the window is one cell wider
    /// and taller than subtracting its corners suggests.
    fn geometry_of(info: &CONSOLE_SCREEN_BUFFER_INFO) -> Geometry {
        let span =
            |low: i16, high: i16| (i32::from(high) - i32::from(low) + 1).clamp(1, SPAN) as u16;
        let axis = |cells: i16| i32::from(cells).clamp(1, SPAN) as u16;
        let edge = |cells: i16| i32::from(cells).clamp(0, SPAN) as u16;
        Geometry {
            window: (
                span(info.srWindow.Left, info.srWindow.Right),
                span(info.srWindow.Top, info.srWindow.Bottom),
            ),
            origin: (edge(info.srWindow.Left), edge(info.srWindow.Top)),
            buffer: (axis(info.dwSize.X), axis(info.dwSize.Y)),
            limit: ceiling(info),
        }
    }

    /// `dwMaximumWindowSize`: the largest window this console could display.
    ///
    /// It is worth measuring and drawing inside, and it is not worth asking
    /// permission from: it can sit below the buffer (a console with a thousand
    /// rows of history can display far fewer) and level with it (the alternate
    /// screen, where the two are the same number). A repair asks the console
    /// instead.
    fn ceiling(info: &CONSOLE_SCREEN_BUFFER_INFO) -> (u16, u16) {
        (
            i32::from(info.dwMaximumWindowSize.X).clamp(1, SPAN) as u16,
            i32::from(info.dwMaximumWindowSize.Y).clamp(1, SPAN) as u16,
        )
    }

    /// What the handle this process was started with claims, which is the
    /// screen its bytes go to — and, on a console that switched buffers under
    /// the program, not necessarily the screen anybody can see.
    pub fn stdout() -> Option<Geometry> {
        stdout_handle().and_then(|handle| info(handle).as_ref().map(geometry_of))
    }

    /// The size of the screen the interface is painting, which is the size the
    /// console will reflow when it changes.
    ///
    /// This is the buffer the terminal library measures too: a legacy console
    /// that is asked through the handle the process started with answers about
    /// the main screen while the interface is on the alternate one, and the two
    /// numbers being different is what a resized window turns into ghosting.
    pub fn shown() -> Option<Geometry> {
        shown_handle()
            .as_ref()
            .and_then(|owned| info(owned.0).as_ref().map(geometry_of))
    }

    /// Whether the screen being shown is the one this program asked for.
    ///
    /// Latched once, by [`adopt`], because the question can only be answered
    /// while the screen is still the way the console handed it over.
    static OWNED: AtomicBool = AtomicBool::new(false);

    pub fn adopt() {
        if let Some(geometry) = shown() {
            OWNED.store(geometry.is_own_screen(), Ordering::Relaxed);
        }
    }

    pub fn report() -> String {
        let code_page = unsafe { GetConsoleOutputCP() };
        let mut mode = 0u32;
        let mut kind = 0;
        if let Some(handle) = stdout_handle() {
            unsafe {
                GetConsoleMode(handle, &mut mode);
                kind = GetFileType(handle);
            }
        }
        // GetFileType: 1 is a disk, 2 a character device (a console, of one kind
        // or another), 3 a pipe. Which one decides whether the interface talks
        // to conhost in cells or in escape sequences.
        let kind = match kind {
            0 => "unknown",
            1 => "disk",
            2 => "char",
            3 => "pipe",
            _ => "other",
        };
        format!(
            "cp={code_page} kind={kind} vt={} ours={}",
            mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0,
            OWNED.load(Ordering::Relaxed)
        )
    }

    /// The price list, read off the cells of the console this process draws on.
    ///
    /// See [`super::prices`] for why the question is asked of every character
    /// rather than of one, and why the ones that cannot be answered fall back on
    /// the code page rather than on the average of the others.
    pub fn prices(witnesses: &[char]) -> Prices {
        let code_page = unsafe { GetConsoleOutputCP() };
        // Measure the ruler before measuring the screen. `A` is one cell in
        // every code page ever written, so a console that charges anything else
        // for it is not charging anything: it is refusing to be read, and an
        // answer taken from it is a number with no cells behind it. The code
        // page answers for the whole screen in that case, which is the guess the
        // log admits to rather than a measurement the layout trusts by mistake.
        if price_of('A') != Some(SymbolCells::Single) {
            super::note("symbols unreadable: A does not cost one cell");
            return Prices::implied(code_page);
        }
        Prices::measured(
            witnesses
                .iter()
                .filter_map(|&ch| Some((ch, price_of(ch)?)))
                .collect(),
            charged_by_code_page(code_page),
        )
    }

    /// Paint two spaces over a cell and whatever was standing behind it.
    ///
    /// Two cells, because the character being taken back out may have cost two,
    /// and clearing only the first is the half-clear that leaves a spacer on the
    /// glass for the next frame to inherit.
    fn blank(owned: Handle, at: COORD) {
        let mut written = 0u32;
        unsafe {
            FillConsoleOutputCharacterW(owned, b' ' as u16, 2, at, &mut written);
        }
    }

    /// What one character costs: measured the first time it is asked about, and
    /// remembered afterwards — including the cases where no answer could be
    /// read, because a character the console refuses to price is not worth
    /// writing to the screen again every beat.
    fn price_of(ch: char) -> Option<SymbolCells> {
        static PRICES: OnceLock<Mutex<HashMap<char, Option<SymbolCells>>>> = OnceLock::new();
        let answers = PRICES.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some(known) = answers
            .lock()
            .ok()
            .and_then(|known| known.get(&ch).copied())
        {
            return known;
        }
        let measured = measure_symbol_cells(ch);
        if let Ok(mut known) = answers.lock() {
            known.insert(ch, measured);
        }
        measured
    }

    /// Write one character and read its price off the step the cursor took.
    ///
    /// The cursor is asked to sit in the top-left cell of the window first,
    /// because that is the safest cell to spend a character in: a wide one in the
    /// last cell of the last row has nowhere to put its second cell, and a wrap
    /// there scrolls the screen this program is about to draw on. Asking is not
    /// getting. Measured on the 18363 box, through the buffer the console is
    /// showing, while the alternate screen is up: `SetConsoleCursorPosition`
    /// answers that it succeeded and leaves the cursor exactly where it was. The
    /// first version of this function anchored its arithmetic to the cell it had
    /// *asked* for rather than to the cell the cursor was actually standing on,
    /// so what it read off sixty characters was not sixty prices but sixty
    /// distances from an anchor nobody moved: one cell for the first character,
    /// two for the second, sixty for the sixtieth. A console that was charging
    /// one cell for every ambiguous character on code page 65001 therefore
    /// reported itself a screenful of two-cell ones, the layout quietly gave up
    /// the rounded frames it could afford, and the probe built to catch one
    /// overflow produced a different one.
    ///
    /// So the anchor is the cursor *found*, read back after the request, and the
    /// price is the *step* between two readings of it — a price that does not
    /// care whether the reset was obeyed. Where it was, every character is asked
    /// in the same cell; where it was not, the questions walk the top row one
    /// step at a time, and [`blank`] takes each one back out the moment it has
    /// been answered, so what is left behind is a row of empty cells that the
    /// first frame paints over regardless.
    ///
    /// `None` — no answer, fall back on the code page — when there is no screen
    /// to ask, when the character did not land, when the cursor did not move,
    /// when it ended up anywhere other than one or two columns further along the
    /// same row, and when the row has no room left to answer in. A price of zero
    /// is deliberately not an answer: a console that writes nothing where it was
    /// told to write is not one whose prices can be read off it.
    fn measure_symbol_cells(ch: char) -> Option<SymbolCells> {
        let owned = shown_handle()?;
        let before = info(owned.0)?;
        let at = COORD {
            X: before.srWindow.Left,
            Y: before.srWindow.Top,
        };
        // Ask, and then find out where the cursor really is. The request is
        // worth making even on a console that ignores it, because on the ones
        // that obey all sixty questions are asked in one cell instead of in
        // sixty of them.
        unsafe {
            SetConsoleCursorPosition(owned.0, at);
        }
        let room = info(owned.0)?;
        let here = room.dwCursorPosition;
        // Leave room for the widest answer this function can read. Walking into
        // the last column of the window would spend a character on a guess the
        // console cannot confirm, and the cell after the window's edge is where
        // a wrap begins.
        if i32::from(here.X) > i32::from(room.srWindow.Right) - 2 {
            return None;
        }
        blank(owned.0, here);
        let glyph: Vec<u16> = ch.to_string().encode_utf16().collect();
        let mut written = 0u32;
        let ok = unsafe {
            WriteConsoleW(
                owned.0,
                glyph.as_ptr().cast(),
                1,
                &mut written,
                std::ptr::null(),
            )
        };
        let after = info(owned.0);
        // Take the character back out of the screen, so the next character is
        // asked in an empty cell for the same reason this one was, and so a
        // program that is only borrowing somebody's console leaves no debt.
        blank(owned.0, here);
        unsafe { SetConsoleCursorPosition(owned.0, before.dwCursorPosition) };
        if ok == 0 || written != 1 {
            return None;
        }
        // Whatever the cursor stepped past is what the character cost. Reading it
        // back from the console instead of counting bytes is the point: the
        // console is the thing this program has to agree with.
        let after = after?;
        if after.dwCursorPosition.Y != here.Y {
            return None;
        }
        price_from_step(i32::from(after.dwCursorPosition.X) - i32::from(here.X))
    }

    /// Make the buffer agree with the window, and report whether it did.
    ///
    /// The window is left alone, which is a change of mind worth keeping in
    /// words: an earlier version of this function moved the window back onto the
    /// first cell of its buffer and won that argument for exactly one frame. A
    /// console that has settled its window at the bottom of a taller buffer puts
    /// it straight back, and the screen scrolls one row every time it does —
    /// after which the renderer's diff and the glass are a row apart everywhere,
    /// which is the precise damage this function exists to prevent. Measured on
    /// the 18363 box: the origin went 0, then 17, then 20, one scroll per
    /// repair, while the size never changed at all.
    ///
    /// The buffer is the half that can move without a fight, and a buffer the
    /// size of its window has only one place it can be. Agreeing about the size
    /// therefore puts the picture back under the window by itself, without
    /// scrolling anything.
    pub fn repair() -> bool {
        let Some(owned) = shown_handle() else {
            return false;
        };
        let Some(info) = info(owned.0) else {
            return false;
        };
        let geometry = geometry_of(&info);
        let ours = OWNED.load(Ordering::Relaxed);
        if !geometry.needs_repair(ours) {
            return false;
        }
        let want = geometry.repair_target(ours);
        let size = COORD {
            X: want.0 as i16,
            Y: want.1 as i16,
        };
        // There is no ceiling worth checking here, which is worth stating
        // because an earlier version of this function had one and it made the
        // repair impossible. On the box the bug was reported from, the buffer
        // behind the alternate screen is exactly the size of the window, so the
        // largest window the console will admit to is the size the window
        // already is: every request looks illegal to a caller that compares
        // against `limit`, and the answer is `false` in the one situation this
        // function exists for. The console knows whether the cells exist and
        // this function does not, so its return value is the only measurement
        // worth trusting.
        unsafe { SetConsoleScreenBufferSize(owned.0, size) != 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        charged_by_code_page, measurement, price_from_step, spend, Geometry, Prices, SymbolCells,
    };

    /// A console with room above the window, so the limit is not what holds
    /// this one back.
    fn console(window: (u16, u16), buffer: (u16, u16)) -> Geometry {
        let limit = (window.0.max(buffer.0), window.1.max(buffer.1).max(9_999));
        Geometry {
            window,
            origin: (0, 0),
            buffer,
            limit,
        }
    }

    /// A console whose window asks for more cells than it could ever show.
    fn too_wide(window: (u16, u16), limit: (u16, u16)) -> Geometry {
        let buffer = (window.0.max(limit.0), window.1.max(limit.1));
        Geometry {
            window,
            origin: (0, 0),
            buffer,
            limit,
        }
    }

    #[test]
    fn a_step_that_cannot_be_a_price_is_not_an_answer() {
        // The two real prices, and the readings that are not prices at all: a
        // write that never landed, a cursor that went the wrong way, and a step
        // so large it can only be the probe losing the character it was asked
        // about. That last one is not hypothetical — this probe once reported
        // `cost=60` for a middot, because the cursor had been asked rather than
        // told to go home and the sixty questions were therefore measured from
        // wherever the fifty-ninth had left it. Every character past the second
        // came back "two cells", and a console that charged one for all of them
        // was drawn as if it charged two.
        assert_eq!(price_from_step(1), Some(SymbolCells::Single));
        assert_eq!(price_from_step(2), Some(SymbolCells::Double));
        for not_a_price in [0, -1, 3, 60] {
            assert_eq!(
                price_from_step(not_a_price),
                None,
                "{not_a_price} is a cursor moving, not a character costing"
            );
        }
    }

    #[test]
    fn a_far_eastern_code_page_charges_two_cells_for_a_one_column_character() {
        // Shift-JIS, GBK, Unified Hangul and Big5: the four consoles this
        // fallback exists for, and the reason the interface does not trust its
        // own arithmetic about character widths on a Chinese Windows box.
        for code_page in [932, 936, 949, 950] {
            assert_eq!(
                charged_by_code_page(code_page),
                SymbolCells::Double,
                "code page {code_page} spells every rule and arrow in two bytes"
            );
        }
    }

    #[test]
    fn a_console_can_price_the_rules_fairly_and_the_shading_dishonestly() {
        // The reason a price is a list and not a verdict: the two sets a frame is
        // drawn from can land on different answers, and a frame that treats them
        // as one question overflows by whichever set its single answer was taken
        // from. This is the unlucky shape of it — rules affordable, shading not —
        // which is what the first Chinese window looked like, rounded cards that
        // fitted and gauges that did not. Whether any particular console charges
        // this way is measured rather than assumed here; the list exists so the
        // layout never has to decide which half of the screen it is spending.
        let prices = Prices::measured(
            vec![
                ('│', SymbolCells::Single),
                ('─', SymbolCells::Single),
                ('█', SymbolCells::Double),
                ('░', SymbolCells::Double),
            ],
            SymbolCells::Single,
        );
        assert_eq!(prices.price('│'), SymbolCells::Single);
        assert_eq!(prices.price('█'), SymbolCells::Double);
        assert!(prices.overcharged());
        let line = prices.text();
        assert!(
            line.contains("█░"),
            "{line} should name the glyphs at fault"
        );
        assert!(
            line.contains("measured"),
            "{line} should say where it came from"
        );
    }

    #[test]
    fn a_character_nobody_asked_about_costs_what_the_code_page_charges() {
        // Forgetting to ask is not forgetting to pay. The fallback is the
        // pessimistic answer, so a glyph added to a card without its price being
        // asked comes out as ASCII rather than as a row off the edge of a
        // Chinese console.
        let chinese = Prices::implied(936);
        assert_eq!(chinese.price('█'), SymbolCells::Double);
        assert!(chinese.overcharged());
        assert!(chinese.text().starts_with("implied"));

        let utf8 = Prices::implied(65001);
        assert_eq!(utf8.price('█'), SymbolCells::Single);
        assert!(!utf8.overcharged());

        let asked = Prices::flat(SymbolCells::Double);
        assert_eq!(asked.price('a'), SymbolCells::Double);
    }

    #[test]
    fn a_named_price_is_obeyed_and_a_rubbish_one_is_ignored() {
        // The override is the only way to lay out for a screen this program is
        // not sitting in front of, so it has to work both ways and shut up
        // quietly when it holds a word that is not a price.
        assert_eq!(spend("2"), Some(SymbolCells::Double));
        assert_eq!(spend(" Double "), Some(SymbolCells::Double));
        assert_eq!(spend("1"), Some(SymbolCells::Single));
        assert_eq!(spend("SINGLE"), Some(SymbolCells::Single));
        assert_eq!(spend("as big as you like"), None);
        assert_eq!(spend(""), None);
    }

    #[test]
    fn a_code_page_with_no_two_byte_characters_charges_one() {
        // 65001 is the one a person can choose on the same machine to get the
        // interface's glyphs back; 437 is the one a console arrives with.
        for code_page in [65001, 437, 1252, 0] {
            assert_eq!(charged_by_code_page(code_page), SymbolCells::Single);
        }
    }

    #[test]
    fn a_window_that_fits_its_buffer_is_a_place_to_draw() {
        let tight = console((120, 40), (120, 300));
        assert!(!tight.outlived_its_buffer());
        assert_eq!(tight.drawable(), (120, 40));
    }

    #[test]
    fn scrollback_is_height_the_interface_does_not_get() {
        // A console with a thousand rows of history is still forty rows of
        // interface. Painting into the rest would push the prompt away.
        let tight = console((100, 40), (100, 1_000));
        assert_eq!(tight.drawable(), (100, 40));
    }

    #[test]
    fn a_window_wider_than_its_buffer_is_not_a_place_to_draw() {
        // The 1909 conhost case: the alternate buffer is still 100 columns,
        // the maximized window claims 180. Writing column 101 wraps to the
        // next row and everything below it shifts.
        let tight = console((180, 50), (100, 30));
        assert!(tight.outlived_its_buffer());
        assert_eq!(tight.drawable(), (100, 30));
    }

    #[test]
    fn a_repair_grows_the_buffer_to_the_window_and_no_further() {
        let tight = console((180, 50), (100, 30));
        assert!(tight.needs_repair(false));
        assert_eq!(tight.repair_target(false), (180, 50));
        // Nothing to grow when the buffer already carries the window, even
        // when it towers over it with scrollback.
        let roomy = console((120, 40), (120, 900));
        assert_eq!(roomy.repair_target(false), (120, 900));
        assert!(!roomy.needs_repair(false));
        assert!(!roomy.outlived_its_buffer());
    }

    #[test]
    fn a_buffer_that_grew_underneath_us_is_brought_back_to_the_window() {
        // Measured on the 18363 box: something grew the screen to 180x50 while
        // the window stayed 100x30, and the window slid down to row 20 to sit
        // at the bottom of what it now had. The interface went on painting rows
        // 0..30 of a screen showing rows 20..50 — an interface drawing where
        // nobody is looking, and a diff comparing cells it can no longer see.
        let ours = Geometry {
            window: (100, 30),
            origin: (0, 20),
            buffer: (180, 50),
            limit: (180, 50),
        };
        // Note the `false`: by now the screen no longer looks like one the
        // console made for us, which is why `adopt` has to take the promise
        // early rather than ask whenever a repair is wanted.
        assert!(!ours.is_own_screen());
        assert!(ours.misplaced());
        assert!(ours.needs_repair(true));
        // The window does not move, because conhost would move it back and the
        // screen would scroll one row each time it did. The buffer comes down to
        // meet it, which leaves the window exactly one place to be.
        assert_eq!(ours.repair_target(true), (100, 30));
    }

    #[test]
    fn somebody_elses_history_is_never_cut_down_to_our_window() {
        // The same shape on a screen this program did not make: nine thousand
        // rows of somebody's commands, with the window parked at the bottom of
        // them. That is a console working as designed, and shrinking it to the
        // window would delete the record to fix a picture that was never broken.
        let theirs = Geometry {
            window: (120, 30),
            origin: (0, 8_971),
            buffer: (120, 9_001),
            limit: (120, 30),
        };
        assert!(!theirs.is_own_screen());
        assert!(!theirs.needs_repair(false));
        assert_eq!(theirs.repair_target(false), (120, 9_001));
    }

    #[test]
    fn a_screen_with_no_history_announces_itself_as_ours() {
        // What `adopt` latches on: the alternate screen arrives with nowhere to
        // scroll, so its buffer already equals its window.
        assert!(console((120, 30), (120, 30)).is_own_screen());
        assert!(!console((120, 30), (120, 9_001)).is_own_screen());
    }

    #[test]
    fn a_window_the_console_could_never_show_is_not_a_place_to_draw() {
        // The claim on the desk: a window of 180 columns on a console whose
        // font and pixels can display 150 of them. The buffer is big enough to
        // hold 180, so only the limit knows the last thirty columns have no
        // cells — and growing anything cannot invent them.
        let tight = too_wide((180, 50), (150, 9_999));
        assert_eq!(tight.drawable(), (150, 50));
    }

    #[test]
    fn an_alternate_buffer_the_size_of_its_window_leaves_nothing_to_grow() {
        // Measured on Windows 10 18363: entering the alternate screen leaves a
        // buffer the size of the window, and the console's own maximum is that
        // same number. Nothing here can grow, so the frame stays inside the
        // cells that exist and waits for the user to move the window.
        let tight = Geometry {
            window: (120, 30),
            origin: (0, 0),
            buffer: (120, 30),
            limit: (120, 30),
        };
        assert!(!tight.outlived_its_buffer());
        assert!(tight.is_own_screen());
        assert_eq!(tight.repair_target(true), (120, 30));
        assert!(!tight.needs_repair(true));
        assert!(!tight.needs_repair(false));
        assert_eq!(tight.drawable(), (120, 30));
    }

    #[test]
    fn a_repair_asks_for_the_window_even_when_the_limit_disagrees() {
        // The ask is the window, not the ceiling: whether those cells can exist
        // is a question the console answers with its return value. Refusing
        // first is what made this repair unreachable on the machine it was
        // written for, because there the limit is the size of the window.
        let tight = too_wide((180, 50), (150, 60));
        assert_eq!(tight.repair_target(false), (180, 60));
        assert!(tight.repair_target(false).0 > tight.limit.0);
        assert_eq!(tight.drawable(), (150, 50));
    }

    #[test]
    fn a_trace_line_writes_both_answers_side_by_side() {
        // The whole point of the file: one number tells a story, two numbers
        // tell which buffer lied.
        let line = measurement(
            1_530,
            "resize",
            &[
                ("stdout", Some(console((180, 50), (100, 30)))),
                ("shown", Some(console((180, 50), (180, 50)))),
            ],
        );
        assert_eq!(
            line,
            "+  1530ms resize  stdout=180x50+0+0 in 100x30 of 180x9999  \
               shown=180x50+0+0 in 180x50 of 180x9999"
        );
    }

    #[test]
    fn a_window_scrolled_down_its_buffer_still_holds_the_frame_it_asked_for() {
        // The size is honest here and the picture is still wrong: the console
        // moved the visible rectangle down its own history, so the first cells
        // the interface paints are not the first cells anybody can see. The
        // drawable area is unchanged, which is exactly why the size alone is
        // not enough to notice — and why the origin is measured at all.
        let scrolled = Geometry {
            window: (120, 30),
            origin: (0, 897),
            buffer: (120, 9_000),
            limit: (120, 30),
        };
        assert!(scrolled.misplaced());
        assert!(!scrolled.outlived_its_buffer());
        assert_eq!(scrolled.drawable(), (120, 30));
    }

    #[test]
    fn a_handle_that_is_not_a_console_is_written_as_none() {
        let line = measurement(4, "start", &[("stdout", None), ("shown", None)]);
        assert_eq!(line, "+     4ms start  stdout=none  shown=none");
    }
}
