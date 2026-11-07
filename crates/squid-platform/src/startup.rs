//! Everything that runs when the machine does: LaunchAgents, login items, the
//! registry Run keys, the startup folder, scheduled tasks and services.
//!
//! The list is the product here. A cleaner that only knows about files cannot
//! answer "why is this thing back after I deleted it", and the answer is nearly
//! always one of these entries. The inventory is therefore read-only by
//! default: every entry says whether squid may change it, and the ones it may
//! not say why in the same field the UI shows.
//!
//! Disabling an entry means renaming its plist to `*.plist.disabled` on macOS
//! and moving its registry value into a sibling backup key on Windows. Both are
//! reversible without a privileged helper, which is the whole point: squid never
//! installs a service and never asks to become one.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Who owns the entry, which decides who has to approve changing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StartupScope {
    /// Inside the current user's home or HKCU: squid may change it.
    User,
    /// Root-owned file or HKLM/SYSTEM task: needs one OS authorisation.
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StartupSource {
    LaunchAgent,
    LaunchDaemon,
    LoginItem,
    RegistryRun,
    StartupFolder,
    ScheduledTask,
    Service,
}

impl StartupSource {
    pub fn as_str(self) -> &'static str {
        match self {
            StartupSource::LaunchAgent => "launch-agent",
            StartupSource::LaunchDaemon => "launch-daemon",
            StartupSource::LoginItem => "login-item",
            StartupSource::RegistryRun => "registry-run",
            StartupSource::StartupFolder => "startup-folder",
            StartupSource::ScheduledTask => "scheduled-task",
            StartupSource::Service => "service",
        }
    }
}

/// Why an entry deserves a second look. None of these mean "malicious"; they
/// mean the entry is the kind that hides things.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StartupRisk {
    /// The program the entry points at is not there any more.
    MissingTarget,
    /// Living in a temporary folder, where malware likes to be rebuilt.
    InTempDir,
    /// The target exists but carries no usable code signature.
    Unsigned,
    /// Started through a script interpreter rather than a binary.
    ScriptInterpreter,
    /// Created within the last fortnight.
    RecentlyCreated,
    /// Marked hidden, or in a folder a person would not browse.
    HiddenEntry,
    /// Runs from a dot-directory directly under the home folder.
    HiddenHomeDirectory,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StartupItem {
    /// Stable identifier: the plist path, registry value, task path or service.
    pub id: String,
    pub name: String,
    pub source: StartupSource,
    pub scope: StartupScope,
    /// The program the entry runs, as the author wrote it.
    pub command: String,
    /// The executable squid resolved from that command, when it could.
    pub program: Option<PathBuf>,
    pub working_directory: Option<PathBuf>,
    /// Vendor named by the code signature, when the target is signed.
    pub publisher: Option<String>,
    pub signed: bool,
    /// An entry the operating system vendor owns.
    pub system_vendor: bool,
    pub disabled: bool,
    /// False means squid will refuse to touch it, with `readonly_reason`.
    pub modifiable: bool,
    pub readonly_reason: Option<String>,
    pub hidden: bool,
    pub modified_ms: Option<u64>,
    pub risks: Vec<StartupRisk>,
}

impl StartupItem {
    /// A bare item for tests and for callers that build one by hand.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        source: StartupSource,
        scope: StartupScope,
    ) -> Self {
        Self::builder(id.into(), name.into(), source, scope)
    }

    fn builder(id: String, name: String, source: StartupSource, scope: StartupScope) -> Self {
        StartupItem {
            id,
            name,
            source,
            scope,
            command: String::new(),
            program: None,
            working_directory: None,
            publisher: None,
            signed: false,
            system_vendor: false,
            disabled: false,
            modifiable: false,
            readonly_reason: None,
            hidden: false,
            modified_ms: None,
            risks: Vec::new(),
        }
    }
}

/// Everything squid could read. Unreadable folders are simply absent, and the
/// notes say which roots could not be listed.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StartupInventory {
    pub items: Vec<StartupItem>,
    pub notes: Vec<String>,
}

impl StartupInventory {
    pub fn user_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.scope == StartupScope::User)
            .count()
    }

    pub fn system_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.scope == StartupScope::System)
            .count()
    }

    pub fn disabled_count(&self) -> usize {
        self.items.iter().filter(|item| item.disabled).count()
    }

    pub fn find(&self, id: &str) -> Option<&StartupItem> {
        self.items.iter().find(|item| item.id == id)
    }
}

/// Collect every entry this platform exposes.
pub fn inventory() -> StartupInventory {
    #[cfg(target_os = "macos")]
    {
        macos::inventory()
    }
    #[cfg(windows)]
    {
        windows::inventory()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        StartupInventory::default()
    }
}

/// Enable or disable one entry, returning the refreshed item.
pub fn set_enabled(id: &str, enabled: bool) -> Result<StartupItem, String> {
    #[cfg(target_os = "macos")]
    {
        macos::set_enabled(id, enabled)
    }
    #[cfg(windows)]
    {
        windows::set_enabled(id, enabled)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (id, enabled);
        Err("Startup item management is not implemented for this platform".into())
    }
}

/// The startup entry that launches one executable, if any. Used by the process
/// table so a row can say "this came from a login item".
pub fn entry_for(program: &Path) -> Option<StartupItem> {
    #[cfg(target_os = "macos")]
    {
        macos::entry_for(program)
    }
    #[cfg(windows)]
    {
        windows::entry_for(program)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = program;
        None
    }
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

fn modified_ms(path: &Path) -> Option<u64> {
    let time = std::fs::metadata(path).ok()?.modified().ok()?;
    match time.duration_since(UNIX_EPOCH) {
        Ok(since) => Some(since.as_millis() as u64),
        Err(_) => Some(0),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

const RECENT_MS: u64 = 14 * 24 * 60 * 60 * 1000;

/// Judges one resolved program: signature, freshness, and where it lives.
fn judge_program(item: &mut StartupItem, program: Option<&Path>) {
    let Some(program) = program else {
        return;
    };
    item.program = Some(program.to_path_buf());
    if !program.exists() {
        item.risks.push(StartupRisk::MissingTarget);
        return;
    }
    if item.modified_ms.is_none() {
        item.modified_ms = modified_ms(program);
    }
    if let Some(modified) = item.modified_ms {
        if modified > 0 && now_ms().saturating_sub(modified) < RECENT_MS {
            item.risks.push(StartupRisk::RecentlyCreated);
        }
    }

    let text = program.to_string_lossy().to_lowercase();
    if text.contains("/temp/")
        || text.contains("\\temp\\")
        || text.starts_with("/private/var/folders/")
        || text.contains("/tmp/")
        || program.starts_with(home_dir().join("Library/Caches"))
    {
        item.risks.push(StartupRisk::InTempDir);
    }
    if in_hidden_home_dir(program) {
        item.risks.push(StartupRisk::HiddenHomeDirectory);
    }
    if let Some(file_name) = program
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
    {
        const INTERPRETERS: [&str; 7] = [
            "bash",
            "sh",
            "zsh",
            "python",
            "python3",
            "perl",
            "osascript",
        ];
        let stem = file_name.to_lowercase();
        let stem = stem.strip_suffix(".exe").unwrap_or(&stem);
        if INTERPRETERS.contains(&stem) {
            item.risks.push(StartupRisk::ScriptInterpreter);
        }
    }

    let trust = crate::trust::assess(program);
    item.signed = trust.signed;
    item.publisher = trust.signer.clone();
    if !trust.signed {
        item.risks.push(StartupRisk::Unsigned);
    }
}

/// True when the path lives in a dot-directory directly under home, which is
/// where updaters and helpers that nobody installed tend to sit.
fn in_hidden_home_dir(program: &Path) -> bool {
    let home = home_dir();
    let Ok(relative) = program.strip_prefix(&home) else {
        return false;
    };
    relative
        .components()
        .next()
        .map(|first| first.as_os_str().to_string_lossy().starts_with('.'))
        .unwrap_or(false)
}

/// Order user-owned entries first; inside a scope, alphabetically.
fn sort_items(items: &mut [StartupItem]) {
    items.sort_by(|a, b| {
        (b.scope == StartupScope::User)
            .cmp(&(a.scope == StartupScope::User))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use plist::Value;

    const SHELL_PATHS: [&str; 2] = ["/bin", "/usr/bin"];

    struct Root {
        dir: PathBuf,
        source: StartupSource,
        scope: StartupScope,
    }

    fn roots() -> Vec<Root> {
        let home = home_dir();
        vec![
            Root {
                dir: home.join("Library/LaunchAgents"),
                source: StartupSource::LaunchAgent,
                scope: StartupScope::User,
            },
            Root {
                dir: PathBuf::from("/Library/LaunchAgents"),
                source: StartupSource::LaunchAgent,
                scope: StartupScope::System,
            },
            Root {
                dir: PathBuf::from("/Library/LaunchDaemons"),
                source: StartupSource::LaunchDaemon,
                scope: StartupScope::System,
            },
            Root {
                dir: PathBuf::from("/System/Library/LaunchAgents"),
                source: StartupSource::LaunchAgent,
                scope: StartupScope::System,
            },
            Root {
                dir: PathBuf::from("/System/Library/LaunchDaemons"),
                source: StartupSource::LaunchDaemon,
                scope: StartupScope::System,
            },
        ]
    }

    pub fn inventory() -> StartupInventory {
        let mut out = StartupInventory::default();
        for root in roots() {
            let entries = match std::fs::read_dir(&root.dir) {
                Ok(entries) => entries,
                Err(error) => {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        out.notes.push(format!(
                            "{} could not be listed: {error}",
                            root.dir.display()
                        ));
                    }
                    continue;
                }
            };
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    name.ends_with(".plist") || name.ends_with(".plist.disabled")
                })
                .collect();
            files.sort();
            for path in files {
                if let Some(item) = read_plist(&path, root.source, root.scope) {
                    out.items.push(item);
                }
            }
        }
        out.items.extend(login_items());
        sort_items(&mut out.items);
        out
    }

    fn read_plist(path: &Path, source: StartupSource, scope: StartupScope) -> Option<StartupItem> {
        let file_name = path.file_name()?.to_string_lossy().into_owned();
        let label = file_name
            .trim_end_matches(".disabled")
            .trim_end_matches(".plist")
            .to_string();
        let mut item =
            StartupItem::builder(path.to_string_lossy().into_owned(), label, source, scope);
        item.disabled = file_name.ends_with(".disabled");
        item.modified_ms = modified_ms(path);
        item.system_vendor = path.starts_with("/System/Library");
        item.command = path.display().to_string();

        // A `.disabled` file still describes the program it used to run, so the
        // plist is parsed whichever suffix it currently wears.
        let Some(dict) = Value::from_file(path)
            .ok()
            .and_then(|v| v.into_dictionary())
        else {
            classify(&mut item, path, scope);
            return Some(item);
        };
        item.working_directory = dict
            .get("WorkingDirectory")
            .and_then(|value| value.as_string())
            .map(PathBuf::from);
        if let Some(arguments) = dict.get("ProgramArguments").and_then(|v| v.as_array()) {
            item.command = arguments
                .iter()
                .filter_map(|value| value.as_string())
                .collect::<Vec<_>>()
                .join(" ");
        }
        let program = program_of(&dict);
        judge_program(&mut item, program.as_deref().map(Path::new));
        classify(&mut item, path, scope);
        Some(item)
    }

    /// Decides what squid may do with the entry, and clears the risk flags on
    /// entries Apple owns: a hundred unsigned system agents is not a signal.
    fn classify(item: &mut StartupItem, path: &Path, scope: StartupScope) {
        if item.system_vendor {
            item.readonly_reason = Some("part of macOS".into());
            item.risks.clear();
        } else if scope == StartupScope::System {
            item.readonly_reason = Some("requires an administrator authorisation".into());
        } else {
            let writable = crate::is_user_writable(path)
                && crate::is_user_writable(path.parent().unwrap_or(path));
            item.modifiable = writable;
            if !writable {
                item.readonly_reason = Some("this file is not owned by you".into());
            }
        }
    }

    /// `Program`, or the head of `ProgramArguments`, resolved the way launchd
    /// resolves it: a bare name is looked up in the system path.
    fn program_of(dict: &plist::Dictionary) -> Option<String> {
        if let Some(program) = dict.get("Program").and_then(|value| value.as_string()) {
            return Some(resolve(program));
        }
        let head = dict
            .get("ProgramArguments")?
            .as_array()?
            .first()?
            .as_string()?;
        Some(resolve(head))
    }

    fn resolve(program: &str) -> String {
        let expanded = if let Some(rest) = program.strip_prefix('~') {
            home_dir()
                .join(rest.trim_start_matches('/'))
                .to_string_lossy()
                .into_owned()
        } else {
            program.to_string()
        };
        if expanded.contains('/') {
            return expanded;
        }
        SHELL_PATHS
            .iter()
            .map(|dir| PathBuf::from(dir).join(&expanded))
            .find(|candidate| candidate.exists())
            .map(|candidate| candidate.to_string_lossy().into_owned())
            .unwrap_or(expanded)
    }

    /// The login-item database macOS still maintains beside the modern
    /// per-app setting. Reading it costs one file scan; changing it belongs to
    /// System Settings, so these entries are reported read-only.
    fn login_items() -> Vec<StartupItem> {
        let path = home_dir().join(
            "Library/Application Support/com.apple.backgroundtaskmanagementagent/backgrounditems.btm",
        );
        let mut items = Vec::new();
        for found in printable_paths(&path, 8) {
            if !found.contains("/Applications/") && !found.starts_with("/Library/") {
                continue;
            }
            let label = Path::new(&found)
                .file_name()
                .map(|name| name.to_string_lossy().trim_end_matches(".app").to_string())
                .unwrap_or_else(|| found.clone());
            let mut item = StartupItem::builder(
                found.clone(),
                label,
                StartupSource::LoginItem,
                StartupScope::User,
            );
            item.command = found.clone();
            judge_program(&mut item, Some(Path::new(&found)));
            item.readonly_reason = Some("login items are managed in System Settings".into());
            items.push(item);
        }
        items
    }

    /// Pull printable absolute paths out of a binary archive without a parser.
    fn printable_paths(path: &Path, minimum: usize) -> Vec<String> {
        let Ok(bytes) = std::fs::read(path) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        let mut current = String::new();
        for byte in bytes {
            if byte.is_ascii_graphic() || byte == b' ' {
                current.push(byte as char);
            } else {
                if current.len() >= minimum && current.starts_with('/') {
                    found.push(current.clone());
                }
                current.clear();
            }
        }
        if current.len() >= minimum && current.starts_with('/') {
            found.push(current);
        }
        found.sort();
        found.dedup();
        found
    }

    pub fn set_enabled(id: &str, enabled: bool) -> Result<StartupItem, String> {
        let current = inventory();
        let item = current
            .find(id)
            .ok_or_else(|| "That startup item is no longer there.".to_string())?;
        if !item.modifiable {
            return Err(item
                .readonly_reason
                .clone()
                .unwrap_or_else(|| "Squid cannot change this item.".into()));
        }
        let path = PathBuf::from(id);
        let to = if enabled {
            without_disabled(&path)
        } else {
            with_disabled(&path)
        };
        if enabled && to.exists() {
            return Err("Something re-created this entry while you were looking.".into());
        }
        std::fs::rename(&path, &to)
            .map_err(|error| format!("Could not move the entry: {error}"))?;

        // Stop a just-disabled agent now instead of at the next login, and let
        // launchd adopt a re-enabled one. Both are best-effort: the rename is
        // the durable change, and neither asks for a password.
        let uid = unsafe { libc::getuid() };
        let domain = format!("gui/{uid}");
        let label = to
            .file_name()
            .map(|name| {
                name.to_string_lossy()
                    .trim_end_matches(".disabled")
                    .trim_end_matches(".plist")
                    .to_string()
            })
            .unwrap_or_default();
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &domain, &label])
            .output();
        if enabled {
            let _ = std::process::Command::new("launchctl")
                .args(["bootstrap", &domain, &to.to_string_lossy()])
                .output();
        }

        let refreshed = inventory();
        let moved = to.to_string_lossy().into_owned();
        refreshed
            .find(&moved)
            .or_else(|| refreshed.find(id))
            .cloned()
            .ok_or_else(|| "The entry moved somewhere Squid cannot see.".into())
    }

    fn with_disabled(path: &Path) -> PathBuf {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(".disabled");
        path.with_file_name(name)
    }

    fn without_disabled(path: &Path) -> PathBuf {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        path.with_file_name(name.trim_end_matches(".disabled"))
    }

    pub fn entry_for(program: &Path) -> Option<StartupItem> {
        inventory()
            .items
            .into_iter()
            .find(|item| item.program.as_deref() == Some(program))
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// A private home folder, with `HOME` pointed at it for as long as the guard
    /// is held.
    ///
    /// `inventory()` reads the real `~/Library/LaunchAgents`, so a test of it has
    /// to move `HOME`, which is process-wide state: two of these running at once
    /// would each see the other's agents and both would be wrong in a way that
    /// only appears under `cargo test`'s threads. The mutex is handed back rather
    /// than leaked, so the next test is blocked only while this one is actually
    /// running and a deadlock is impossible.
    struct Fixture {
        home: PathBuf,
        // Named so it is dropped at the end of the test, not of `fixture()`.
        _serial: std::sync::MutexGuard<'static, ()>,
    }

    fn fixture(tag: &str) -> Fixture {
        use std::sync::Mutex;
        static GUARD: Mutex<()> = Mutex::new(());
        let serial = GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let home = std::env::temp_dir().join(format!("squid-startup-{tag}"));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("Library/LaunchAgents")).expect("fixture root");
        std::env::set_var("HOME", &home);
        Fixture {
            home,
            _serial: serial,
        }
    }

    /// A plist small enough to read in a test and shaped the way launchd wants.
    fn agent(home: &Path, label: &str, program: &Path) -> PathBuf {
        let path = home
            .join("Library/LaunchAgents")
            .join(format!("{label}.plist"));
        std::fs::write(
            &path,
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{label}</string>
<key>Program</key><string>{}</string>
</dict></plist>"#,
                program.display()
            ),
        )
        .expect("write plist");
        path
    }

    #[test]
    fn a_user_agent_is_listed_with_the_program_it_runs() {
        let f = fixture("listed");
        let home = f.home.clone();
        let program = home.join("updater");
        std::fs::write(&program, b"#!/bin/sh\n").expect("program");
        let path = agent(&home, "com.tester.updater", &program);

        let inventory = inventory();
        let item = inventory
            .find(&path.to_string_lossy())
            .expect("the agent in the fixture home is listed");
        assert_eq!(item.name, "com.tester.updater");
        assert_eq!(item.source, StartupSource::LaunchAgent);
        assert_eq!(item.scope, StartupScope::User);
        assert_eq!(item.program.as_deref(), Some(program.as_path()));
        assert!(!item.disabled, "a plain .plist is an enabled entry");
    }

    #[test]
    fn disabling_and_re_enabling_one_entry_round_trips() {
        // 9.3: "user-level startup items can be disabled and restored". The
        // rename is the durable half of that, and it has to survive a re-read
        // rather than only a cached frontend row.
        let f = fixture("toggle");
        let home = f.home.clone();
        let program = home.join("updater");
        std::fs::write(&program, b"#!/bin/sh\n").expect("program");
        let path = agent(&home, "com.tester.toggle", &program);
        let id = path.to_string_lossy().into_owned();

        let off = set_enabled(&id, false).expect("the user's own agent can be disabled");
        assert!(off.disabled, "the refreshed item says it is off");
        assert!(path.with_extension("plist.disabled").exists());
        assert!(!path.exists(), "the enabled name is gone");
        assert!(
            inventory().find(&id).is_none(),
            "the old id is no longer what the inventory reports"
        );

        let moved = path.with_extension("plist.disabled");
        let on = set_enabled(&moved.to_string_lossy(), true).expect("it can be put back");
        assert!(!on.disabled);
        assert!(path.exists(), "the original file name is restored");
        assert!(!moved.exists());
    }

    #[test]
    fn a_disabled_agent_keeps_explaining_what_it_used_to_run() {
        let f = fixture("disabled");
        let home = f.home.clone();
        let program = home.join("helper");
        std::fs::write(&program, b"#!/bin/sh\n").expect("program");
        let path = agent(&home, "com.tester.helper", &program);
        let off = path.with_extension("plist.disabled");
        std::fs::rename(&path, &off).expect("rename");

        let listing = inventory();
        let item = listing
            .find(&off.to_string_lossy())
            .expect("a .disabled file is still listed");
        assert!(item.disabled);
        assert_eq!(item.name, "com.tester.helper", "the label drops .disabled");
        assert_eq!(
            item.program.as_deref(),
            Some(program.as_path()),
            "the plist is parsed whichever suffix it wears"
        );
    }

    #[test]
    fn a_vanished_target_is_reported_not_dropped() {
        let f = fixture("gone");
        let home = f.home.clone();
        let missing = home.join("nothing-here");
        let path = agent(&home, "com.tester.ghost", &missing);

        let listing = inventory();
        let item = listing
            .find(&path.to_string_lossy())
            .expect("an entry whose target is gone is still worth showing");
        assert!(item.risks.contains(&StartupRisk::MissingTarget));
    }

    #[test]
    fn entry_for_finds_the_agent_that_launches_a_program() {
        // 9.3: "process details can be linked to a startup item". The join is
        // this lookup, so it has to be exact rather than by name.
        let f = fixture("entryfor");
        let home = f.home.clone();
        let program = home.join("keeper");
        std::fs::write(&program, b"#!/bin/sh\n").expect("program");
        let path = agent(&home, "com.tester.keeper", &program);

        let found = entry_for(&program).expect("the process row can name its entry");
        assert_eq!(found.id, path.to_string_lossy());
        assert!(
            entry_for(&home.join("unrelated")).is_none(),
            "a program nothing starts has no entry"
        );
    }

    #[test]
    fn user_entries_come_before_system_entries() {
        let f = fixture("sorted");
        let home = f.home.clone();
        let program = home.join("zeta");
        std::fs::write(&program, b"#!/bin/sh\n").expect("program");
        agent(&home, "zz.zeta", &program);
        agent(&home, "aa.alpha", &program);

        let items = inventory().items;
        let first_system = items
            .iter()
            .position(|item| item.scope == StartupScope::System);
        let user: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.scope == StartupScope::User)
            .map(|(index, _)| index)
            .collect();
        if let Some(system) = first_system {
            assert!(
                user.iter().all(|index| *index < system),
                "the entries the user owns lead the list"
            );
        }
        let names: Vec<&str> = user
            .iter()
            .map(|index| items[*index].name.as_str())
            .filter(|name| name.ends_with("alpha") || name.ends_with("zeta"))
            .collect();
        assert_eq!(names, vec!["aa.alpha", "zz.zeta"], "then alphabetical");
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::process::CommandExt;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_ALL_ACCESS, KEY_READ};
    use winreg::RegKey;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN_ONCE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
    const DISABLED_KEY: &str = r"Software\Squid\Disabled Run";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn inventory() -> StartupInventory {
        let mut out = StartupInventory::default();

        for (hive, hive_name, scope) in [
            (HKEY_CURRENT_USER, "HKCU", StartupScope::User),
            (HKEY_LOCAL_MACHINE, "HKLM", StartupScope::System),
        ] {
            for subkey in [RUN_KEY, RUN_ONCE_KEY] {
                let Ok(key) = RegKey::predef(hive).open_subkey_with_flags(subkey, KEY_READ) else {
                    continue;
                };
                for name in key.enum_values().flatten().filter_map(|(name, _)| {
                    // `Run` and `Silent` are internal bookkeeping, not user entries.
                    if name.eq_ignore_ascii_case("silent") {
                        None
                    } else {
                        Some(name)
                    }
                }) {
                    let Ok(command) = key.get_value::<String, _>(&name) else {
                        continue;
                    };
                    let id = format!(r"{hive_name}\{subkey}\{name}");
                    let mut item =
                        StartupItem::builder(id, name, StartupSource::RegistryRun, scope);
                    item.command = command.clone();
                    item.hidden = subkey == RUN_ONCE_KEY;
                    judge_program(
                        &mut item,
                        program_of_command(&command).as_deref().map(Path::new),
                    );
                    if scope == StartupScope::System {
                        item.readonly_reason =
                            Some("requires an administrator authorisation".into());
                    } else {
                        item.modifiable = true;
                    }
                    if item.hidden {
                        item.risks.push(StartupRisk::HiddenEntry);
                    }
                    out.items.push(item);
                }
            }
        }

        for (folder, scope) in startup_folders() {
            let Ok(entries) = std::fs::read_dir(&folder) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let raw = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if raw.eq_ignore_ascii_case("desktop.ini") {
                    continue;
                }
                let disabled = raw.to_lowercase().ends_with(".squid-disabled");
                let label = raw
                    .trim_end_matches(".squid-disabled")
                    .trim_end_matches(".lnk")
                    .trim_end_matches(".exe")
                    .to_string();
                let mut item = StartupItem::builder(
                    path.to_string_lossy().into_owned(),
                    label,
                    StartupSource::StartupFolder,
                    scope,
                );
                item.command = path.display().to_string();
                item.disabled = disabled;
                item.modified_ms = modified_ms(&path);
                judge_program(&mut item, Some(&path));
                if scope == StartupScope::System {
                    item.readonly_reason = Some("requires an administrator authorisation".into());
                } else {
                    item.modifiable = crate::is_user_writable(&folder);
                    if !item.modifiable {
                        item.readonly_reason = Some("this folder is not writable".into());
                    }
                }
                out.items.push(item);
            }
        }

        out.items.extend(parked_entries());
        out.items.extend(scheduled_tasks());
        out.items.extend(services());
        sort_items(&mut out.items);
        out
    }

    fn startup_folders() -> Vec<(PathBuf, StartupScope)> {
        let mut folders = Vec::new();
        if let Ok(appdata) = std::env::var("APPDATA") {
            folders.push((
                PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs\Startup"),
                StartupScope::User,
            ));
        }
        if let Ok(programdata) = std::env::var("PROGRAMDATA") {
            folders.push((
                PathBuf::from(programdata).join(r"Microsoft\Windows\Start Menu\Programs\Startup"),
                StartupScope::System,
            ));
        }
        folders
    }

    /// Values this release parked. A disabled Run value is deleted from its
    /// key, so it can only be listed from here: the marker names where it came
    /// from and the `.command` sibling keeps what to put back.
    fn parked_entries() -> Vec<StartupItem> {
        let Ok(key) =
            RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(DISABLED_KEY, KEY_READ)
        else {
            return Vec::new();
        };
        let mut items = Vec::new();
        for name in key.enum_values().flatten().map(|(name, _)| name) {
            let Some(value) = name.strip_suffix(".command") else {
                continue;
            };
            let Ok(marker) = key.get_value::<String, _>(value) else {
                continue;
            };
            let Ok(command) = key.get_value::<String, _>(&name) else {
                continue;
            };
            let Some((hive_name, rest)) = marker.split_once('\\') else {
                continue;
            };
            let Some((_, _)) = rest.rsplit_once('\\') else {
                continue;
            };
            let mut item = StartupItem::builder(
                marker.clone(),
                value.to_string(),
                StartupSource::RegistryRun,
                StartupScope::User,
            );
            item.command = command.clone();
            item.disabled = true;
            judge_program(
                &mut item,
                program_of_command(&command).as_deref().map(Path::new),
            );
            item.modifiable = hive_name == "HKCU";
            if !item.modifiable {
                item.readonly_reason = Some("requires an administrator authorisation".into());
            }
            items.push(item);
        }
        items
    }

    /// Strip quotes and the `rundll32`-style indirection to reach a file path.
    fn program_of_command(command: &str) -> Option<String> {
        let trimmed = command.trim();
        let first = if let Some(rest) = trimmed.strip_prefix('"') {
            rest.split('"').next()?.to_string()
        } else {
            trimmed
                .split_whitespace()
                .next()?
                .trim_matches(|c| c == '"' || c == '\'')
                .to_string()
        };
        if first.is_empty() {
            return None;
        }
        let candidate = PathBuf::from(&first);
        if candidate.is_absolute() {
            return Some(first);
        }
        let system = std::env::var("SystemRoot").ok()?;
        let in_system = PathBuf::from(system).join("System32").join(&first);
        if in_system.is_file() {
            return Some(in_system.to_string_lossy().into_owned());
        }
        Some(first)
    }

    /// One `schtasks` call for the whole tree. Every column would be a hundred
    /// fields, so only the four that mean something here are kept.
    fn scheduled_tasks() -> Vec<StartupItem> {
        let Ok(output) = std::process::Command::new("schtasks")
            .creation_flags(CREATE_NO_WINDOW)
            .args(["/fo", "csv", "/v", "/nh"])
            .output()
        else {
            return Vec::new();
        };
        let text = String::from_utf8_lossy(&output.stdout);
        let mut items = Vec::new();
        for record in task_records(&text) {
            let Some(raw) = record.get("task name") else {
                continue;
            };
            let path = raw.replace('"', "");
            if !path.starts_with('\\') {
                continue;
            }
            let action = record.get("task to run").cloned().unwrap_or_default();
            if action.is_empty() || action.eq_ignore_ascii_case("n/a") {
                continue;
            }
            let status = record.get("status").cloned().unwrap_or_default();
            let author = record.get("author").cloned().unwrap_or_default();
            let name = path
                .rsplit_once('\\')
                .map(|(_, tail)| tail.to_string())
                .unwrap_or_else(|| path.clone());
            let mut item = StartupItem::builder(
                format!("task://{path}"),
                name,
                StartupSource::ScheduledTask,
                StartupScope::System,
            );
            item.command = action.clone();
            item.disabled = !status.is_empty() && !status.eq_ignore_ascii_case("enabled");
            item.system_vendor = author.eq_ignore_ascii_case("Microsoft Corporation");
            judge_program(
                &mut item,
                program_of_command(&action).as_deref().map(Path::new),
            );
            if item.system_vendor {
                item.risks.clear();
            }
            item.readonly_reason = Some("scheduled tasks are read-only in this release".into());
            items.push(item);
        }
        items
    }

    fn task_records(text: &str) -> Vec<std::collections::HashMap<String, String>> {
        let header = text.lines().next().unwrap_or_default();
        let columns = csv_fields(header);
        let wanted = ["Task Name", "Status", "Author", "Task To Run"];
        let mut out = Vec::new();
        for line in text.lines().skip(1) {
            let fields = csv_fields(line);
            if fields.len() < columns.len() {
                continue;
            }
            let mut record = std::collections::HashMap::new();
            for (index, column) in columns.iter().enumerate() {
                if wanted.iter().any(|want| want.eq_ignore_ascii_case(column)) {
                    record.insert(column.to_lowercase(), fields[index].trim().to_string());
                }
            }
            if record.values().any(|value| value.starts_with('\\')) {
                out.push(record);
            }
        }
        out
    }

    fn csv_fields(line: &str) -> Vec<String> {
        let mut fields = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        let mut chars = line.chars().peekable();
        while let Some(char) = chars.next() {
            match char {
                '"' if quoted && chars.peek() == Some(&'"') => {
                    current.push('"');
                    chars.next();
                }
                '"' => quoted = !quoted,
                ',' if !quoted => fields.push(std::mem::take(&mut current)),
                other => current.push(other),
            }
        }
        fields.push(current);
        fields
    }

    /// Only automatic services are listed. Listing every driver would bury the
    /// handful of entries a person can act on, and none of them are actionable
    /// in this release anyway.
    fn services() -> Vec<StartupItem> {
        let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(r"SYSTEM\CurrentControlSet\Services", KEY_READ)
        else {
            return Vec::new();
        };
        let mut items = Vec::new();
        for name in key.enum_keys().flatten() {
            let Ok(service) = key.open_subkey_with_flags(&name, KEY_READ) else {
                continue;
            };
            if service.get_value::<u32, _>("Start").ok() != Some(2) {
                continue;
            }
            let image = service
                .get_value::<String, _>("ImagePath")
                .unwrap_or_default();
            if !image.to_lowercase().ends_with(".exe") {
                continue;
            }
            let display = service
                .get_value::<String, _>("DisplayName")
                .unwrap_or_else(|_| name.clone());
            let mut item = StartupItem::builder(
                format!("service://{name}"),
                display,
                StartupSource::Service,
                StartupScope::System,
            );
            item.command = image.clone();
            judge_program(
                &mut item,
                expand_system_path(&image).as_deref().map(Path::new),
            );
            item.system_vendor = item
                .program
                .as_ref()
                .map(|path| path.starts_with(r"C:\Windows"))
                .unwrap_or(false);
            if item.system_vendor {
                item.risks.clear();
            }
            item.readonly_reason = Some("services are read-only in this release".into());
            items.push(item);
        }
        items
    }

    fn expand_system_path(image: &str) -> Option<String> {
        let system = std::env::var("SystemRoot").unwrap_or_default();
        let trimmed = image
            .trim()
            .trim_start_matches(r"\\?\")
            .replace("%SystemRoot%", &system)
            .replace("%systemroot%", &system);
        let first = if let Some(rest) = trimmed.strip_prefix('"') {
            rest.split('"').next()?.to_string()
        } else {
            trimmed.split_whitespace().next()?.to_string()
        };
        Some(first)
    }

    pub fn set_enabled(id: &str, enabled: bool) -> Result<StartupItem, String> {
        let current = inventory();
        let item = current
            .find(id)
            .cloned()
            .ok_or_else(|| "That startup item is no longer there.".to_string())?;
        if !item.modifiable {
            return Err(item
                .readonly_reason
                .clone()
                .unwrap_or_else(|| "Squid cannot change this item.".into()));
        }
        match item.source {
            StartupSource::RegistryRun => set_registry_enabled(&item, enabled),
            StartupSource::StartupFolder => set_folder_enabled(&item, enabled),
            _ => Err("This kind of entry is read-only in this release".into()),
        }
    }

    fn set_registry_enabled(item: &StartupItem, enabled: bool) -> Result<StartupItem, String> {
        let (hive_name, subkey, value) = split_id(&item.id)?;
        let hive = if hive_name == "HKLM" {
            HKEY_LOCAL_MACHINE
        } else {
            HKEY_CURRENT_USER
        };
        let key = RegKey::predef(hive)
            .open_subkey_with_flags(&subkey, KEY_ALL_ACCESS)
            .map_err(|error| format!("Squid cannot write this registry key: {error}"))?;
        let store = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(DISABLED_KEY)
            .map_err(|error| format!("Could not open the Squid backup key: {error}"))?
            .0;
        let marker = format!(r"{hive_name}\{subkey}\{value}");

        if enabled {
            let stored = store
                .get_value::<String, _>(&value)
                .map_err(|_| "Squid did not disable this entry.".to_string())?;
            if stored != marker {
                return Err("Squid did not disable this entry.".into());
            }
            let command = store
                .get_value::<String, _>(&format!("{value}.command"))
                .map_err(|_| "Squid lost the saved command for this entry.".to_string())?;
            key.set_value(&value, &command)
                .map_err(|error| format!("Could not restore the entry: {error}"))?;
            let _ = store.delete_value(format!("{value}.command").as_str());
            let _ = store.delete_value(&value);
        } else {
            let command = key
                .get_value::<String, _>(&value)
                .map_err(|_| "The value is no longer there.".to_string())?;
            store
                .set_value(&value, &marker)
                .map_err(|error| format!("Could not record the backup: {error}"))?;
            store
                .set_value(format!("{value}.command").as_str(), &command)
                .map_err(|error| format!("Could not back up the command: {error}"))?;
            key.delete_value(&value)
                .map_err(|error| format!("Could not remove the entry: {error}"))?;
        }

        let refreshed = inventory();
        refreshed
            .find(&item.id)
            .cloned()
            .ok_or_else(|| "The entry moved somewhere Squid cannot see.".into())
    }

    fn split_id(id: &str) -> Result<(&'static str, String, String), String> {
        let (head, rest) = id.split_once('\\').ok_or("Unrecognised entry id.")?;
        let hive_name = match head {
            "HKCU" => "HKCU",
            "HKLM" => "HKLM",
            _ => return Err("Unrecognised entry id.".into()),
        };
        let (subkey, value) = rest.rsplit_once('\\').ok_or("Unrecognised entry id.")?;
        Ok((hive_name, subkey.to_string(), value.to_string()))
    }

    fn set_folder_enabled(item: &StartupItem, enabled: bool) -> Result<StartupItem, String> {
        let path = PathBuf::from(&item.id);
        let to = if enabled {
            PathBuf::from(item.id.trim_end_matches(".squid-disabled"))
        } else {
            PathBuf::from(format!("{}.squid-disabled", item.id))
        };
        std::fs::rename(&path, &to)
            .map_err(|error| format!("Could not move the entry: {error}"))?;
        let refreshed = inventory();
        let moved = to.to_string_lossy().into_owned();
        refreshed
            .find(&moved)
            .cloned()
            .ok_or_else(|| "The entry moved somewhere Squid cannot see.".into())
    }

    pub fn entry_for(program: &Path) -> Option<StartupItem> {
        inventory()
            .items
            .into_iter()
            .find(|item| item.program.as_deref() == Some(program))
    }
}
