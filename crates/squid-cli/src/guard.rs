//! `squid guard` — the security half of the command line.
//!
//! Three verbs carry the roadmap's CLI contract: `status` says what protects
//! this machine and what squid can see, `scan` finds things, `quarantine` acts on
//! a finding. The rest (`startup`, `held`, `restore`, `unignore`) exists because
//! a person reading a finding in a terminal needs the same follow-up the desktop
//! app offers, and it is all the same code underneath.
//!
//! The one asymmetry worth stating: `quarantine` takes a finding id, not a path.
//! A finding records the hash and size the scan saw, and the quarantine refuses
//! a file that has changed since. Taking a path from the command line would throw
//! that away, so the CLI re-runs the scan to find the id again instead.

use crate::{bullet, clip, heading, human_stream_is_tty, live, paint, shout, shrink};
use anyhow::Result;
use clap::{Args, Subcommand};
use squid_core::progress::{CancelToken, Event, Progress};
use squid_core::{paths, Bytes, Settings};
use squid_guard::quarantine::Quarantine;
use squid_guard::scan::{Guard, ScanTarget};
use squid_guard::Confidence;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Args, Debug)]
pub struct GuardArgs {
    #[command(subcommand)]
    pub action: GuardAction,
}

#[derive(Subcommand, Debug)]
pub enum GuardAction {
    /// What protects this machine right now, and what squid can see
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Scan for malware. With no paths, the quick-scan range is used.
    Scan(ScanArgs),
    /// What starts when this machine logs in
    Startup(StartupArgs),
    /// Move one finding into the quarantine, by the id a scan printed
    Quarantine { id: String },
    /// Put a quarantined file back where it came from
    Restore { id: String },
    /// What squid is holding, and why
    Held {
        #[arg(long)]
        json: bool,
    },
    /// Stop ignoring a finding so future scans report it again
    Unignore { fingerprint: String },
}

#[derive(Args, Debug)]
pub struct ScanArgs {
    /// Paths to scan instead of the quick-scan range
    paths: Vec<String>,
    /// Also walk the folders a payload rarely lives in
    #[arg(long)]
    full: bool,
    /// Machine-readable output with every finding and its evidence
    #[arg(long)]
    json: bool,
    /// Quarantine confirmed findings without asking
    #[arg(long, short = 'y')]
    yes: bool,
}

#[derive(Args, Debug)]
pub struct StartupArgs {
    /// Include the entries the operating system owns
    #[arg(long)]
    all: bool,
    /// Only the entries that deserve a second look
    #[arg(long)]
    risky: bool,
    #[arg(long)]
    json: bool,
}

pub fn run(args: GuardArgs, json: bool) -> Result<i32> {
    match args.action {
        GuardAction::Status { json: as_json } => status(as_json || json),
        GuardAction::Scan(args) => scan(args, json),
        GuardAction::Startup(args) => startup(args),
        GuardAction::Quarantine { id } => quarantine(&id),
        GuardAction::Restore { id } => restore(&id),
        GuardAction::Held { json: as_json } => held(as_json || json),
        GuardAction::Unignore { fingerprint } => unignore(&fingerprint),
    }
}

/// A guard with the user's exclusions applied. Built once per command so what
/// `status` reports and what `scan` uses cannot disagree.
fn context() -> Guard {
    let (settings, errors) = Settings::load();
    for error in errors {
        shout("33", &format!("config: {error}"));
    }
    Guard::new(settings.security.exclusion_paths())
}

fn status(as_json: bool) -> Result<i32> {
    let protection = squid_platform::protection::status();
    let intel = squid_guard::intel::status();
    let startup = squid_platform::startup::inventory();
    let mut held = Quarantine::new();
    let held_items = held.entries().len();
    let held_size = held.total_size();

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "protection": protection,
                "rules": intel,
                "startup": {
                    "total": startup.items.len(),
                    "user": startup.user_count(),
                    "system": startup.system_count(),
                    "disabled": startup.disabled_count(),
                    "notes": startup.notes,
                },
                "quarantine": { "items": held_items, "size": held_size },
            }))?
        );
        return Ok(0);
    }

    heading("Protection");
    let state = match protection.is_healthy() {
        Some(true) => paint("32", "active"),
        Some(false) => paint("31", "needs attention"),
        None => paint("33", "unknown"),
    };
    let product = if protection.product_name.is_empty() {
        protection.provider.as_str()
    } else {
        protection.product_name.as_str()
    };
    println!(
        "{}",
        crate::priced(&format!(
            "  {:<12} {} ({})",
            paint("90", "system"),
            state,
            product
        ))
    );
    if !protection.definitions.is_empty() {
        println!(
            "{}",
            crate::priced(&format!(
                "  {:<12} {}{}",
                paint("90", "definitions"),
                protection.definitions,
                if protection.definitions_date.is_empty() {
                    String::new()
                } else {
                    format!(" · from {}", protection.definitions_date)
                }
            ))
        );
    }
    let flag = |value: Option<bool>| match value {
        Some(true) => paint("32", "on"),
        Some(false) => paint("31", "off"),
        None => paint("33", "unknown"),
    };
    for (label, value) in [
        ("background", protection.background_scans),
        ("at launch", protection.launch_scans),
        ("gatekeeper", protection.gatekeeper),
    ] {
        if value.is_some() {
            println!(
                "{}",
                crate::priced(&format!("  {:<12} {}", paint("90", label), flag(value)))
            );
        }
    }
    for note in &protection.notes {
        shout("33", &format!("  ! {note}"));
    }

    heading("Squid");
    let rules = if intel.source == "builtin" {
        format!(
            "{} built-in rules (no pack downloaded)",
            intel.builtin_rules
        )
    } else {
        format!(
            "{} rules from pack r{}, {} built-in",
            intel.installed_rules, intel.revision, intel.builtin_rules
        )
    };
    for (label, value) in [
        ("rules", rules),
        (
            "startup",
            format!(
                "{} user · {} system · {} disabled",
                startup.user_count(),
                startup.system_count(),
                startup.disabled_count()
            ),
        ),
        (
            "quarantine",
            format!("{} items · {}", held_items, Bytes(held_size).human_long()),
        ),
    ] {
        println!(
            "{}",
            crate::priced(&format!("  {:<12} {}", paint("90", label), value))
        );
    }
    shout(
        "90",
        "  squid reads the protection above; it does not replace it",
    );
    for note in &startup.notes {
        shout("33", &format!("  ! {note}"));
    }
    println!();
    shout("90", "  squid guard scan · squid guard startup");
    Ok(if protection.is_healthy() == Some(false) {
        2
    } else {
        0
    })
}

fn scan(args: ScanArgs, json: bool) -> Result<i32> {
    let target = if !args.paths.is_empty() {
        ScanTarget::Paths(args.paths.clone())
    } else if args.full {
        ScanTarget::Full
    } else {
        ScanTarget::Quick
    };
    let mode = target.mode();
    shout(
        "90",
        &format!("  {} scan — {}", mode.as_str(), mode.estimate()),
    );

    let mut guard = context();
    let started = std::time::Instant::now();
    let report = with_progress("scan", &mut guard, move |guard, progress, cancel| {
        guard.scan(&target, progress, cancel)
    })?;

    // Recording happens before anything is printed, so an interrupted report
    // still leaves a finding id the next command can act on.
    let mut store = squid_guard::store::Store::new();
    store.set_scan(squid_guard::store::StoredScan::record(&report));

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(if report.confirmed() > 0 { 2 } else { 0 });
    }

    heading(&format!("Security · {}", mode.as_str()));
    if report.cancelled {
        shout("33", "  cancelled — this scan did not finish");
    }
    if report.findings.is_empty() {
        if report.clean_and_complete() {
            shout("32", "  ✓ nothing found in the range this scan covers");
        } else {
            shout("33", "  nothing found in what could be read");
        }
    }
    for finding in &report.findings {
        let dot = match finding.confidence {
            Confidence::Confirmed => paint("31", "●"),
            Confidence::Suspicious => paint("33", "●"),
            Confidence::Advisory => paint("90", "○"),
        };
        shout(
            "0",
            &format!(
                "  {} {}  {}",
                dot,
                paint("1", &shrink(&finding.name, 32)),
                paint(
                    "90",
                    &shrink(finding.display_path.as_deref().unwrap_or(""), 54)
                )
            ),
        );
        shout(
            "0",
            &format!("      {} {}", paint("90", "→"), shrink(&finding.title, 88)),
        );
        for item in finding.evidence.iter().take(3) {
            shout("90", &format!("      {}", shrink(&item.summary(), 92)));
        }
        shout("36", &format!("      {}", finding.id));
    }
    for advisory in &report.system_advisories {
        shout("90", &format!("  ○ {}", shrink(&advisory.description, 96)));
    }

    if !report.coverage.complete() {
        shout("33", "  coverage was limited:");
        for gap in report.coverage.gaps() {
            shout(
                "33",
                &format!(
                    "    {} — {}",
                    gap.display_path,
                    gap.reason.as_deref().unwrap_or(gap.outcome.as_str())
                ),
            );
        }
    }
    shout(
        "90",
        &format!(
            "  {} files checked · {} reused · {} skipped · {:.1}s",
            squid_core::bytes::format_thousands(report.examined as u64),
            squid_core::bytes::format_thousands(report.cached as u64),
            squid_core::bytes::format_thousands(report.skipped as u64),
            started.elapsed().as_secs_f64()
        ),
    );

    // 2 means "this scan has something to act on", which covers a suspicious
    // finding as well as one that never finished: a script should check the
    // report, not infer safety from an exit code.
    let incomplete = report.cancelled || !report.coverage.complete();
    let (settings, _) = Settings::load();
    let confirmed: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.confidence == Confidence::Confirmed)
        .collect();
    if confirmed.is_empty() {
        let outstanding = report.confirmed() + report.suspicious();
        return Ok(if outstanding > 0 || incomplete { 2 } else { 0 });
    }
    if args.yes || settings.security.auto_quarantine {
        let mut held = Quarantine::new();
        for finding in confirmed {
            match held.quarantine(finding) {
                Ok(entry) => shout(
                    "32",
                    &format!("  quarantined {} ({})", entry.display_path, entry.id),
                ),
                Err(error) => shout(
                    "31",
                    &format!(
                        "  could not quarantine {}: {error}",
                        finding.display_path.clone().unwrap_or_default()
                    ),
                ),
            }
        }
    } else {
        shout(
            "90",
            "  nothing was moved — squid guard quarantine <id> acts on one finding",
        );
    }
    Ok(2)
}

fn startup(args: StartupArgs) -> Result<i32> {
    let inventory = squid_platform::startup::inventory();
    if args.json {
        println!("{}", serde_json::to_string_pretty(&inventory)?);
        return Ok(0);
    }
    heading("Startup items");
    let mut shown = 0_usize;
    for item in &inventory.items {
        if !args.all && item.system_vendor {
            continue;
        }
        if args.risky && item.risks.is_empty() {
            continue;
        }
        shown += 1;
        let scope = match item.scope {
            squid_platform::startup::StartupScope::User => paint("36", "user"),
            squid_platform::startup::StartupScope::System => paint("90", "sys "),
        };
        let state = if item.disabled {
            paint("33", "off")
        } else if item.modifiable {
            paint("32", "on ")
        } else {
            paint("90", "ro ")
        };
        shout(
            "0",
            &format!(
                "  {scope} {state} {} {}",
                paint("1", &shrink(&item.name, 28)),
                paint("90", &shrink(item.source.as_str(), 15))
            ),
        );
        shout("90", &format!("      {}", shrink(&item.command, 96)));
        let mut marks: Vec<String> = item
            .risks
            .iter()
            .map(|risk| format!("{risk:?}").to_lowercase().replace('_', "-"))
            .collect();
        if let Some(publisher) = &item.publisher {
            marks.push(format!("signed: {publisher}"));
        } else if !item.signed {
            marks.push("unsigned".into());
        }
        if !marks.is_empty() {
            shout("33", &format!("      {}", shrink(&marks.join(" · "), 96)));
        }
    }
    println!();
    shout(
        "90",
        &format!(
            "  {shown} of {} entries · {} belong to the operating system",
            inventory.items.len(),
            inventory
                .items
                .iter()
                .filter(|item| item.system_vendor)
                .count()
        ),
    );
    for note in &inventory.notes {
        shout("33", &format!("  ! {note}"));
    }
    Ok(0)
}

fn quarantine(id: &str) -> Result<i32> {
    let store = squid_guard::store::Store::new();
    let Some(found) = store.finding(id) else {
        shout(
            "31",
            &format!("  no finding {id} — run `squid guard scan` and use an id it printed"),
        );
        return Ok(1);
    };
    let finding = found.clone();
    let mut held = Quarantine::new();
    match held.quarantine(&finding) {
        Ok(entry) => {
            shout("32", &format!("  quarantined {}", entry.display_path));
            shout("90", &format!("  squid guard restore {}", entry.id));
            let mut store = squid_guard::store::Store::new();
            store.update_scan(|scan| {
                if let Some(tracked) = scan.findings.iter_mut().find(|f| f.id == id) {
                    tracked.quarantined = true;
                }
            });
            Ok(0)
        }
        Err(error) => {
            shout("31", &format!("  refused: {error}"));
            Ok(1)
        }
    }
}

fn restore(id: &str) -> Result<i32> {
    let mut held = Quarantine::new();
    match held.restore(id) {
        Ok(path) => {
            shout("32", &format!("  restored {}", paths::pretty(&path)));
            // The finding is live again on disk, so the next scan must be able
            // to report it; clearing the flag keeps the stored scan honest.
            let mut store = squid_guard::store::Store::new();
            store.update_scan(|scan| {
                for finding in &mut scan.findings {
                    if finding
                        .path
                        .as_ref()
                        .is_some_and(|candidate| paths::same_path(candidate, &path))
                    {
                        finding.quarantined = false;
                    }
                }
            });
            Ok(0)
        }
        Err(error) => {
            shout("31", &format!("  refused: {error}"));
            Ok(1)
        }
    }
}

fn held(as_json: bool) -> Result<i32> {
    let mut held = Quarantine::new();
    if as_json {
        println!("{}", serde_json::to_string_pretty(held.entries())?);
        return Ok(0);
    }
    heading("Quarantine");
    if held.entries().is_empty() {
        shout("90", "  empty — nothing has been moved out of the way");
        return Ok(0);
    }
    for entry in held.entries() {
        shout(
            "0",
            &format!(
                "  {} {}  {}",
                paint("33", "◆"),
                paint("1", &shrink(&entry.display_path, 58)),
                paint("90", &Bytes(entry.size).human_long())
            ),
        );
        shout(
            "90",
            &format!("      {} · {}", shrink(&entry.title, 58), entry.id),
        );
    }
    println!();
    shout(
        "90",
        &format!(
            "  {} items · {} · restore puts one back, the folder is the last copy",
            held.entries().len(),
            Bytes(held.total_size()).human_long()
        ),
    );
    shout(
        "90",
        &format!(
            "  {}",
            paths::pretty(&squid_guard::coverage::quarantine_root())
        ),
    );
    Ok(0)
}

fn unignore(fingerprint: &str) -> Result<i32> {
    let (mut settings, errors) = Settings::load();
    for error in errors {
        shout("33", &format!("config: {error}"));
    }
    if settings.security.unignore(fingerprint) {
        let path = settings.save()?;
        shout("32", &format!("  {fingerprint} will be reported again"));
        shout("90", &format!("  saved {}", paths::pretty(&path)));
    } else {
        shout("90", &format!("  {fingerprint} was not ignored"));
    }
    Ok(0)
}

/// The same progress rendering the cleanup commands use, driven by a guard scan
/// rather than an `Engine`.
fn with_progress<T>(
    label: &str,
    guard: &mut Guard,
    work: impl FnOnce(&mut Guard, &Progress, &CancelToken) -> squid_core::Result<T>,
) -> squid_core::Result<T> {
    let label = label.to_owned();
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
                    Event::Deleted { path, freed } => shout(
                        "90",
                        &format!(
                            "  ✓ {} ({})",
                            shrink(&paths::pretty(&path), 70),
                            freed.human()
                        ),
                    ),
                    Event::Bucket { .. }
                    | Event::Metered { .. }
                    | Event::Group { .. }
                    | Event::Done(_) => {}
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
    let result = work(guard, &progress, &cancel);
    stop.store(true, Ordering::Relaxed);
    let _ = printer_thread.join();
    result
}
