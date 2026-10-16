//! `squid` — one tool for cleaning, uninstalling, purging and watching a Mac.
//!
//! The binary is deliberately thin: every decision about *what* may be deleted
//! is made in `squid-engine`, and every deletion goes through
//! `squid_engine::delete`, which re-checks each path against the safety policy
//! and the ledger. This file only parses arguments, shows progress and asks for
//! confirmation.

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use crossterm::tty::IsTty;
use squid_core::progress::{CancelToken, Event, Progress};
use squid_core::{
    paths, Bytes, Category, CleanSummary, CleanupItem, DeleteMode, Locale, ScanReport, Settings,
};
use squid_engine::analyze;
use squid_engine::delete::{self, DeleteJob};
use squid_engine::{clean, purge, uninstall, Engine};
use squid_platform::currency::Exchange;
use squid_platform::TrashTarget;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

// ------------------------------------------------------------------------ CLI

#[derive(Parser, Debug)]
#[command(
    name = "squid",
    version,
    about = "clean · uninstall · purge · watch — a system utility that never touches what it does not own",
    long_about = None,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Machine-readable output where supported
    #[arg(long, global = true)]
    json: bool,

    /// Only the essential lines
    #[arg(long, short = 'q', global = true)]
    quiet: bool,

    /// Never emit ANSI colour
    #[arg(long, global = true)]
    no_color: bool,

    /// Interface language: `zh` or `en`. Overrides the locale the terminal
    /// exported, which Terminal.app does not export at all.
    #[arg(long, global = true, value_name = "LANG")]
    lang: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Clear caches, logs, browser junk, DerivedData and other regenerable files
    Clean(CleanArgs),
    /// Remove an application together with everything it left behind
    Uninstall(UninstallArgs),
    /// Deep clean: node_modules, build folders, disk images, stale archives
    Purge(PurgeArgs),
    /// Open the interface: disk map, system meters, cleanup, history
    State,
    /// One-shot system snapshot (scripts, dotfiles, cron)
    Status(StatusArgs),
    /// Who owns this disk? A size tree for one path
    Analyze(AnalyzeArgs),
    /// What squid has already removed
    History(HistoryArgs),
    #[command(about = format!("Put a session's files back from the {}", bin()))]
    Undo(UndoArgs),
    /// Protect a path from every future scan
    Whitelist(WhitelistArgs),
    /// List the cleanup rules squid knows
    Rules,
    /// Report permissions, settings and ledger health
    Doctor,
}

#[derive(Args, Debug)]
struct CleanArgs {
    /// Show what would go without deleting anything
    #[arg(long)]
    dry_run: bool,
    /// Do not ask
    #[arg(long, short = 'y')]
    yes: bool,
    /// Only these categories (caches, browsers, logs, developer, …)
    #[arg(long, value_delimiter = ',')]
    categories: Vec<String>,
    /// Also look for leftovers of apps that are gone
    #[arg(long)]
    orphans: bool,
    /// Override the per-rule age filter, in days (0 disables it)
    #[arg(long)]
    min_age: Option<u64>,
    /// Skip rules that need administrator rights
    #[arg(long)]
    user_only: bool,
    #[arg(long, conflicts_with = "permanent", help = format!("Move to {} (default)", bin()))]
    trash: bool,
    #[arg(long, conflicts_with = "trash", help = format!("Delete for real, skipping the {}", bin()))]
    permanent: bool,
}

#[derive(Args, Debug)]
struct UninstallArgs {
    /// Application name or bundle identifier
    name: Option<String>,
    /// List installed applications
    #[arg(long)]
    list: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long, short = 'y')]
    yes: bool,
    #[arg(long, conflicts_with = "permanent", help = format!("Move to {} (default)", bin()))]
    trash: bool,
    #[arg(long, conflicts_with = "trash", help = format!("Delete for real, skipping the {}", bin()))]
    permanent: bool,
}

#[derive(Args, Debug)]
struct PurgeArgs {
    /// Where to look; defaults to the configured purge roots
    paths: Vec<String>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long, short = 'y')]
    yes: bool,
    /// Also offer folders that might still be needed
    #[arg(long)]
    all: bool,
    /// Age filter in days (default from settings)
    #[arg(long)]
    min_age: Option<u64>,
}

#[derive(Args, Debug)]
struct StatusArgs {
    /// How many processes to list
    #[arg(long, default_value_t = 8)]
    processes: usize,
}

#[derive(Args, Debug)]
struct AnalyzeArgs {
    /// Path to measure (default: your home folder)
    path: Option<String>,
    /// Levels below the root
    #[arg(long, default_value_t = 1)]
    depth: usize,
    /// Children to show per folder
    #[arg(long, default_value_t = 24)]
    top: usize,
    /// Also list the largest files
    #[arg(long)]
    files: bool,
}

#[derive(Args, Debug)]
struct HistoryArgs {
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

#[derive(Args, Debug)]
struct UndoArgs {
    #[arg(help = format!("Session id; the newest session with files in the {} when omitted", bin()))]
    session: Option<String>,
}

#[derive(Args, Debug)]
struct WhitelistArgs {
    #[command(subcommand)]
    action: WhitelistAction,
}

#[derive(Subcommand, Debug)]
enum WhitelistAction {
    /// Never offer these paths again
    Add { paths: Vec<String> },
    /// Stop protecting these paths
    Remove { paths: Vec<String> },
    /// Show the list
    List,
}

static COLOR: AtomicBool = AtomicBool::new(false);
static QUIET: AtomicBool = AtomicBool::new(false);
/// With `--json` stdout is a data stream: every human line is rerouted to
/// stderr so `squid clean --json | jq` never sees a progress message.
static DIAG_STDERR: AtomicBool = AtomicBool::new(false);

/// What to print when this console cannot afford the characters a report is
/// drawn out of.
///
/// The interface walks a finished frame; a command line has no frame, so its
/// frame is one line, and this is where a line stops being text and starts
/// being cells. It is the same exchange the interface spends, reached from the
/// other side — a gauge here repeats `█` twenty-eight times, and on the console
/// that turns the interface into a pile of half-frames it prints a Storage
/// table whose rows are each a screen and a half long.
///
/// The price is not measured here, even though measuring is what the interface
/// does. Measuring spends characters on whatever the console is showing at the
/// time, and a command line is a guest on that screen: it is in a pipeline, or
/// somebody's shell is still sitting there waiting for its next prompt, and a
/// row of sixty test characters written and erased in front of them is somebody
/// else's work interrupted. So the command line takes the pessimistic price the
/// code page implies and buys nothing, which costs it a plainer gauge on a
/// console that a measurement might have cleared. A table that fits is worth
/// more than a prettier one that does not.
///
/// See [`squid_platform::console::prices_unasked`] for that reasoning at the
/// counter, and `squid_tui::glyphs` for the half that walks a frame.
fn change() -> &'static Exchange {
    static CHANGE: OnceLock<Exchange> = OnceLock::new();
    CHANGE.get_or_init(|| Exchange::new(squid_platform::console::prices_unasked()))
}

/// One line a person is shown, in the characters their console can pay for.
///
/// Nobody is stopped from seeing what a report actually says. Point the output
/// at a file or a pipe and the text comes back as it was, because a file is not
/// a console: whatever glass reads it tomorrow may well be able to afford the
/// `█`, and an archive of `#` is a worse record for the sake of a screen that
/// was never looking. Only a screen is offered the exchange.
fn priced(text: &str) -> String {
    if !human_stream_is_tty() {
        return text.to_string();
    }
    change().afford(text)
}

/// Print one row of a report.
///
/// [`diag`] and [`say`] own the lines that carry a colour code, obey `--quiet`
/// or dodge `--json`; what is left is the body of the tables, which is printed
/// straight out because a report's rows belong to the report whoever asked for
/// it. That leftover is also where every gauge, bullet and rule in this
/// program's output is drawn, which is why there is one macro for it rather
/// than thirteen `println!`s of which twelve remember the console and one does
/// not. `no_line_of_chrome_escapes_the_counter` is what keeps it that way.
macro_rules! row {
    ($($arg:tt)*) => {
        println!("{}", $crate::priced(&format!($($arg)*)))
    };
}

/// One human-readable diagnostic line.
fn diag(code: &str, text: &str) {
    if DIAG_STDERR.load(Ordering::Relaxed) {
        eprintln!("{}", priced(&paint(code, text)));
    } else {
        println!("{}", priced(&paint(code, text)));
    }
}

/// One already-formatted human line. The plans and tables are built from
/// several `paint` fragments, so they cannot go through `diag`, but they obey
/// the same contract: `--quiet` hides them and `--json` reroutes them. A plan
/// table that leaks onto stdout is enough to break `squid purge --json --yes
/// | jq`, and the real machine is what caught it.
fn say(text: &str) {
    if QUIET.load(Ordering::Relaxed) {
        return;
    }
    if DIAG_STDERR.load(Ordering::Relaxed) {
        if !text.is_empty() {
            eprintln!("{}", priced(text));
        }
    } else {
        println!("{}", priced(text));
    }
}

/// The stream a human line lands on: stdout while it is still free, stderr
/// once `--json` has claimed it for JSON alone.
///
/// Two questions get asked of it, and they have the same answer. A progress
/// line may only overwrite a line when something can watch it being
/// overwritten. Chrome may only be exchanged for ASCII when somebody is looking
/// at the console that cannot afford it — because that is the only place the
/// exchange is an improvement rather than a loss.
fn human_stream_is_tty() -> bool {
    if DIAG_STDERR.load(Ordering::Relaxed) {
        std::io::stderr().is_tty()
    } else {
        std::io::stdout().is_tty()
    }
}

/// Rewrite the live progress line, or clear it when `text` is `None`.
fn live(text: Option<&str>) {
    let line = match text {
        Some(text) => format!("\r\x1b[K{}", priced(text)),
        None => "\r\x1b[K".to_string(),
    };
    if DIAG_STDERR.load(Ordering::Relaxed) {
        write_live(&mut std::io::stderr().lock(), &line);
    } else {
        write_live(&mut std::io::stdout().lock(), &line);
    }
}

fn write_live(sink: &mut dyn Write, line: &str) {
    let _ = sink.write_all(line.as_bytes());
    let _ = sink.flush();
}

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(error) => {
            if let Some(inner) = error.downcast_ref::<squid_core::SquidError>() {
                if matches!(inner, squid_core::SquidError::Cancelled) {
                    eprintln!(
                        "{}",
                        priced(&paint("90", "cancelled — nothing was changed"))
                    );
                    std::process::exit(130);
                }
            }
            eprintln!(
                "{}",
                priced(&format!("{} {}", paint("31", "squid:"), error))
            );
            let mut source = error.source();
            while let Some(cause) = source.take() {
                eprintln!("{}", priced(&paint("90", &format!("  caused by: {cause}"))));
                source = cause.source();
            }
            std::process::exit(1);
        }
    };
    std::process::exit(code);
}

fn run() -> Result<i32> {
    let cli = Cli::parse();
    COLOR.store(
        !cli.no_color && std::io::stdout().is_tty(),
        Ordering::Relaxed,
    );
    QUIET.store(cli.quiet, Ordering::Relaxed);
    DIAG_STDERR.store(cli.json, Ordering::Relaxed);
    // `--lang` is a flag on the whole program, so honour it for the whole
    // program: publish it as the one hint every reader already agrees on
    // before any command asks what language it should speak. Setting it only
    // inside the interface branch made `squid --lang en doctor` answer with
    // the language the *terminal* asked for, which reads like a bug.
    if let Some(lang) = &cli.lang {
        std::env::set_var("SQUID_LANG", lang);
    }

    match cli.command {
        None | Some(Command::State) => {
            if cli.no_color {
                // The interface probes for colour when it opens, and
                // https://no-color.org is the one promise every program agrees
                // on — so `--no-color` is kept by saying it out loud rather
                // than by teaching the TUI a second switch of its own.
                std::env::set_var("NO_COLOR", "1");
            }
            squid_tui::run().context("could not open the interface")?;
            Ok(0)
        }
        Some(Command::Clean(args)) => cmd_clean(args, cli.json),
        Some(Command::Uninstall(args)) => cmd_uninstall(args, cli.json),
        Some(Command::Purge(args)) => cmd_purge(args, cli.json),
        Some(Command::Status(args)) => cmd_status(args, cli.json),
        Some(Command::Analyze(args)) => cmd_analyze(args, cli.json),
        Some(Command::History(args)) => cmd_history(args, cli.json),
        Some(Command::Undo(args)) => cmd_undo(args, cli.json),
        Some(Command::Whitelist(args)) => cmd_whitelist(args),
        Some(Command::Rules) => cmd_rules(cli.json),
        Some(Command::Doctor) => cmd_doctor(),
    }
}

// ---------------------------------------------------------------------- output

fn paint(code: &str, text: &str) -> String {
    if COLOR.load(Ordering::Relaxed) {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn shout(code: &str, text: &str) {
    if !QUIET.load(Ordering::Relaxed) || code == "31" {
        diag(code, text);
    }
}

fn heading(text: &str) {
    if QUIET.load(Ordering::Relaxed) {
        return;
    }
    diag("0", "");
    diag("1;36", text);
    diag("90", &"─".repeat(text.chars().count().min(64)));
}

fn shrink(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    if text.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>() <= width {
        return text.to_string();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let mut budget = width - 3;
    let mut out = String::new();
    for ch in text.chars().rev() {
        let wide = ch.width().unwrap_or(0);
        if wide > budget {
            break;
        }
        budget -= wide;
        out.insert(0, ch);
    }
    format!("...{out}")
}

fn clip(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    if text.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>() <= width {
        return text.to_string();
    }
    let mut budget = width.saturating_sub(1);
    let mut out = String::new();
    for ch in text.chars() {
        let wide = ch.width().unwrap_or(0);
        if wide > budget {
            break;
        }
        budget -= wide;
        out.push(ch);
    }
    out.push('…');
    out
}

/// [`clip`] plus space padding so the clipped text always occupies exactly
/// `width` display columns. Columns after it stay aligned.
fn fit(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    let clipped = clip(text, width);
    let used: usize = clipped.chars().map(|c| c.width().unwrap_or(0)).sum();
    let mut out = clipped;
    out.push_str(&" ".repeat(width.saturating_sub(used)));
    out
}

/// What this operating system calls the place deleted files wait in.
///
/// Windows has no Trash folder: the shell's Recycle Bin is what a user can
/// actually open, so every sentence that tells a person where a file went, or
/// how to get it back, has to use their operating system's word.
fn bin() -> &'static str {
    squid_platform::current().bin_noun().en()
}

/// Visible width of a string, counting CJK glyphs as two columns.
fn display_width(text: &str) -> usize {
    use unicode_width::UnicodeWidthChar;
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Why this row is greyed out (or what is worth knowing about it). The item's
/// own note names the concrete reason — "no git repo and only 2 days old" —
/// which beats the generic lock label, so the note wins when both exist.
fn why(item: &CleanupItem) -> Option<String> {
    if let Some(note) = item.note.as_deref() {
        return Some(note.to_string());
    }
    item.lock.map(delete::lock_reason).map(str::to_string)
}

fn row(size: Bytes, label: &str, path: &str, note: Option<&str>) {
    let terminal = terminal_width();
    let label_cell = fit(label, 30);
    let path_width = terminal
        .saturating_sub(11 + display_width(&label_cell) + 2)
        .max(16);
    say(&format!(
        "  {}  {}  {}",
        paint("33", &format!("{:>9}", size.human())),
        paint("1", &label_cell),
        paint("90", &shrink(path, path_width))
    ));
    if let Some(note) = note {
        say(&format!(
            "             {}",
            paint("90", &format!("↳ {note}"))
        ));
    }
}

fn terminal_width() -> usize {
    crossterm::terminal::size()
        .map(|(width, _)| width as usize)
        .unwrap_or(100)
        .clamp(60, 200)
}

fn ask(question: &str) -> bool {
    if !std::io::stdin().is_tty() {
        shout(
            "33",
            "not a terminal — pass --yes to run this without asking",
        );
        return false;
    }
    print!("{} ", priced(&paint("1;33", &format!("{question} [y/N]"))));
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

// ----------------------------------------------------------- progress plumbing

/// Run `work` while pumping its progress events to the terminal.
fn with_progress<T>(
    label: &str,
    work: impl FnOnce(&Engine, &Progress, &CancelToken) -> squid_core::Result<T>,
) -> squid_core::Result<T> {
    let label = label.to_owned();
    let engine = Engine::new();
    let progress = Progress::new();
    let cancel = CancelToken::new();
    let printer = progress.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let printer_stop = stop.clone();
    let printer_thread = std::thread::spawn(move || {
        let mut frames = label.chars();
        loop {
            let stop_now = printer_stop.load(Ordering::Relaxed);
            for event in printer.drain() {
                match event {
                    Event::Phase { phase, detail } => {
                        shout("36", &format!("  {} {phase} — {detail}", bullet()));
                    }
                    Event::Tick {
                        done,
                        total,
                        subject,
                    } => {
                        if human_stream_is_tty() {
                            let glyph = frames
                                .next()
                                .unwrap_or_else(|| label.chars().next().unwrap_or('#'));
                            let text = match total {
                                Some(total) if total > 0 => format!(
                                    "  {glyph} {done}/{}",
                                    squid_core::bytes::format_thousands(total)
                                ),
                                _ => format!(
                                    "  {glyph} {}",
                                    squid_core::bytes::format_thousands(done)
                                ),
                            };
                            let tail = subject
                                .as_ref()
                                .map(|path| shrink(&paths::pretty(path), 40))
                                .unwrap_or_default();
                            live(Some(&format!(
                                "{} {}",
                                paint("36", &text),
                                paint("90", &tail)
                            )));
                        }
                    }
                    Event::Warning(text) => shout("33", &format!("  ! {}", clip(&text, 100))),
                    Event::Note(text) => shout("90", &format!("  · {}", clip(&text, 100))),
                    Event::Deleted { path, freed } => {
                        shout(
                            "90",
                            &format!(
                                "  ✓ {} ({})",
                                shrink(&paths::pretty(&path), 70),
                                freed.human()
                            ),
                        );
                    }
                    Event::Bucket { .. } | Event::Done(_) => {}
                }
            }
            if stop_now {
                if human_stream_is_tty() {
                    live(None);
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    });
    let result = work(&engine, &progress, &cancel);
    stop.store(true, Ordering::Relaxed);
    let _ = printer_thread.join();
    result
}

fn bullet() -> String {
    if COLOR.load(Ordering::Relaxed) {
        "●".to_string()
    } else {
        "*".to_string()
    }
}

/// Print a scan the CleanMyMac way: grouped, biggest first, totals last.
fn show_scan(report: &ScanReport, title: &str) {
    heading(title);
    if report.items.is_empty() {
        shout("90", "  nothing to remove — this is a good outcome");
        return;
    }
    let mut categories: Vec<Category> = Category::ALL.to_vec();
    categories.retain(|category| report.items.iter().any(|item| item.category == *category));
    categories.sort_by(|a, b| {
        let total_of = |category: &Category| -> u64 {
            report
                .items
                .iter()
                .filter(|item| item.category == *category)
                .map(|item| item.on_disk().as_u64())
                .sum()
        };
        total_of(b).cmp(&total_of(a))
    });
    for category in categories {
        let items: Vec<&CleanupItem> = report
            .items
            .iter()
            .filter(|item| item.category == category)
            .collect();
        let total: Bytes = items.iter().map(|item| item.on_disk()).sum();
        say(&paint(
            "1;36",
            &format!(
                "  {}  {}  ·  {}",
                TRIANGLE,
                category.title(),
                total.human_long()
            ),
        ));
        for item in items.iter().take(18) {
            row(
                item.on_disk(),
                &item.label,
                &item.display_path(),
                why(item).as_deref(),
            );
        }
        if items.len() > 18 {
            shout(
                "90",
                &format!("             … and {} more", items.len() - 18),
            );
        }
    }
    let selected: Bytes = report
        .items
        .iter()
        .filter(|item| item.selected && item.lock.is_none())
        .map(|item| item.on_disk())
        .sum();
    say("");
    shout(
        "1",
        &format!(
            "  {} items · {} selected · {}",
            report.items.len(),
            report
                .items
                .iter()
                .filter(|item| item.selected && item.lock.is_none())
                .count(),
            selected.human_long()
        ),
    );
    if report.unreadable > 0 {
        shout(
            "90",
            &format!("  {} locations could not be read", report.unreadable),
        );
    }
}

const TRIANGLE: &str = "▸";

/// The one place anything is ever deleted from.
fn delete_items(
    items: Vec<CleanupItem>,
    mode: DeleteMode,
    command: &str,
) -> squid_core::Result<delete::DeleteOutcome> {
    let removable: Vec<CleanupItem> = items
        .into_iter()
        .filter(|item| item.selected && item.lock.is_none())
        .collect();
    let job = DeleteJob::new(removable, mode, command);
    let count = job.removable().len();
    with_progress("delete", move |engine, progress, cancel| {
        if !COLOR.load(Ordering::Relaxed) {
            // keep the count alive for the summary line
            eprintln!(
                "{}",
                priced(&paint("90", &format!("  removing {count} items")))
            );
        }
        delete::execute(engine, &job, progress, cancel)
    })
}

fn report_outcome(outcome: &delete::DeleteOutcome) {
    let summary = &outcome.summary;
    heading("Done");
    let freed = summary.freed_on_disk();
    // "24.31 GB freed · moved to Trash" contradicts itself: a rename inside
    // one volume frees nothing until the Trash is emptied. Say which of the
    // two the number is.
    let (verb, where_it_went) = match summary.mode {
        Some(DeleteMode::Trash) => (
            "moved",
            format!(
                "into the {b} — still on the disk until you empty it (squid undo restores them)",
                b = bin()
            ),
        ),
        Some(DeleteMode::Permanent) => ("freed", "permanently deleted".to_string()),
        None => ("freed", "done".to_string()),
    };
    shout(
        "1;32",
        &format!(
            "  {} {verb} · {} removed · {}",
            freed.human_long(),
            summary.removed,
            where_it_went
        ),
    );
    if let Some(session) = &summary.session_id {
        shout(
            "90",
            &format!("  session {session} — squid undo {}", session),
        );
    }
    for (path, reason) in outcome.skipped.iter().take(8) {
        shout(
            "33",
            &format!("  skipped {} — {reason}", paths::pretty(path)),
        );
    }
    if summary.failed > 0 {
        for error in summary.errors.iter().take(6) {
            shout("31", &format!("  ! {error}"));
        }
    }
}

fn mode_from(trash: bool, permanent: bool, settings: &Settings) -> DeleteMode {
    if permanent {
        DeleteMode::Permanent
    } else if trash {
        DeleteMode::Trash
    } else {
        settings.delete_mode
    }
}

/// The document a run owes `--json` when it deleted nothing. Same shape as
/// every other run, so a script never has to special-case "nothing was
/// selected" and stdout is never empty on success.
fn empty_summary(mode: DeleteMode) -> CleanSummary {
    CleanSummary {
        mode: Some(mode),
        ..Default::default()
    }
}

fn mode_note(mode: DeleteMode) -> String {
    match mode {
        DeleteMode::Trash => {
            format!(
                "everything goes to the {} first — squid undo puts it back",
                bin()
            )
        }
        DeleteMode::Permanent => format!("permanent deletion: no {}, no undo", bin()),
    }
}

// ----------------------------------------------------------------------- clean

fn parse_categories(values: &[String]) -> Result<Vec<Category>> {
    let mut out = Vec::new();
    for value in values {
        for piece in value.split(',') {
            let piece = piece.trim();
            if piece.is_empty() {
                continue;
            }
            let category = Category::parse(piece)
                .ok_or_else(|| anyhow::anyhow!("unknown category '{piece}' — try `squid rules`"))?;
            out.push(category);
        }
    }
    Ok(out)
}

fn selected_of(items: &[CleanupItem]) -> (usize, Bytes) {
    let chosen: Vec<&CleanupItem> = items
        .iter()
        .filter(|item| item.selected && item.lock.is_none())
        .collect();
    let size = chosen.iter().map(|item| item.on_disk()).sum();
    (chosen.len(), size)
}

fn cmd_clean(args: CleanArgs, json: bool) -> Result<i32> {
    let settings = Engine::new().settings().clone();
    let mode = mode_from(args.trash, args.permanent, &settings);
    let categories = parse_categories(&args.categories)?;
    let options = clean::CleanOptions {
        categories,
        skip_admin_rules: args.user_only,
        min_age_days: args.min_age,
        orphans: args.orphans,
        fast: false,
    };
    let started = std::time::Instant::now();
    let report = with_progress("scan", move |engine, progress, cancel| {
        let report = clean::scan(engine, &options, progress, cancel)?;
        clean::publish_buckets(&report, progress);
        Ok(clean::sort_for_display(report))
    })?;

    let (count, size) = selected_of(&report.items);
    // `--json` without `--yes` promises a plan, never a deletion; a dry run
    // promises the same even when `--yes` came along for the ride.
    if json && (!args.yes || args.dry_run) {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(0);
    }
    if !args.dry_run {
        show_scan(&report, "Cleanup");
    } else {
        show_scan(&report, "Cleanup · dry run");
    }
    shout(
        "90",
        &format!("  scanned in {:.1}s", started.elapsed().as_secs_f64()),
    );

    if args.dry_run {
        shout("33", "  dry run — nothing was deleted");
        return Ok(0);
    }
    if count == 0 {
        if json {
            println!("{}", serde_json::to_string_pretty(&empty_summary(mode))?);
        }
        shout(
            "90",
            "  nothing is selected; use --dry-run to inspect first",
        );
        return Ok(0);
    }
    shout("90", &format!("  {}", mode_note(mode)));
    if !args.yes
        && !ask(&format!(
            "Delete {count} items ({}), {}",
            size.human_long(),
            match mode {
                DeleteMode::Trash => format!("to the {}", bin()),
                DeleteMode::Permanent => "permanently".to_string(),
            }
        ))
    {
        shout("33", "  cancelled — nothing was deleted");
        return Ok(0);
    }
    let outcome = delete_items(report.items, mode, "squid clean")?;
    if json {
        println!("{}", serde_json::to_string_pretty(&outcome.summary)?);
    } else {
        report_outcome(&outcome);
    }
    Ok(0)
}

// ------------------------------------------------------------------- uninstall

fn cmd_uninstall(args: UninstallArgs, json: bool) -> Result<i32> {
    let settings = Engine::new().settings().clone();
    let mode = mode_from(args.trash, args.permanent, &settings);

    if args.list || args.name.is_none() {
        let apps = with_progress("apps", |engine, progress, cancel| {
            uninstall::scan_apps(engine, true, progress, cancel)
        })?;
        if json {
            println!("{}", serde_json::to_string_pretty(&apps)?);
            return Ok(0);
        }
        heading(&format!("Applications · {}", apps.len()));
        let total: Bytes = apps.iter().map(|app| app.size).sum();
        for app in &apps {
            say(&format!(
                "  {}  {}  {}{} {}",
                paint("33", &format!("{:>9}", app.size.human())),
                paint("1", &fit(&app.name, 34)),
                paint("90", &fit(app.version.as_deref().unwrap_or("—"), 12)),
                if app.running {
                    paint("33", " running")
                } else {
                    "        ".to_string()
                },
                paint("90", &shrink(&paths::pretty(&app.path), 44))
            ));
        }
        say("");
        shout(
            "1",
            &format!("  {} apps · {}", apps.len(), total.human_long()),
        );
        shout("90", "  squid uninstall <name> builds a removal plan");
        return Ok(0);
    }

    let query = args.name.clone().unwrap_or_default();
    let found = with_progress("find", |engine, progress, cancel| {
        uninstall::find(engine, &query, cancel, progress)
    })?;
    if found.is_empty() {
        shout("31", &format!("no application matches '{query}'"));
        shout("90", "  try `squid uninstall --list`");
        return Ok(1);
    }
    let app = if found.len() == 1 {
        found[0].clone()
    } else {
        let exact = found
            .iter()
            .find(|app| app.name.eq_ignore_ascii_case(&query))
            .or_else(|| {
                found.iter().find(|app| {
                    app.bundle_id
                        .as_deref()
                        .map(|id| id.eq_ignore_ascii_case(&query))
                        .unwrap_or(false)
                })
            })
            .cloned();
        match exact {
            Some(app) => app,
            None => {
                heading(&format!("'{query}' matches {} applications", found.len()));
                for app in &found {
                    say(&format!(
                        "  {}  {}  {}",
                        paint("33", &format!("{:>9}", app.size.human())),
                        paint("1", &clip(&app.name, 34)),
                        paint("90", &shrink(&paths::pretty(&app.path), 50))
                    ));
                }
                shout(
                    "33",
                    "  be more specific — pass part of the name or the bundle id",
                );
                return Ok(1);
            }
        }
    };

    let plan = with_progress("plan", {
        let app = app.clone();
        move |engine, progress, cancel| uninstall::build_plan(engine, &app, progress, cancel)
    })?;

    if json && (!args.yes || args.dry_run) {
        println!("{}", serde_json::to_string_pretty(&plan)?);
        return Ok(0);
    }

    heading(&format!("Uninstall · {}", app.name));
    shout("90", &format!("  {}", paths::pretty(&app.path)));
    shout(
        "90",
        &format!(
            "  {} · {}",
            app.bundle_id
                .clone()
                .unwrap_or_else(|| "no bundle id".into()),
            app.version.clone().unwrap_or_else(|| "version ?".into())
        ),
    );
    for warning in &plan.warnings {
        shout("33", &format!("  ! {warning}"));
    }
    let (count, size) = selected_of(&plan.items);
    for item in &plan.items {
        row(
            item.on_disk(),
            if item.rule_id == "uninstall-bundle" {
                "app bundle"
            } else {
                item.category.title()
            },
            &item.display_path(),
            why(item).as_deref(),
        );
    }
    say("");
    shout("1", &format!("  {count} items · {}", size.human_long()));
    if args.dry_run {
        shout("33", "  dry run — nothing was deleted");
        return Ok(0);
    }
    shout("90", &format!("  {}", mode_note(mode)));
    if !args.yes
        && !ask(&format!(
            "Remove {} and {} leftover items ({})?",
            app.name,
            count,
            size.human_long()
        ))
    {
        shout("33", "  cancelled — nothing was deleted");
        return Ok(0);
    }
    let outcome = with_progress("delete", move |engine, progress, cancel| {
        uninstall::remove_plan(engine, &plan, mode, progress, cancel)
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&outcome.summary)?);
    } else {
        report_outcome(&outcome);
    }
    Ok(0)
}

// ----------------------------------------------------------------------- purge

fn cmd_purge(args: PurgeArgs, json: bool) -> Result<i32> {
    let settings = Engine::new().settings().clone();
    let mode = mode_from(false, false, &settings);
    let roots: Vec<PathBuf> = args.paths.iter().map(|path| paths::expand(path)).collect();
    let options = purge::PurgeOptions {
        roots,
        min_age_days: args.min_age,
        force: args.all,
        max_depth: None,
    };
    if json && (!args.yes || args.dry_run) {
        let report = with_progress("purge", {
            let options = options.clone();
            move |engine, progress, cancel| purge::scan(engine, &options, progress, cancel)
        })?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(0);
    }
    let report = with_progress("purge", move |engine, progress, cancel| {
        let mut report = purge::scan(engine, &options, progress, cancel)?;
        report.items.sort_by(|a, b| {
            b.on_disk()
                .cmp(&a.on_disk())
                .then_with(|| a.path.cmp(&b.path))
        });
        Ok(report)
    })?;
    show_scan(
        &report,
        if args.dry_run {
            "Deep clean · dry run"
        } else {
            "Deep clean"
        },
    );
    let snapshots = purge::local_snapshots();
    if let Some(note) = purge::snapshot_note(&snapshots) {
        heading("Time Machine");
        shout("33", &format!("  {note}"));
        for snapshot in snapshots.iter().take(6) {
            shout("90", &format!("  · {}", snapshot.name));
        }
        shout(
            "90",
            "  squid never touches snapshots — that is tmutil's job",
        );
    }
    let (count, size) = selected_of(&report.items);
    if args.dry_run {
        shout("33", "  dry run — nothing was deleted");
        return Ok(0);
    }
    if count == 0 {
        if json {
            println!("{}", serde_json::to_string_pretty(&empty_summary(mode))?);
        }
        shout(
            "90",
            "  nothing safe to remove; --all offers the ambiguous folders too",
        );
        return Ok(0);
    }
    shout("90", &format!("  {}", mode_note(mode)));
    if !args.yes
        && !ask(&format!(
            "Delete {count} folders ({}), {}",
            size.human_long(),
            match mode {
                DeleteMode::Trash => format!("to the {}", bin()),
                DeleteMode::Permanent => "permanently".to_string(),
            }
        ))
    {
        shout("33", "  cancelled — nothing was deleted");
        return Ok(0);
    }
    let outcome = delete_items(report.items, mode, "squid purge")?;
    if json {
        println!("{}", serde_json::to_string_pretty(&outcome.summary)?);
    } else {
        report_outcome(&outcome);
    }
    Ok(0)
}

// ---------------------------------------------------------------------- status

fn cmd_status(args: StatusArgs, json: bool) -> Result<i32> {
    let mut sampler = squid_stats::Sampler::new();
    let _ = sampler.prime();
    std::thread::sleep(Duration::from_millis(500));
    let snapshot = sampler.refresh();
    if json {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(0);
    }
    heading(&format!(
        "{} · {}",
        snapshot.host.hostname, snapshot.host.hardware
    ));
    let kv = |label: &str, value: String| {
        row!("  {:<10} {}", paint("90", label), value);
    };
    kv(
        "system",
        format!(
            "{} {} · {}",
            snapshot.host.os_name, snapshot.host.os_version, snapshot.host.arch
        ),
    );
    kv("uptime", squid_stats::human_uptime(snapshot.uptime_secs));
    kv(
        "cpu",
        format!(
            "{}%",
            paint(
                if snapshot.cpu.total > 80.0 {
                    "31"
                } else {
                    "32"
                },
                &format!("{:.0}", snapshot.cpu.total)
            )
        ),
    );
    kv(
        "load",
        format!(
            "{:.2} {:.2} {:.2} · {} cores · {}",
            snapshot.cpu.load[0],
            snapshot.cpu.load[1],
            snapshot.cpu.load[2],
            snapshot.cpu.logical_cores,
            clip(&snapshot.cpu.brand, 28)
        ),
    );
    kv(
        "memory",
        format!(
            "{} / {} ({}%) · swap {} / {}",
            snapshot.memory.used.human(),
            snapshot.memory.total.human(),
            snapshot.memory.percent_used(),
            snapshot.memory.swap_used.human(),
            snapshot.memory.swap_total.human()
        ),
    );
    kv(
        "network",
        format!(
            "↓ {}/s ↑ {}/s · since boot {}",
            snapshot.network.rx_per_sec.human(),
            snapshot.network.tx_per_sec.human(),
            (snapshot.network.rx_total + snapshot.network.tx_total).human()
        ),
    );
    kv(
        "disk",
        format!(
            "↓ {}/s ↑ {}/s · {:.1}% active",
            snapshot.disk.read_per_sec.human(),
            snapshot.disk.write_per_sec.human(),
            snapshot.disk.activity
        ),
    );
    if let Some(battery) = &snapshot.battery {
        kv("battery", battery.label());
    }
    heading("Volumes");
    for volume in &snapshot.volumes {
        let used = volume.used();
        let width = 28usize;
        let ratio = if volume.total.as_u64() == 0 {
            0.0
        } else {
            used.as_u64() as f64 / volume.total.as_u64() as f64
        };
        let filled = (ratio * width as f64).round() as usize;
        let bar = format!(
            "{}{}",
            paint("36", &"█".repeat(filled)),
            paint("90", &"░".repeat(width - filled))
        );
        row!(
            "  {} [{}] {} / {}  {}",
            paint("1", &fit(&volume.name, 14)),
            bar,
            paint("32", &used.human()),
            volume.total.human(),
            paint("90", &format!("{} free", volume.free.human()))
        );
    }
    heading(&format!("Top processes · {}", args.processes));
    for process in snapshot.top_processes.iter().take(args.processes) {
        row!(
            "  {:>6}  {}  {:>9}  {}",
            paint("36", &format!("{:.1}%", process.cpu)),
            paint("1", &fit(&process.name, 30)),
            paint("33", &process.memory.human()),
            paint("90", &format!("pid {}", process.pid))
        );
    }
    Ok(0)
}

// --------------------------------------------------------------------- analyze

fn cmd_analyze(args: AnalyzeArgs, json: bool) -> Result<i32> {
    let root = args
        .path
        .as_deref()
        .map(paths::expand)
        .unwrap_or_else(paths::home_dir);
    if !root.is_dir() {
        anyhow::bail!("{} is not a folder", root.display());
    }
    let node = with_progress("map", {
        let root = root.clone();
        let depth = args.depth;
        let top = args.top;
        move |engine, progress, cancel| {
            let _ = engine;
            analyze::explore(&root, depth, top, cancel, progress)
        }
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&node)?);
        return Ok(0);
    }
    heading(&format!(
        "{} · {}",
        paths::pretty(&node.path),
        node.size().human_long()
    ));
    print_tree(&node, 0, &node.size());
    if !node.complete {
        shout(
            "90",
            "  (the walk hit its limit — the totals are a floor, not the truth)",
        );
    }
    if args.files {
        let files = with_progress("files", {
            let root = root.clone();
            move |engine, progress, cancel| {
                let _ = engine;
                analyze::largest_files(&root, 20, Bytes(10 * 1024 * 1024), cancel, progress)
            }
        })?;
        heading("Largest files");
        for file in &files {
            row(
                file.size(),
                &paths::label_for(&file.path),
                &paths::pretty(&file.path),
                None,
            );
        }
    }
    Ok(0)
}

fn print_tree(node: &analyze::Node, depth: usize, total: &Bytes) {
    let width = 22usize;
    let bar_w = (terminal_width() as i64 - 34 - depth as i64 * 2).clamp(6, 30) as usize;
    for child in node.display_children().iter().take(40) {
        let size = child.size();
        let ratio = if total.is_zero() {
            0.0
        } else {
            size.as_u64() as f64 / total.as_u64() as f64
        };
        let filled = (ratio * bar_w as f64).round() as usize;
        let bar = format!(
            "{}{}",
            paint("35", &"█".repeat(filled)),
            paint("90", &"░".repeat(bar_w.saturating_sub(filled)))
        );
        row!(
            "  {}{} [{}] {} {:>4.1}%",
            "  ".repeat(depth),
            paint("1", &fit(&child.name(), width)),
            bar,
            paint("33", &format!("{:>8}", size.human())),
            ratio * 100.0
        );
        if depth + 1 < 2 {
            print_tree(child, depth + 1, total);
        }
    }
}

// ------------------------------------------------------------- history / undo

/// What a run did to its bytes, in one word. A run the user restored gets its
/// own label: still calling it "to Trash" after an undo is how the ledger
/// starts telling a story that no longer happened.
fn mode_label(session: &squid_core::ledger::Session) -> String {
    if session.restored {
        "restored".to_string()
    } else {
        match session.mode {
            DeleteMode::Trash => format!("to {}", bin()),
            DeleteMode::Permanent => "for good".to_string(),
        }
    }
}

/// Width of the mode column in `history`: the operating system's own word has
/// to fit there, and "to Recycle Bin" is wider than "to Trash".
fn mode_col_width() -> usize {
    format!("to {}", bin()).len().max(9)
}

fn cmd_history(args: HistoryArgs, json: bool) -> Result<i32> {
    let sessions = squid_core::ledger::load_sessions(args.limit);
    if json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(0);
    }
    heading(&format!("History · {} sessions", sessions.len()));
    if sessions.is_empty() {
        shout(
            "90",
            &format!(
                "  nothing yet — the ledger lives in {}",
                paths::pretty(&squid_core::ledger::history_dir())
            ),
        );
        return Ok(0);
    }
    let mut freed = Bytes::ZERO;
    let mut trashed = Bytes::ZERO;
    let mut undone = 0;
    for session in &sessions {
        // `kept`, not `freed`: a session the user already restored put every
        // byte back, and a total that counts it twice overstates the space.
        let reclaimed = session.kept();
        freed += reclaimed;
        if session.restored {
            undone += 1;
        }
        if session.mode == DeleteMode::Trash {
            // A rename into ~/.Trash is on the same volume, so the bytes are
            // still on the disk. Counting them as "freed" is how a correct
            // number comes to look like a broken one.
            trashed += reclaimed;
        }
        let failures = if session.failed > 0 {
            paint("33", &format!(" · {} failed", session.failed))
        } else {
            String::new()
        };
        row!(
            "  {}  {}  {}  {}  {}  {}",
            paint(
                "90",
                &fit(
                    session
                        .started
                        .replace('T', " ")
                        .split('+')
                        .next()
                        .and_then(|stamp| stamp.split('.').next())
                        .unwrap_or(&session.started),
                    19
                )
            ),
            paint("1", &fit(&session.command, 20)),
            paint("32", &format!("{:>11}", session.freed().human_fixed())),
            paint(
                "33",
                &format!(
                    "{:>14} removed",
                    squid_core::bytes::format_thousands(session.removed as u64)
                ),
            ),
            paint(
                "90",
                &format!(
                    "{}{}",
                    fit(&mode_label(session), mode_col_width()),
                    failures
                )
            ),
            paint("90", &shrink(&session.id, 18))
        );
    }
    println!();
    shout(
        "1",
        &format!(
            "  {} removed across {} sessions",
            freed.human_long(),
            sessions.len()
        ),
    );
    if !trashed.is_zero() {
        shout(
            "90",
            &format!(
                "  {} of it is sitting in the {b} — empty the {b} to actually reclaim it",
                trashed.human_long(),
                b = bin()
            ),
        );
    }
    if undone > 0 {
        shout(
            "90",
            &format!("  {undone} session(s) were restored afterwards and are counted as nothing"),
        );
    }
    shout(
        "90",
        &format!("  {}", paths::pretty(&squid_core::ledger::history_dir())),
    );
    shout(
        "90",
        &format!(
            "  squid undo restores the newest session that went to the {}",
            bin()
        ),
    );
    Ok(0)
}

fn cmd_undo(args: UndoArgs, json: bool) -> Result<i32> {
    let id = match args.session {
        Some(id) => id,
        None => {
            let sessions = squid_core::ledger::load_sessions(40);
            match sessions
                .iter()
                .find(|session| session.mode == DeleteMode::Trash && !session.entries.is_empty())
            {
                Some(session) => session.id.clone(),
                None => {
                    shout("33", &format!("no deletions in the {} to restore", bin()));
                    return Ok(0);
                }
            }
        }
    };
    // Whether squid kept a path it can rename back is a property of the
    // session's mode, so read it from the ledger rather than guessing.
    let trashed = squid_core::ledger::load_session(&id)
        .map(|session| session.mode == DeleteMode::Trash)
        .unwrap_or(false);
    let (restored, skipped) =
        delete::restore_session(&id).with_context(|| format!("could not restore session {id}"))?;
    if json {
        println!(
            "{}",
            serde_json::json!({ "session": id, "restored": restored, "skipped": skipped })
        );
    } else {
        shout(
            "1;32",
            &format!("{restored} items restored from session {id}"),
        );
        if skipped > 0 {
            // Trash mode on Windows moves the bytes into the shell's Recycle
            // Bin, which keeps them under its own names — squid never has a
            // path to rename back. Saying "path in use" there blamed the user
            // for a restore that was never squid's to perform.
            let shell_owns_it = trashed
                && matches!(
                    Engine::new().platform().trash_target(),
                    TrashTarget::RecycleBin
                );
            if shell_owns_it {
                shout(
                    "90",
                    &format!("{skipped} are in the Recycle Bin — restore them there"),
                );
            } else {
                shout(
                    "90",
                    &format!("{skipped} could not be put back (path in use)"),
                );
            }
        }
    }
    Ok(0)
}

// ------------------------------------------------------ whitelist / rules / doctor

fn cmd_whitelist(args: WhitelistArgs) -> Result<i32> {
    let (mut settings, errors) = Settings::load();
    for error in errors {
        shout("33", &format!("config: {error}"));
    }
    match args.action {
        WhitelistAction::Add { paths: values } => {
            if values.is_empty() {
                anyhow::bail!("nothing to protect — pass one or more paths or globs");
            }
            for value in values {
                let pattern = value.replace('~', &paths::home_dir().to_string_lossy());
                shout("32", &format!("protected {pattern}"));
                settings.add_whitelist(pattern);
            }
        }
        WhitelistAction::Remove { paths: values } => {
            for value in values {
                let pattern = value.replace('~', &paths::home_dir().to_string_lossy());
                if settings.remove_whitelist(&pattern) {
                    shout("33", &format!("unprotected {pattern}"));
                } else {
                    shout("90", &format!("{pattern} was not protected"));
                }
            }
        }
        WhitelistAction::List => {
            heading("Whitelist");
            if settings.whitelist.is_empty() {
                shout("90", "  empty — every matching path is offered for review");
            }
            for pattern in &settings.whitelist {
                row!("  {} {}", paint("32", "●"), pattern);
            }
            println!();
            shout("90", &format!("  {}", paths::pretty(&Settings::path())));
            return Ok(0);
        }
    }
    let path = settings.save()?;
    shout("90", &format!("saved {}", paths::pretty(&path)));
    Ok(0)
}

fn cmd_rules(json: bool) -> Result<i32> {
    let engine = Engine::new();
    let rules = squid_rules::RuleSet::load(engine.settings());
    if json {
        let view: Vec<serde_json::Value> = rules
            .rules()
            .iter()
            .map(|rule| {
                serde_json::json!({
                    "id": rule.id,
                    "category": rule.category.id(),
                    "title": rule.title,
                    "os": rule.os,
                    "needs_admin": rule.needs_admin,
                    "risky": rule.risky,
                    "note": rule.note,
                    "min_age_days": rule.min_age_days,
                    "select_by_default": rule.select_by_default,
                    "enabled": engine.settings().rule_enabled(&rule.id),
                    "patterns": rule.patterns,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&view)?);
        return Ok(0);
    }
    for category in rules.categories() {
        let list = rules.by_category(category);
        println!();
        row!(
            "{}",
            paint(
                "1;36",
                &format!("  {} · {}", category.title(), category.blurb())
            )
        );
        for rule in list {
            let enabled = engine.settings().rule_enabled(&rule.id);
            let mut flags = Vec::new();
            if !enabled {
                flags.push(paint("31", "off"));
            }
            if rule.needs_admin && !squid_engine::is_root() {
                flags.push(paint("33", "admin"));
            }
            if rule.risky {
                flags.push(paint("31", "risky"));
            }
            if rule.min_age_days > 0 {
                flags.push(paint("90", &format!(">{}d", rule.min_age_days)));
            }
            if !rule.select_by_default {
                flags.push(paint("90", "opt-in"));
            }
            row!(
                "  {}  {:<28} {}",
                if enabled {
                    paint("32", "●")
                } else {
                    paint("31", "○")
                },
                rule.id,
                paint("90", rule.display_title())
            );
            if !flags.is_empty() {
                row!("                    {}", flags.join("  "));
            }
        }
    }
    println!();
    shout(
        "90",
        &format!(
            "  {} rules · disable one with `squid rules` docs or settings.toml disabled_rules",
            rules.len()
        ),
    );
    for error in rules.errors() {
        shout("31", &format!("  ! {error}"));
    }
    Ok(0)
}

fn cmd_doctor() -> Result<i32> {
    let engine = Engine::new();
    let platform = engine.platform();
    heading("squid doctor");
    let kv = |label: &str, value: String| row!("  {:<12} {}", paint("90", label), value);
    kv("version", squid_core::VERSION.to_string());
    kv("platform", platform.display_name());
    kv("os", platform.os_version());
    kv("hardware", platform.hardware_model());
    kv(
        "privileges",
        if squid_engine::is_root() {
            paint("33", "root — every path is offered, be careful")
        } else {
            format!(
                "{} (user-owned paths only)",
                std::env::var("USER").unwrap_or_else(|_| "standard".into())
            )
        },
    );
    kv(
        "config",
        format!(
            "{} {}",
            paths::pretty(&Settings::path()),
            if Settings::path().exists() {
                "(present)"
            } else {
                "(defaults — created on first save)"
            }
        ),
    );
    let (locale, source) = Locale::explain(engine.settings());
    kv(
        "language",
        format!("{} — decided by {}", locale.code(), source.describe()),
    );
    // The other half of "it looks wrong in this terminal": squid only paints
    // 24-bit colour where the host said it can parse it, so `TERM`/`COLORTERM`
    // are worth printing next to the answer they produced.
    let mode = squid_tui::theme::color_mode();
    kv(
        "colour",
        format!(
            "{} — probed from TERM={} COLORTERM={} TERM_PROGRAM={}",
            mode.label(),
            std::env::var("TERM").unwrap_or_else(|_| "(unset)".into()),
            std::env::var("COLORTERM").unwrap_or_else(|_| "(unset)".into()),
            std::env::var("TERM_PROGRAM").unwrap_or_else(|_| "(unset)".into()),
        ),
    );
    for error in engine.settings_errors() {
        shout("31", &format!("    ! {error}"));
    }
    let probe = Settings::config_dir().join(".squid-write-probe");
    let writable = std::fs::create_dir_all(Settings::config_dir()).is_ok()
        && std::fs::write(&probe, b"ok").is_ok();
    let _ = std::fs::remove_file(&probe);
    kv(
        "writable",
        if writable {
            paint("32", "yes — settings and ledger can be written")
        } else {
            paint("31", "no — check ownership of ~/.config/squid")
        },
    );
    let rules = squid_rules::RuleSet::load(engine.settings());
    kv("rules", format!("{} loaded", rules.len()));
    kv(
        "delete mode",
        match engine.settings().delete_mode {
            DeleteMode::Trash => format!("{} (undo available)", bin()),
            DeleteMode::Permanent => "permanent".to_string(),
        },
    );
    let sessions = squid_core::ledger::load_sessions(500);
    let freed: Bytes = sessions.iter().map(|session| session.freed()).sum();
    kv(
        "ledger",
        format!(
            "{} sessions · {} · {}",
            sessions.len(),
            freed.human_long(),
            paths::pretty(&squid_core::ledger::history_dir())
        ),
    );
    // Ask the platform, not the filesystem: Windows has no ~/.Trash yet its
    // Recycle Bin works, and claiming otherwise made people think Trash mode
    // was silently permanent.
    let trash = match platform.trash_target() {
        TrashTarget::RecycleBin => paint("32", "Recycle Bin — restorable from Windows itself"),
        TrashTarget::Folder(dir) => paint("32", &format!("reachable ({})", paths::pretty(&dir))),
        TrashTarget::Unreachable => paint(
            "33",
            &format!("no {} here — deletions fall back to permanent", bin()),
        ),
    };
    kv("trash", trash);
    let volumes = platform.volumes().unwrap_or_default();
    kv("volumes", format!("{}", volumes.len()));
    for volume in &volumes {
        row!(
            "    {} {}",
            paint("1", &fit(&volume.name, 16)),
            paint(
                "90",
                &format!("{} free of {}", volume.free.human(), volume.total.human())
            )
        );
    }
    let mut probes = vec![
        paths::home_dir().join("Library/Caches"),
        Path::new("/Applications").to_path_buf(),
        paths::home_dir().join("Library/Safari"),
        paths::home_dir().join("Library/Mail"),
    ];
    if let Some(volume) = volumes.iter().find(|volume| volume.is_root) {
        probes.push(volume.mount_point.join("Library/Application Support"));
    }
    let blocked: Vec<String> = probes
        .into_iter()
        .filter(|path| path.exists() && std::fs::read_dir(path).is_err())
        .map(|path| paths::pretty(&path))
        .collect();
    if blocked.is_empty() {
        kv("access", paint("32", "every probe folder is readable"));
    } else {
        kv(
            "access",
            paint(
                "33",
                "some folders are blocked — grant Full Disk Access to your terminal",
            ),
        );
        for path in blocked {
            shout("90", &format!("    {path}"));
        }
    }
    println!();
    shout(
        "90",
        &format!("  safety: {}", mode_note(engine.settings().delete_mode)),
    );
    Ok(0)
}

#[cfg(test)]
mod chrome {
    use squid_platform::console::Prices;
    use squid_platform::currency::{self, Exchange};
    use unicode_width::UnicodeWidthStr;

    /// The source with its strings, character literals and comments blanked, so
    /// that a scan of the program is not a scan of its prose — which matters
    /// more here than in most programs, because half of this file is sentences
    /// about what it prints, and they quote the characters they describe.
    fn only_code(source: &str) -> String {
        let chars: Vec<char> = source.chars().collect();
        let mut code = String::with_capacity(source.len());
        let mut index = 0;
        while index < chars.len() {
            match chars[index] {
                '/' if chars.get(index + 1) == Some(&'/') => {
                    while index < chars.len() && chars[index] != '\n' {
                        code.push(' ');
                        index += 1;
                    }
                }
                // A double quote always opens a string. A single quote only
                // opens a character literal, and closes again almost immediately
                // when it does; otherwise it is a lifetime, and blanking from a
                // lifetime to the next quote is how a print site vanishes from
                // an audit whose entire job is to find print sites.
                quote @ ('"' | '\'') if quote == '"' || closes_char_literal(&chars, index) => {
                    code.push(' ');
                    index += 1;
                    while index < chars.len() {
                        if chars[index] == '\\' {
                            code.push(' ');
                            index += 2;
                            continue;
                        }
                        code.push(' ');
                        index += 1;
                        if chars[index - 1] == quote {
                            break;
                        }
                    }
                }
                ch => {
                    code.push(ch);
                    index += 1;
                }
            }
        }
        code
    }

    /// Whether the single quote at `open` opens a character literal.
    ///
    /// `'q'`, `'\n'` and `'\u{1F600}'` all close inside a few characters, which
    /// is what this asks; `'static`, `'a mut` and the apostrophe of a prose word
    /// that escaped the comment blanker do not, so they stay where they are.
    fn closes_char_literal(chars: &[char], open: usize) -> bool {
        let mut index = open + 1;
        while index < chars.len() && index - open <= 12 {
            if chars[index] == '\\' {
                index += 2;
                continue;
            }
            if chars[index] == '\'' {
                return true;
            }
            index += 1;
        }
        false
    }

    /// The index just past the parenthesis that closes the one at `open`.
    fn closed(code: &str, open: usize) -> Option<usize> {
        let mut depth = 0;
        for (offset, ch) in code[open..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(open + offset);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Every line a person is shown pays for its chrome, and the only lines that
    /// do not are JSON and blank ones.
    ///
    /// The rule is about the printing rather than the characters: a `█` is
    /// nothing to be ashamed of, and this program is full of them. It is
    /// `println!` that owes the console an answer, because that is the moment a
    /// measured row becomes a row of cells. So every print of this file either
    /// goes through [`priced`] — directly, or as a [`row!`], which is the same
    /// thing with the format arguments left where they were written — or prints
    /// JSON, which is data: rewriting a person's report to flatter their
    /// terminal is a different bug with the same fingerprint.
    ///
    /// The scan reads the source because the mistake it looks for cannot be seen
    /// from the machine it was made on. `println!("  {}", plan)` standing among
    /// twelve `row!` calls is invisible on every terminal that charges one cell
    /// for everything, including the one it was typed on, and arrives as a
    /// report folded over on itself in front of somebody else.
    #[test]
    fn no_line_of_chrome_escapes_the_counter() {
        let code = only_code(include_str!("main.rs"));
        let mut offenders = Vec::new();
        for call in ["println!(", "eprintln!(", "print!("] {
            let mut from = 0;
            while let Some(found) = code[from..].find(call) {
                let at = from + found;
                from = at + call.len();
                let Some(end) = closed(&code, at + call.len() - 1) else {
                    continue;
                };
                let group = code[at + call.len()..end].trim();
                if group.is_empty() || group.contains("serde_json") || group.contains("priced") {
                    continue;
                }
                offenders.push(group.to_string());
            }
        }
        assert!(
            offenders.is_empty(),
            "{} print site(s) print a line nobody priced: {offenders:?}",
            offenders.len()
        );
    }

    /// What a console that reprices things charges for a line, which is not what
    /// `format!` billed for it: the layout counts a `█` as the one column it
    /// measured, and the glass charges two.
    fn charged(text: &str) -> usize {
        text.chars()
            .map(|ch| 1 + currency::disputed(ch) as usize)
            .sum()
    }

    #[test]
    fn a_report_row_is_as_long_as_the_row_was_billed_for() {
        // The disk row from `squid status`, built the way the command builds it.
        // The row is the whole complaint: nobody called the gauge too ugly, they
        // called the table unreadable, and the table is unreadable because this
        // row is a screen and a half long on the glass it is printed on.
        let row = format!(
            "  {:<14} [{}{}] 212.4 GB / 460 GB  247.6 GB free",
            "Macintosh HD",
            "█".repeat(20),
            "░".repeat(8)
        );
        // Every character is one column to the layout, so the row is as many
        // columns as it is characters — and twenty-eight of those characters are
        // the gauge, which is what the console doubles.
        assert_eq!(row.width(), row.chars().count());
        assert_eq!(
            charged(&row),
            row.width() + 28,
            "the row has to be overcharged before the exchange is worth testing"
        );

        let paid = Exchange::new(Prices::implied(936)).afford(&row);
        assert_eq!(
            charged(&paid),
            paid.width(),
            "a console may not charge for a cell the row did not pay for"
        );
        assert!(
            paid.contains("Macintosh HD") && paid.contains("212.4 GB"),
            "the row still says what it said: {paid}"
        );
    }
}
