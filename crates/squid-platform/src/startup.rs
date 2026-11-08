//! Everything that runs when the machine does: LaunchAgents, login items, the
//! registry Run keys, the startup folder, scheduled tasks and services.
//!
//! The list is the product here. A cleaner that only knows about files cannot
//! answer "why is this thing back after I deleted it", and the answer is nearly
//! always one of these entries. The inventory is therefore read-only by
//! default: every entry says whether squid may change it, and the ones it may
//! not say why in the same field the UI shows.
//!
//! Entries are also attributed to the application that installed them, because
//! nobody reads 800 plist labels: both operating systems show this list per app,
//! and `StartupOwner` is what makes that possible without guessing.
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
    /// The target is not readable by this user, so its signature was never asked
    /// about. Different from `Unsigned`, which is an answer and an accusation.
    SignatureUnreadable,
    /// Started through a script interpreter rather than a binary.
    ScriptInterpreter,
    /// Created within the last fortnight.
    RecentlyCreated,
    /// Marked hidden, or in a folder a person would not browse.
    HiddenEntry,
    /// Runs from a dot-directory directly under the home folder.
    HiddenHomeDirectory,
}

/// Which of the two lists the operating systems themselves show an entry
/// belongs to. macOS splits "Login Items" (apps that open when you log in) from
/// "Allow in the Background" (helpers that keep running); Windows splits the
/// Startup folder and Run keys from scheduled tasks and services. A page that
/// shows one flat list of 800 labels answers neither question.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StartupCategory {
    /// Opens an application or window at login: a login item, a Startup folder
    /// shortcut, or a Run value that launches an app.
    OpenAtLogin,
    /// A helper that keeps running without a window: LaunchAgents and
    /// LaunchDaemons, scheduled tasks, services.
    Background,
}

impl StartupCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            StartupCategory::OpenAtLogin => "open-at-login",
            StartupCategory::Background => "background",
        }
    }
}

/// Why squid will not change an entry. A typed answer rather than a sentence
/// from the platform layer, because the platform layer does not know which
/// language the page is wearing; each variant becomes one short localised phrase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StartupLock {
    /// Apple's or Microsoft's own entry.
    OperatingSystem,
    /// Root-owned plist, HKLM value, system task or service: one authorisation away.
    NeedsAuthorisation,
    /// The declaring file or folder is not writable by this user.
    NotWritable,
    /// macOS owns the login-item list and changes it through System Settings.
    ManagedBySystemSettings,
    /// This release reports these entries and leaves them alone.
    NotManaged,
}

/// The application an entry belongs to, worked out from the paths launchd or the
/// registry name, never from the entry's label. This is what lets the list be
/// "Docker · 3 background items" instead of three opaque plist names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupOwner {
    /// The name to show the user: the bundle's `CFBundleDisplayName`, or the
    /// application folder name when the bundle cannot be read.
    pub name: String,
    /// The `.app` bundle, or the directory the entry lives in.
    pub bundle_path: Option<PathBuf>,
    /// True when the owning application is Apple's or Microsoft's own.
    pub vendor: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    /// False means squid will refuse to touch it, and `lock` says which kind of
    /// refusal it is.
    pub modifiable: bool,
    pub lock: Option<StartupLock>,
    pub hidden: bool,
    pub modified_ms: Option<u64>,
    pub risks: Vec<StartupRisk>,
    /// Which list of the operating system's own this entry would appear in.
    pub category: StartupCategory,
    /// The application that installed the entry, when one could be named.
    pub owner: Option<StartupOwner>,
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
            lock: None,
            hidden: false,
            modified_ms: None,
            risks: Vec::new(),
            category: StartupCategory::Background,
            owner: None,
        }
    }
}

/// Everything squid could read. Unreadable folders are simply absent, and the
/// notes say which ones and why.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StartupInventory {
    pub items: Vec<StartupItem>,
    pub notes: Vec<StartupNote>,
}

/// Something the page has to admit it could not read. `kind` is typed rather than
/// a sentence, because the platform layer does not know whether the page is
/// wearing Chinese or English; a note that arrives already translated is a note
/// that arrives in the wrong language. `detail` carries the operating system's own
/// words beside it, which are never translated and are therefore honest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupNote {
    pub kind: StartupNoteKind,
    /// The folder, tool or list this note is about.
    pub subject: String,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StartupNoteKind {
    /// A folder holding entries that could not be listed.
    UnreadableRoot,
    /// The login-item list could not be read at all, which on macOS means the
    /// automation permission has not been granted.
    LoginItemsUnavailable,
}

/// The command line speaks English, so it prints the note whole; the GUI reads
/// `kind` and translates it. One fact, two honest renderings.
impl std::fmt::Display for StartupNote {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            StartupNoteKind::UnreadableRoot => {
                write!(formatter, "{} could not be listed", self.subject)?;
            }
            StartupNoteKind::LoginItemsUnavailable => {
                write!(formatter, "login items could not be read")?;
            }
        }
        if self.detail.is_empty() {
            Ok(())
        } else {
            write!(formatter, ": {}", self.detail)
        }
    }
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

/// What a refused change says. The reason itself is shown from `StartupLock`,
/// which the page can translate; this is only what the command returns when the
/// inventory it was given has already gone stale.
const LOCK_REFUSAL: &str = "Squid cannot change this entry.";

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

/// Judges one resolved program: freshness and where it lives. The signature half
/// is deliberately not here — see `apply_trust`.
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
}

/// The signature half of the judgement, kept apart from `judge_program` because
/// on macOS it costs one `codesign` process per entry. An M1 Pro has ~870 startup
/// entries, 845 of which Apple owns and nobody is going to inspect, so assessing
/// all of them took 8 s to answer a question about 25. Entries whose target is
/// gone are not assessed either: "not there" is already the finding.
fn apply_trust(item: &mut StartupItem) {
    if item.system_vendor || item.risks.contains(&StartupRisk::MissingTarget) {
        return;
    }
    let Some(program) = item.program.clone() else {
        return;
    };
    // A root-owned helper installed mode 0711 — `/Library/PrivilegedHelperTools/
    // com.docker.socket`, for one — can be run and cannot be read, and `codesign`
    // answers that with "Permission denied". Reporting it as *unsigned* would be a
    // false accusation against a program that is signed and merely private.
    if !program.is_dir() && std::fs::File::open(&program).is_err() {
        item.risks.push(StartupRisk::SignatureUnreadable);
        return;
    }
    let trust = crate::trust::assess(&program);
    item.signed = trust.signed;
    item.publisher = trust.signer.clone();
    if !trust.signed {
        item.risks.push(StartupRisk::Unsigned);
    }
}

/// The `.app` bundle an entry belongs to, found in the path launchd will run.
/// macOS nests helpers inside the bundle that owns them, which is why this works
/// without asking Launch Services anything.
#[cfg(target_os = "macos")]
fn bundle_owning(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.to_string_lossy().to_lowercase().ends_with(".app") {
            return Some(candidate.to_path_buf());
        }
        current = candidate.parent();
    }
    None
}

/// The installed applications, keyed by bundle id.
///
/// A helper installed in `/Library/PrivilegedHelperTools` says nothing about its
/// owner in its path, but launchd's convention is that its label begins with the
/// bundle id of the app that installed it. Checking that against what is actually
/// installed is a lookup rather than a guess: `com.docker.socket` is Docker's
/// because `com.docker.*` belongs to exactly one installed application.
#[cfg(target_os = "macos")]
#[derive(Default)]
struct AppIndex {
    by_id: std::collections::BTreeMap<String, StartupOwner>,
}

#[cfg(target_os = "macos")]
impl AppIndex {
    fn scan() -> AppIndex {
        let mut out = AppIndex::default();
        let roots = [
            PathBuf::from("/Applications"),
            home_dir().join("Applications"),
            PathBuf::from("/System/Applications"),
            PathBuf::from("/System/Library/CoreServices"),
        ];
        for root in roots {
            let Ok(entries) = std::fs::read_dir(&root) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.to_string_lossy().ends_with(".app") {
                    continue;
                }
                let Some(info) = bundle_info(&path) else {
                    continue;
                };
                out.by_id.entry(info.0).or_insert(StartupOwner {
                    name: info.1,
                    bundle_path: Some(path.clone()),
                    vendor: root.starts_with("/System"),
                });
            }
        }
        out
    }

    /// The longest bundle-id prefix of `label` that one installed app owns.
    fn owner_of(&self, label: &str) -> Option<StartupOwner> {
        let mut parts: Vec<String> = label
            .to_lowercase()
            .split('.')
            .map(|part| part.to_string())
            .collect();
        // The shortest usable prefix has two components. `org` on its own is a
        // registry suffix, not a vendor, and stopping there matched
        // `org.wireshark.ChmodBPF` to the only `org.*` application installed.
        while parts.len() > 2 {
            parts.pop();
            let prefix = parts.join(".");
            let matches: Vec<&StartupOwner> = self
                .by_id
                .range(prefix.clone()..)
                .take_while(|(id, _)| *id == &prefix || id.starts_with(&format!("{prefix}.")))
                .map(|(_, owner)| owner)
                .collect();
            // Only claim it when a single application is in the family; two
            // installers sharing a vendor prefix is a tie, and a tie is a guess.
            if matches.len() == 1 {
                return Some(matches[0].clone());
            }
            if matches.len() > 1 {
                break;
            }
        }
        None
    }
}

/// The bundle id and display name an `.app` declares for itself.
#[cfg(target_os = "macos")]
fn bundle_info(bundle: &Path) -> Option<(String, String)> {
    let dict = plist::Value::from_file(bundle.join("Contents/Info.plist"))
        .ok()?
        .into_dictionary()?;
    let id = dict
        .get("CFBundleIdentifier")
        .and_then(|value| value.as_string())?
        .to_lowercase();
    let name = ["CFBundleDisplayName", "CFBundleName"]
        .iter()
        .find_map(|key| dict.get(key).and_then(|value| value.as_string()))
        .map(str::to_string)
        .or_else(|| {
            bundle
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })?;
    Some((id, name))
}

/// Name the application every entry belongs to.
///
/// macOS nests helpers inside the `.app` that installed them, so the path usually
/// answers it; where it does not, the entry's launchd label is matched against the
/// bundle ids of the applications that are actually installed. Windows has no such
/// nesting, so the install folder under Program Files carries the name. An entry
/// that still names nothing stays ungrouped rather than being filed under a guess.
#[cfg(target_os = "macos")]
fn attribute_owners(items: &mut [StartupItem]) {
    let mut index: Option<AppIndex> = None;
    for item in items {
        if item.system_vendor {
            continue;
        }
        let bundle = item
            .program
            .clone()
            .or_else(|| {
                (item.id.contains('/') || item.id.ends_with(".app"))
                    .then(|| PathBuf::from(&item.id))
            })
            .as_deref()
            .and_then(bundle_owning);
        if let Some(bundle) = bundle {
            let vendor = bundle.starts_with("/System");
            let name = bundle_info(&bundle)
                .map(|(_, name)| name)
                .unwrap_or_else(|| {
                    bundle
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_else(|| bundle.display().to_string())
                });
            item.owner = Some(StartupOwner {
                name,
                bundle_path: Some(bundle),
                vendor,
            });
            continue;
        }
        // Nothing in the path names an application: a helper in
        // `/Library/PrivilegedHelperTools`, or an entry whose target is gone.
        let label = item
            .id
            .rsplit('/')
            .next()
            .unwrap_or(&item.id)
            .trim_end_matches(".plist")
            .trim_end_matches(".disabled");
        let index = index.get_or_insert_with(AppIndex::scan);
        if let Some(owner) = index.owner_of(label) {
            item.owner = Some(owner);
            continue;
        }
        // The signing organisation is the last honest answer, and it is the same
        // one System Settings groups background items by.
        if let Some(organisation) = item.publisher.as_deref().and_then(organisation_of) {
            item.owner = Some(StartupOwner {
                name: organisation,
                bundle_path: None,
                vendor: false,
            });
        }
    }
}

/// `Developer ID Application: Docker Inc (9P49FL5KQ8)` to `Docker Inc`.
#[cfg(target_os = "macos")]
fn organisation_of(signer: &str) -> Option<String> {
    let tail = signer.rsplit(": ").next().unwrap_or(signer);
    let name = tail.split(" (").next().unwrap_or(tail).trim();
    (!name.is_empty() && name != "not set").then(|| name.to_string())
}

#[cfg(windows)]
fn attribute_owners(items: &mut [StartupItem]) {
    for item in items {
        if item.system_vendor {
            continue;
        }
        let Some(program) = item.program.as_deref() else {
            continue;
        };
        // The publisher the Authenticode signature names is what Control Panel and
        // Task Manager both show, so it leads where the folder is a vendor directory.
        if let Some(name) = item
            .publisher
            .as_deref()
            .and_then(organisation_of)
            .or_else(|| install_folder_owning(program))
        {
            item.owner = Some(StartupOwner {
                name,
                bundle_path: None,
                vendor: false,
            });
        }
    }
}

/// `CN = "Microsoft Corporation"` and `Microsoft Corporation` to one name.
#[cfg(windows)]
fn organisation_of(signer: &str) -> Option<String> {
    let tail = signer.rsplit('=').next().unwrap_or(signer);
    let name = tail.trim_matches(|c| c == '"' || c == '\'').trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// The folder under Program Files that holds one executable, which is the unit
/// Windows itself groups installed programs by.
#[cfg(windows)]
fn install_folder_owning(program: &Path) -> Option<String> {
    let text = program.to_string_lossy();
    // Located without regard to case, then cut out of the original: the folder is
    // shown to the user, and `MOZILLA firefox` is not what it is called.
    let start = text.to_lowercase().find("program files")? + "program files".len();
    let mut parts = text[start..]
        .split(['\\', '/'])
        .filter(|part| !part.trim().is_empty());
    // ` (x86)` is part of the root, not a product, so the one folder after it is
    // the vendor or product a person would recognise.
    let first = parts.next()?;
    let name = if first.trim_start().starts_with('(') {
        parts.next()?
    } else {
        first
    };
    (!name.trim().is_empty()).then(|| name.trim().to_string())
}

/// What the operating system shows an entry under. Both platforms keep the same
/// two lists — the things that open a window when you log in, and the helpers
/// that run behind it — so the entry's own source decides it.
fn categorise(item: &mut StartupItem) {
    item.category = match item.source {
        StartupSource::LoginItem | StartupSource::StartupFolder | StartupSource::RegistryRun => {
            StartupCategory::OpenAtLogin
        }
        _ => StartupCategory::Background,
    };
}

#[cfg(test)]
mod attribution_tests {
    use super::*;

    /// The helper in `/Library/PrivilegedHelperTools` is named after nobody: its
    /// path says where helpers live, not who installed it. Its label is matched
    /// against installed bundle ids instead, which is honest only while exactly one
    /// application owns the family.
    #[test]
    #[cfg(target_os = "macos")]
    fn a_helper_is_claimed_only_by_an_unambiguous_bundle_family() {
        let owner = |name: &str| StartupOwner {
            name: name.to_string(),
            bundle_path: None,
            vendor: false,
        };
        let index = AppIndex {
            by_id: [
                ("com.docker.docker".to_string(), owner("Docker")),
                (
                    "com.apphousekitchen.aldente-pro".to_string(),
                    owner("AlDente"),
                ),
                ("org.geogebra6.mac".to_string(), owner("GeoGebra")),
            ]
            .into_iter()
            .collect(),
        };
        assert_eq!(
            index.owner_of("com.docker.socket").map(|o| o.name),
            Some("Docker".into())
        );
        assert_eq!(
            index
                .owner_of("com.apphousekitchen.aldente-pro.helper")
                .map(|o| o.name),
            Some("AlDente".into())
        );
        // `org` on its own is a registry suffix, not a vendor. Without that bound
        // `org.wireshark.ChmodBPF` was matched to the only `org.*` app installed.
        assert_eq!(index.owner_of("org.wireshark.ChmodBPF"), None);
        assert_eq!(index.owner_of("us.zoom.ZoomDaemon"), None);

        // Two applications in one family is a tie, and a tie is not an answer.
        let tied = AppIndex {
            by_id: [
                ("com.google.chrome".to_string(), owner("Chrome")),
                ("com.google.keystone".to_string(), owner("Keystone")),
            ]
            .into_iter()
            .collect(),
        };
        // Where a prefix names one application exactly, that is the answer even
        // though a shallower prefix would be a tie.
        assert_eq!(
            tied.owner_of("com.google.keystone.agent").map(|o| o.name),
            Some("Keystone".into())
        );
        assert_eq!(
            tied.owner_of("com.google.Chrome.helper").map(|o| o.name),
            Some("Chrome".into()),
            "the deeper prefix that does resolve is still used"
        );
        assert_eq!(
            tied.owner_of("com.google.SomeUpdater"),
            None,
            "the only prefix left is a tie between two applications, so nothing is claimed"
        );
    }

    #[test]
    #[cfg(windows)]
    fn a_program_files_path_names_the_product_folder() {
        assert_eq!(
            install_folder_owning(Path::new(
                r"C:\Program Files\Google\Chrome\Application\chrome.exe"
            )),
            Some("Google".into())
        );
        // ` (x86)` belongs to the root, so the product is the folder after it.
        assert_eq!(
            install_folder_owning(Path::new(
                r"C:\Program Files (x86)\Mozilla Firefox\firefox.exe"
            )),
            Some("Mozilla Firefox".into())
        );
        assert_eq!(
            install_folder_owning(Path::new(
                r"C:\program files (x86)\MOZILLA Firefox\firefox.exe"
            )),
            Some("MOZILLA Firefox".into()),
            "found without regard to case, shown exactly as the folder is named"
        );
        assert_eq!(
            install_folder_owning(Path::new(r"D:\tools\bin\thing.exe")),
            None
        );
    }

    #[test]
    fn an_entry_goes_to_the_list_its_source_declares() {
        let mut login = StartupItem::new(
            "login-item://Docker",
            "Docker",
            StartupSource::LoginItem,
            StartupScope::User,
        );
        let mut daemon = StartupItem::new(
            "/Library/LaunchDaemons/com.docker.vmnetd.plist",
            "com.docker.vmnetd",
            StartupSource::LaunchDaemon,
            StartupScope::System,
        );
        categorise(&mut login);
        categorise(&mut daemon);
        assert_eq!(login.category, StartupCategory::OpenAtLogin);
        assert_eq!(daemon.category, StartupCategory::Background);
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

/// What happens to every entry once both platforms have finished collecting them.
///
/// Trust comes first because an entry whose path names no application is filed
/// under whoever signed it, and that is only known once the signature has been
/// asked about. On macOS that question costs one `codesign` process per entry, so
/// it is asked in parallel: ~870 sequential calls took 8 s on an M1 Pro, and the
/// same calls across the available cores take under 1 s.
fn finish(items: &mut [StartupItem]) {
    use rayon::prelude::*;

    items.par_iter_mut().for_each(apply_trust);
    attribute_owners(items);
    for item in items.iter_mut() {
        categorise(item);
    }
    sort_items(items);
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
                        out.notes.push(StartupNote {
                            kind: StartupNoteKind::UnreadableRoot,
                            subject: root.dir.display().to_string(),
                            detail: error.to_string(),
                        });
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
        out.items.extend(login_items(&mut out.notes));
        finish(&mut out.items);
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
            item.lock = Some(StartupLock::OperatingSystem);
            item.risks.clear();
        } else if scope == StartupScope::System {
            item.lock = Some(StartupLock::NeedsAuthorisation);
        } else {
            let writable = crate::is_user_writable(path)
                && crate::is_user_writable(path.parent().unwrap_or(path));
            item.modifiable = writable;
            if !writable {
                item.lock = Some(StartupLock::NotWritable);
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

    /// What System Settings shows under "Open at Login".
    ///
    /// Asking System Events rather than parsing a file is the point: the list is
    /// maintained by backgroundtaskmanagement, its on-disk archive is a keychain
    /// of security-scoped bookmarks that only root can resolve, and the one
    /// readable copy in the home folder held 9 of the 30 groups the panel shows.
    /// System Events *is* the panel's own view, so the two cannot disagree.
    ///
    /// Changing an entry still belongs to System Settings — the scriptable
    /// `delete` is unreliable across macOS versions — so every row is reported
    /// with its lock rather than with a toggle that might not hold.
    fn login_items(notes: &mut Vec<StartupNote>) -> Vec<StartupItem> {
        let script = "tell application \"System Events\"\n\
            set collected to {}\n\
            repeat with anItem in (every login item)\n\
                set theName to \"\"\n\
                try\n\
                    set theName to name of anItem\n\
                end try\n\
                set thePath to \"\"\n\
                try\n\
                    set thePath to (path of anItem) as text\n\
                end try\n\
                set hiddenFlag to false\n\
                try\n\
                    set hiddenFlag to (hidden of anItem)\n\
                end try\n\
                set end of collected to theName & tab & thePath & tab & (hiddenFlag as text)\n\
            end repeat\n\
            set AppleScript's text item delimiters to linefeed\n\
            return collected as text\n\
            end tell";
        let output = match std::process::Command::new("osascript")
            .args(["-e", script])
            .output()
        {
            Ok(output) if output.status.success() => {
                String::from_utf8_lossy(&output.stdout).into_owned()
            }
            Ok(output) => {
                let reason = String::from_utf8_lossy(&output.stderr).trim().to_string();
                notes.push(StartupNote {
                    kind: StartupNoteKind::LoginItemsUnavailable,
                    subject: String::new(),
                    detail: if reason.is_empty() {
                        "osascript failed".into()
                    } else {
                        reason
                    },
                });
                return Vec::new();
            }
            Err(error) => {
                notes.push(StartupNote {
                    kind: StartupNoteKind::LoginItemsUnavailable,
                    subject: String::new(),
                    detail: error.to_string(),
                });
                return Vec::new();
            }
        };

        parse_login_items(&output)
    }

    /// Turn the script's report into entries. One line per item, tab-separated:
    /// name, path, and whether it launches hidden.
    pub(super) fn parse_login_items(output: &str) -> Vec<StartupItem> {
        let mut items = Vec::new();
        for line in output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let mut fields = line.split('\t');
            let name = fields.next().unwrap_or_default().trim();
            if name.is_empty() {
                continue;
            }
            // System Events reports `missing value` for an item whose file is
            // gone, which is the interesting case rather than a reason to drop it.
            let path = fields.next().unwrap_or_default().trim();
            let path = if path.is_empty() || path == "missing value" {
                String::new()
            } else {
                path.to_string()
            };
            let hidden = fields.next().unwrap_or_default().trim() == "true";
            let mut item = StartupItem::builder(
                format!("login-item://{name}"),
                name.to_string(),
                StartupSource::LoginItem,
                StartupScope::User,
            );
            // Left empty rather than filled with the name: repeating the name under
            // itself says nothing, and the page can say what is actually true.
            item.command = path.clone();
            item.hidden = hidden;
            judge_program(
                &mut item,
                (!path.is_empty()).then(|| Path::new(path.as_str())),
            );
            if hidden {
                item.risks.push(StartupRisk::HiddenEntry);
            }
            item.modifiable = false;
            item.lock = Some(StartupLock::ManagedBySystemSettings);
            items.push(item);
        }
        items
    }

    pub fn set_enabled(id: &str, enabled: bool) -> Result<StartupItem, String> {
        let current = inventory();
        let item = current
            .find(id)
            .ok_or_else(|| "That startup item is no longer there.".to_string())?;
        if !item.modifiable {
            return Err(LOCK_REFUSAL.to_string());
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
    fn a_login_item_report_becomes_entries_that_cannot_be_touched() {
        // The shape System Events answers with: name, path, hidden. Nothing about a
        // real machine is assumed, so the three rows are a bundle that is there, one
        // that is gone, and the `missing value` an item whose file vanished is
        // reported as. Dropping the last two would hide the entries worth seeing.
        let f = fixture("loginitems");
        let present = f.home.join("Keeper.app");
        std::fs::create_dir_all(&present).expect("bundle");
        let gone = f.home.join("Uninstalled.app");
        let report = format!(
            "Keeper\t{}\tfalse\nUninstalled\t{}\tfalse\nDoubao\tmissing value\ttrue\n",
            present.display(),
            gone.display()
        );

        let mut items = macos::parse_login_items(&report);
        assert_eq!(items.len(), 3);
        // Which list an entry belongs to is settled once, over the whole inventory,
        // by the same step the real inventory runs.
        for item in &mut items {
            categorise(item);
        }

        for item in &items {
            assert_eq!(item.source, StartupSource::LoginItem);
            assert_eq!(item.scope, StartupScope::User);
            assert_eq!(item.category, StartupCategory::OpenAtLogin);
            assert!(!item.modifiable, "macOS owns this list");
            assert_eq!(item.lock, Some(StartupLock::ManagedBySystemSettings));
            assert_eq!(item.id, format!("login-item://{}", item.name));
        }

        assert_eq!(items[0].program.as_deref(), Some(present.as_path()));
        assert!(
            items[1].risks.contains(&StartupRisk::MissingTarget),
            "an entry pointing at nothing says so rather than disappearing"
        );

        let vanished = &items[2];
        assert!(
            vanished.command.is_empty(),
            "a name is not a path, and repeating one under the other says nothing"
        );
        assert!(vanished.hidden);
        assert!(vanished.risks.contains(&StartupRisk::HiddenEntry));
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
                        item.lock = Some(StartupLock::NeedsAuthorisation);
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
                    item.lock = Some(StartupLock::NeedsAuthorisation);
                } else {
                    item.modifiable = crate::is_user_writable(&folder);
                    if !item.modifiable {
                        item.lock = Some(StartupLock::NotWritable);
                    }
                }
                out.items.push(item);
            }
        }

        out.items.extend(parked_entries());
        out.items.extend(scheduled_tasks());
        out.items.extend(services());
        finish(&mut out.items);
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
                item.lock = Some(StartupLock::NeedsAuthorisation);
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
            item.lock = Some(StartupLock::NotManaged);
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
            item.lock = Some(StartupLock::NotManaged);
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
            return Err(LOCK_REFUSAL.to_string());
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
