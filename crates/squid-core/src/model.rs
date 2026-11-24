//! The vocabulary shared by the engines, the CLI and the TUI.

use crate::bytes::Bytes;
use crate::progress::CancelToken;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// High level bucket an item belongs to. Mirrors the cards CleanMyMac shows on
/// its cleanup screen so the TUI can render one section per bucket.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    SystemJunk,
    AppCaches,
    BrowserCaches,
    Logs,
    Developer,
    ProjectArtifacts,
    Trash,
    LanguagePacks,
    Leftovers,
    UpdateInstallers,
    /// Duplicate-file scan results (its own page, streamed group by group).
    Duplicates,
    /// AI tool caches and local model files (Ollama, HuggingFace, …).
    AiTools,
    /// Privacy traces: browser history, recent items, clipboard, activity.
    Privacy,
}

impl Category {
    pub const ALL: [Category; 13] = [
        Category::SystemJunk,
        Category::AppCaches,
        Category::BrowserCaches,
        Category::Logs,
        Category::Developer,
        Category::ProjectArtifacts,
        Category::Trash,
        Category::LanguagePacks,
        Category::Leftovers,
        Category::UpdateInstallers,
        Category::Duplicates,
        Category::AiTools,
        Category::Privacy,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Category::SystemJunk => "system",
            Category::AppCaches => "caches",
            Category::BrowserCaches => "browsers",
            Category::Logs => "logs",
            Category::Developer => "developer",
            Category::ProjectArtifacts => "projects",
            Category::Trash => "trash",
            Category::LanguagePacks => "languages",
            Category::Leftovers => "leftovers",
            Category::UpdateInstallers => "update-installers",
            Category::Duplicates => "duplicates",
            Category::AiTools => "ai",
            Category::Privacy => "privacy",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Category::SystemJunk => "System Junk",
            Category::AppCaches => "Application Caches",
            Category::BrowserCaches => "Browser Caches",
            Category::Logs => "Logs & Crash Reports",
            Category::Developer => "Developer Files",
            Category::ProjectArtifacts => "Project Artifacts",
            Category::Trash => "Trash",
            Category::LanguagePacks => "Unneeded Languages",
            Category::Leftovers => "Uninstaller Leftovers",
            Category::UpdateInstallers => "Update Installers",
            Category::Duplicates => "Duplicate Files",
            Category::AiTools => "AI Tools & Models",
            Category::Privacy => "Privacy Traces",
        }
    }

    /// Description shown under the header, CleanMyMac style.
    pub fn blurb(self) -> &'static str {
        match self {
            Category::SystemJunk => "Temporary files, caches and scratch data left by macOS",
            Category::AppCaches => "Regenerable caches that apps keep in ~/Library/Caches",
            Category::BrowserCaches => "Web content, service workers and offline data",
            Category::Logs => "Rotated logs, diagnostic reports and crash dumps",
            Category::Developer => "DerivedData, simulators, containers, package caches",
            Category::ProjectArtifacts => {
                "Build outputs and dependency folders inside your projects"
            }
            Category::Trash => "Files waiting in the Trash",
            Category::LanguagePacks => "Speech and localization data for languages you do not use",
            Category::Leftovers => "Data belonging to apps that are no longer installed",
            Category::UpdateInstallers => "Installer packages apps keep after a successful update",
            Category::Duplicates => "Byte-identical files confirmed by a three-stage hash funnel",
            Category::AiTools => "Local model weights and caches that re-download when needed",
            Category::Privacy => "History and traces that disappear without a recycle bin",
        }
    }

    pub fn parse(value: &str) -> Option<Category> {
        let value = value.trim();
        if let Some(found) = Category::ALL.into_iter().find(|c| c.id() == value) {
            return Some(found);
        }
        // Rules are hand-written, so accept the obvious aliases as well.
        let key = value.to_ascii_lowercase().replace(['_', ' '], "-");
        let key = key.trim_matches('-');
        let alias = match key {
            "app-cache" | "app-caches" | "cache" | "caches" | "application-caches" => {
                Category::AppCaches
            }
            "browser-cache" | "browser-caches" | "browsers" | "web" => Category::BrowserCaches,
            "system-junk" | "system" | "junk" | "temp" => Category::SystemJunk,
            "log" | "logs" | "crash" | "crash-reports" => Category::Logs,
            "dev" | "developer" | "developer-files" | "xcode" => Category::Developer,
            "project" | "projects" | "project-artifacts" | "artifacts" => {
                Category::ProjectArtifacts
            }
            "trash" => Category::Trash,
            "language" | "languages" | "language-packs" | "localization" => Category::LanguagePacks,
            "leftover" | "leftovers" | "uninstall-leftovers" => Category::Leftovers,
            "update-installer" | "update-installers" | "upgrades" | "upgrade-packages" => {
                Category::UpdateInstallers
            }
            "duplicate" | "duplicates" | "duplicate-files" => Category::Duplicates,
            "ai" | "ai-tools" | "ai-models" | "models" => Category::AiTools,
            "privacy" | "privacy-traces" | "private" => Category::Privacy,
            _ => return None,
        };
        Some(alias)
    }
}

/// How a path must be emptied. Getting this right is the difference between
/// "freed 2 GB" and "broke the app".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItemKind {
    /// Delete the path and everything under it.
    Path,
    /// Delete the children only, keep the directory itself (cache roots that
    /// must keep existing so the owning app does not recreate them wrongly).
    Contents,
    /// A single file.
    File,
}

/// Why an item cannot be removed in this run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LockReason {
    /// The owning app is running; its caches may be memory-mapped.
    AppRunning,
    /// Another install of the same bundle id survives, so shared data must stay.
    SharedWithOtherInstall,
    /// The user protected it through the whitelist.
    Whitelisted,
    /// Not owned by the current user (needs admin rights).
    NotOwned,
    /// Inside a protected system location.
    Protected,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CleanupItem {
    pub path: PathBuf,
    pub kind: ItemKind,
    pub size: Bytes,
    pub category: Category,
    /// Human readable label, e.g. "Slack cache".
    pub label: String,
    /// Rule that produced the item; used for whitelist / debug output.
    pub rule_id: String,
    pub selected: bool,
    /// Bytes actually allocated on disk (sparse/compressed aware).
    pub allocated: Bytes,
    pub lock: Option<LockReason>,
    /// Extra context shown in the detail row: "12,431 files".
    pub note: Option<String>,
    /// Roots the producing rule owns. Deletion re-validates against these so a
    /// stale or hand-edited item can never escape its sandbox.
    #[serde(default)]
    pub anchors: Vec<PathBuf>,
    /// Seconds since the *most recent* last-write inside the item (Mole's
    /// decision aid: 1.2 GB alone is not actionable, "untouched for 3 months"
    /// is — and a low value flags a live, working directory). Unknown until
    /// measurement; `None` for unreadable paths.
    #[serde(default)]
    pub last_write_secs: Option<u64>,
    /// File count from measurement — lets the UI say "12 files · 5 folders".
    #[serde(default)]
    pub file_count: u64,
    /// Machine-readable judgement tokens (Mole's evidence chain, made
    /// visible): `lock:<file>`, `record:<file>`, `record-missing`,
    /// `modified:<n>`, `git:ignored|untracked|none|tracked`, `age:<days>`,
    /// `busy`. The UI renders each token as a ✓/! line in a detail panel.
    #[serde(default)]
    pub evidence: Vec<String>,
}

impl CleanupItem {
    pub fn new(
        path: PathBuf,
        kind: ItemKind,
        category: Category,
        label: impl Into<String>,
    ) -> Self {
        let selected = lock_default(kind).is_none();
        Self {
            path,
            kind,
            size: Bytes::ZERO,
            category,
            label: label.into(),
            rule_id: String::new(),
            selected,
            allocated: Bytes::ZERO,
            lock: lock_default(kind),
            note: None,
            anchors: Vec::new(),
            last_write_secs: None,
            file_count: 0,
            evidence: Vec::new(),
        }
    }

    pub fn with_rule(mut self, rule_id: &str) -> Self {
        self.rule_id = rule_id.to_string();
        self
    }

    pub fn with_anchors(mut self, anchors: impl IntoIterator<Item = PathBuf>) -> Self {
        self.anchors = anchors.into_iter().collect();
        self
    }

    /// Attach a human-readable explanation ("off by default", "unreadable", …).
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    pub fn locked(mut self, reason: LockReason) -> Self {
        self.lock = Some(reason);
        self.selected = false;
        self
    }

    pub fn is_removable(&self) -> bool {
        self.lock.is_none()
    }

    pub fn display_path(&self) -> String {
        crate::paths::pretty(&self.path)
    }

    /// Bytes the item really occupies on the disk.
    ///
    /// `allocated` comes from `st_blocks`, `size` from `st_size`, and they
    /// disagree by orders of magnitude for a sparse file: Docker's
    /// `Docker.raw` claims a terabyte of address space on a machine with a
    /// 256 GB SSD. Taking the larger of the two is what made squid promise
    /// more free space than the disk can hold, so the block count wins.
    ///
    /// On Unix the block count is the filesystem's own answer and is used
    /// as-is — a fully sparse image really does occupy nothing, exactly as
    /// `du -A` reports. Only a platform without block counts reads a zero as
    /// "not measured" and falls back to the logical length.
    pub fn on_disk(&self) -> Bytes {
        #[cfg(unix)]
        {
            self.allocated
        }
        #[cfg(not(unix))]
        {
            if self.allocated.is_zero() {
                self.size
            } else {
                self.allocated
            }
        }
    }

    /// Size to show a human: honours `settings.show_allocated`.
    pub fn shown(&self, show_allocated: bool) -> Bytes {
        if show_allocated {
            self.on_disk()
        } else {
            self.size
        }
    }
}

fn lock_default(kind: ItemKind) -> Option<LockReason> {
    match kind {
        ItemKind::Path | ItemKind::Contents | ItemKind::File => None,
    }
}

/// Aggregate produced by every scan.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScanReport {
    pub category: Option<Category>,
    pub items: Vec<CleanupItem>,
    pub scanned_at_ms: u64,
    /// Directories we were not allowed to read (permissions, TCC).
    pub unreadable: u32,
    /// Rules that matched nothing, kept for `--debug`.
    pub skipped_rules: Vec<String>,
}

impl ScanReport {
    /// What the checked items are worth: the same rule as
    /// [`CleanupItem::on_disk`], never the logical size. A report is read as
    /// "this much space comes back", and a 1 TiB sparse `Docker.raw` that
    /// occupies 12 GB is exactly the case where the two numbers differ by an
    /// order of magnitude — the interface that counted `item.size` announced
    /// "1.1 TB" over a 256 GB disk.
    pub fn total(&self) -> Bytes {
        self.items
            .iter()
            .filter(|i| i.selected)
            .map(|i| i.on_disk())
            .sum()
    }

    /// Every item, checked or not. See [`Self::total`] for why this counts the
    /// space occupied rather than the space claimed.
    pub fn total_all(&self) -> Bytes {
        self.items.iter().map(|i| i.on_disk()).sum()
    }

    pub fn selected_count(&self) -> usize {
        self.items.iter().filter(|i| i.selected).count()
    }

    /// Biggest first, by the space actually occupied, so the order of the list
    /// matches the number printed beside each row.
    pub fn sorted(mut self) -> Self {
        self.items.sort_by(|a, b| {
            b.on_disk()
                .cmp(&a.on_disk())
                .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
        });
        self
    }

    pub fn merge(&mut self, other: ScanReport) {
        self.items.extend(other.items);
        self.unreadable += other.unreadable;
        self.skipped_rules.extend(other.skipped_rules);
    }
}

/// What a caller wants to happen to the bytes it approved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DeleteMode {
    /// Move to the macOS/Windows Trash so the user can undo.
    Trash,
    /// unlink(2) immediately. Default for caches: they are regenerable and
    /// trashing 20 GB of caches just moves the problem.
    #[default]
    Permanent,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CleanSummary {
    pub freed: Bytes,
    pub freed_allocated: Bytes,
    pub removed: usize,
    pub failed: usize,
    pub errors: Vec<String>,
    pub mode: Option<DeleteMode>,
    pub session_id: Option<String>,
}

impl CleanSummary {
    pub fn record_error(&mut self, msg: impl Into<String>) {
        self.failed += 1;
        self.errors.push(msg.into());
    }

    /// Space the run actually gave back, on-disk sense. Same rule as
    /// [`CleanupItem::on_disk`]: never `max()` of the two, that is how a
    /// 256 GB Mac ended up reporting 1.1 TB reclaimed.
    pub fn freed_on_disk(&self) -> Bytes {
        if self.freed_allocated.is_zero() {
            self.freed
        } else {
            self.freed_allocated
        }
    }
}

/// One app discovered on the system (macOS `.app`, Linux `.desktop`, Windows
/// registry entries).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    pub bundle_id: Option<String>,
    pub path: PathBuf,
    pub version: Option<String>,
    pub size: Bytes,
    /// Where the app was found: /Applications, ~/Applications, Homebrew Cask…
    pub source: String,
    pub running: bool,
    /// Set when the same bundle id is installed more than once; uninstall must
    /// then keep shared preference files.
    pub has_sibling_install: bool,
    pub publisher: Option<String>,
    pub installed_ms: Option<u64>,
    /// Raw `DisplayIcon` registry value (Windows): a path, optionally with a
    /// `,index` suffix — the icon Explorer shows for this app.
    #[serde(default)]
    pub display_icon: Option<String>,
    /// The uninstaller command recorded by the app itself, when present.
    #[serde(default)]
    pub uninstall_string: Option<String>,
    /// Windows: the exact registry key holding this entry
    /// (`HKCU|HKLM` + subkey), so an orphaned record can be removed after
    /// its files are gone.
    #[serde(default)]
    pub reg_key: Option<String>,
    /// Install is broken: neither the install folder nor the uninstaller
    /// target exists any more (or MSI reports the product absent while the
    /// registry record remains). 「安装不完整」 — offer leftover cleanup.
    #[serde(default)]
    pub dead: bool,
}

impl AppInfo {
    pub fn key(&self) -> String {
        self.bundle_id
            .clone()
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }
}

/// A mounted volume, used by the disk views.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Volume {
    pub name: String,
    pub mount_point: PathBuf,
    pub total: Bytes,
    pub free: Bytes,
    pub is_root: bool,
    pub is_removable: bool,
    pub filesystem: String,
}

impl Volume {
    pub fn used(&self) -> Bytes {
        Bytes(self.total.0.saturating_sub(self.free.0))
    }
}

/// Leftover files found for an app bundle.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UninstallPlan {
    pub app: AppInfo,
    pub items: Vec<CleanupItem>,
    /// Notes the user should see before approving ("iCloud sync data present").
    pub warnings: Vec<String>,
}

impl UninstallPlan {
    pub fn total(&self) -> Bytes {
        self.items.iter().map(|i| i.on_disk()).sum()
    }

    pub fn push(&mut self, item: CleanupItem) {
        self.items.push(item);
    }
}

/// Cancellation hook so the TUI can interrupt long scans.
pub trait Scannable {
    fn scan(&mut self, cancel: &CancelToken) -> crate::error::Result<ScanReport>;
}

pub trait PathExt {
    fn exists_dir(&self) -> bool;
}

impl<P: AsRef<Path>> PathExt for P {
    fn exists_dir(&self) -> bool {
        self.as_ref().is_dir()
    }
}

#[cfg(test)]
mod tests {
    use super::{Category, CleanSummary, CleanupItem, ItemKind, ScanReport};
    use crate::bytes::Bytes;

    #[test]
    fn category_ids_round_trip() {
        for cat in Category::ALL {
            assert_eq!(Category::parse(cat.id()), Some(cat));
        }
    }

    #[test]
    fn category_aliases() {
        assert_eq!(Category::parse("app-caches"), Some(Category::AppCaches));
        assert_eq!(Category::parse("App_Caches"), Some(Category::AppCaches));
        assert_eq!(Category::parse("browsers"), Some(Category::BrowserCaches));
        assert_eq!(Category::parse("bogus"), None);
    }

    fn sparse_item() -> CleanupItem {
        let mut item = CleanupItem::new(
            "/var/lib/docker/Docker.raw".into(),
            ItemKind::File,
            Category::SystemJunk,
            "Docker image",
        );
        item.size = Bytes(1_024_000_000_000); // 1.024 TB of thin provisioning
        item.allocated = Bytes(12_000_000_000); // 12 GB actually written
        item
    }

    /// The 1.1 TB bug: `max(logical, allocated)` turned a 12 GB Docker image
    /// into more free space than a 256 GB disk can hold.
    #[test]
    fn an_item_is_worth_what_it_occupies_not_what_it_claims() {
        let item = sparse_item();
        assert_eq!(item.on_disk(), Bytes(12_000_000_000));
        assert_eq!(item.shown(true), Bytes(12_000_000_000));
        assert_eq!(item.shown(false), item.size);
    }

    #[test]
    fn a_zero_allocation_is_read_the_way_the_platform_means_it() {
        let mut item = sparse_item();
        item.allocated = Bytes::ZERO;
        if cfg!(unix) {
            // `du -A` agrees: eight GiB of holes occupy nothing.
            assert_eq!(item.on_disk(), Bytes::ZERO);
        } else {
            assert_eq!(item.on_disk(), item.size, "no st_blocks: fall back");
        }
    }

    #[test]
    fn a_summary_reports_the_space_that_came_back() {
        let summary = CleanSummary {
            freed: Bytes(1 << 40),
            freed_allocated: Bytes(12_000_000_000),
            ..CleanSummary::default()
        };
        assert_eq!(summary.freed_on_disk(), Bytes(12_000_000_000));
        let summary = CleanSummary {
            freed: Bytes(4096),
            ..CleanSummary::default()
        };
        assert_eq!(summary.freed_on_disk(), Bytes(4096));
    }
    /// The same 1.1 TB bug one level up: a report that totals `item.size`
    /// told the user a scan had found more junk than the disk is large, while
    /// the badge beside it — which totals `on_disk()` — said 34 GB.
    #[test]
    fn a_report_totals_what_the_items_occupy() {
        let report = ScanReport {
            items: vec![sparse_item(), sparse_item()],
            ..ScanReport::default()
        };
        assert_eq!(report.total(), Bytes(24_000_000_000));
        assert_eq!(report.total_all(), Bytes(24_000_000_000));
        assert!(
            report.total().as_u64() < 1 << 35,
            "two 12 GB images are not terabytes"
        );
    }
}
