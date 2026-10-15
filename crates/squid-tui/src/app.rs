//! Application state and input handling.
//!
//! The mental model is intentionally small: a panel is a *list of rows*, a row
//! is either a category header or one [`CleanupItem`], and the only mutation a
//! user can make is "selected / not selected". Everything destructive goes
//! through a confirmation dialog and then through [`squid_engine::delete`],
//! which re-validates every path.

use crate::job::{self, JobHandle};
use crate::theme::Strings;
use squid_core::ledger::Session;
use squid_core::progress::{CancelToken, Event, Progress};
use squid_core::{
    AppInfo, Bytes, Category, CleanSummary, CleanupItem, DeleteMode, ScanReport, UninstallPlan,
    Volume,
};
use squid_engine::analyze::Node;
use squid_engine::delete::DeleteJob;
use squid_engine::delete::DeleteOutcome;
use squid_engine::Engine;
use squid_engine::{analyze, clean, delete, purge, uninstall};
use squid_stats::{Sampler, SystemSnapshot, TrendStore};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Panel {
    Cleanup,
    Uninstall,
    Purge,
    Storage,
    Stats,
    History,
}

impl Panel {
    pub const ALL: [Panel; 6] = [
        Panel::Cleanup,
        Panel::Uninstall,
        Panel::Purge,
        Panel::Storage,
        Panel::Stats,
        Panel::History,
    ];

    pub fn next(self) -> Panel {
        let index = Panel::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Panel::ALL[(index + 1) % Panel::ALL.len()]
    }

    pub fn prev(self) -> Panel {
        let index = Panel::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Panel::ALL[(index + Panel::ALL.len() - 1) % Panel::ALL.len()]
    }

    pub fn title(self, strings: &Strings) -> String {
        match self {
            Panel::Cleanup => strings.cleanup(),
            Panel::Uninstall => strings.uninstall(),
            Panel::Purge => strings.purge(),
            Panel::Storage => strings.storage(),
            Panel::Stats => strings.system(),
            Panel::History => strings.history(),
        }
    }
}

/// A row in a scan list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    Group(Category),
    Item(usize),
}

#[derive(Default)]
pub struct ScanPanel {
    pub items: Vec<CleanupItem>,
    pub lines: Vec<Line>,
    pub collapsed: HashSet<Category>,
    pub cursor: usize,
    pub dirty: bool,
    pub scanned_at: Option<u64>,
    pub buckets: HashMap<Category, (Bytes, usize)>,
    pub summary: Option<CleanSummary>,
    pub skipped: Vec<(PathBuf, String)>,
}

impl ScanPanel {
    /// Re-derive the per-category totals from the items that are actually on
    /// the list.
    ///
    /// The buckets arrive with the scan, but the list can shrink underneath
    /// them: a delete prunes the items it removed, and every group header that
    /// still printed its pre-delete size made the run look like it had done
    /// nothing. Same arithmetic as `clean::publish_buckets`, so a fresh scan
    /// and a recompute agree.
    pub fn sync_buckets(&mut self) {
        if self.items.is_empty() && self.scanned_at.is_none() {
            // An empty list is also what a scan looks like before the first
            // bucket event lands; that is not "everything went to zero". Once
            // a scan has actually happened, an empty list *is* the total.
            return;
        }
        let mut totals: HashMap<Category, (Bytes, usize)> = HashMap::new();
        for item in &self.items {
            let entry = totals.entry(item.category).or_insert((Bytes::ZERO, 0));
            entry.0 += item.on_disk();
            entry.1 += 1;
        }
        self.buckets = totals;
    }

    pub fn rebuild_lines(&mut self) {
        self.sync_buckets();
        let mut order: Vec<Category> = Category::ALL.to_vec();
        order.sort_by(|a, b| {
            let total_of = |category: &Category| -> u64 {
                self.items
                    .iter()
                    .filter(|item| item.category == *category)
                    .map(|item| item.on_disk().as_u64())
                    .sum()
            };
            total_of(b).cmp(&total_of(a))
        });
        let mut lines = Vec::new();
        for category in order {
            let mut indexes: Vec<usize> = (0..self.items.len())
                .filter(|index| self.items[*index].category == category)
                .collect();
            if indexes.is_empty() {
                continue;
            }
            indexes.sort_by(|a, b| {
                let left = self.items[*a].on_disk();
                let right = self.items[*b].on_disk();
                right.cmp(&left)
            });
            lines.push(Line::Group(category));
            if !self.collapsed.contains(&category) {
                lines.extend(indexes.into_iter().map(Line::Item));
            }
        }
        self.lines = lines;
        self.cursor = self.cursor.min(self.lines.len().saturating_sub(1));
        self.dirty = false;
    }

    pub fn selected_size(&self) -> Bytes {
        self.items
            .iter()
            .filter(|item| item.selected && item.lock.is_none())
            .map(|item| item.on_disk())
            .sum()
    }

    pub fn selected_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.selected && item.lock.is_none())
            .count()
    }

    pub fn total_size(&self) -> Bytes {
        self.items.iter().map(|item| item.on_disk()).sum()
    }

    pub fn toggle_at(&mut self, line: usize) {
        match self.lines.get(line) {
            Some(Line::Item(index)) => {
                if let Some(item) = self.items.get_mut(*index) {
                    if item.lock.is_none() {
                        item.selected = !item.selected;
                    }
                }
            }
            Some(Line::Group(category)) => {
                let category = *category;
                let target = !self
                    .items
                    .iter()
                    .filter(|item| item.category == category && item.lock.is_none())
                    .all(|item| item.selected);
                for item in self
                    .items
                    .iter_mut()
                    .filter(|item| item.category == category && item.lock.is_none())
                {
                    item.selected = target;
                }
            }
            None => {}
        }
    }

    pub fn toggle_all(&mut self) {
        let target = !self
            .items
            .iter()
            .filter(|item| item.lock.is_none())
            .all(|item| item.selected);
        for item in self.items.iter_mut() {
            if item.lock.is_none() {
                item.selected = target;
            }
        }
    }

    pub fn collapse_at(&mut self, line: usize, open: bool) {
        if let Some(Line::Group(category)) = self.lines.get(line).copied() {
            if open {
                self.collapsed.remove(&category);
            } else {
                self.collapsed.insert(category);
            }
            self.dirty = true;
        }
    }

    pub fn load(&mut self, report: ScanReport) {
        self.items = report.items;
        self.scanned_at = Some(report.scanned_at_ms);
        self.dirty = true;
        self.rebuild_lines();
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.lines.clear();
        self.buckets.clear();
        self.cursor = 0;
        self.dirty = false;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UninstallView {
    #[default]
    List,
    Plan,
}

#[derive(Default)]
pub struct UninstallPanel {
    pub apps: Vec<AppInfo>,
    pub filter: String,
    pub cursor: usize,
    pub view: UninstallView,
    pub plan: Option<UninstallPlan>,
    pub lines: Vec<Line>,
    pub plan_cursor: usize,
    pub dirty: bool,
    pub summary: Option<CleanSummary>,
}

impl UninstallPanel {
    pub fn visible(&self) -> Vec<usize> {
        let needle = self.filter.trim().to_lowercase();
        (0..self.apps.len())
            .filter(|index| {
                if needle.is_empty() {
                    true
                } else {
                    let app = &self.apps[*index];
                    app.name.to_lowercase().contains(&needle)
                        || app
                            .bundle_id
                            .as_deref()
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(&needle)
                }
            })
            .collect()
    }

    pub fn selected_app(&self) -> Option<AppInfo> {
        let visible = self.visible();
        let raw = *visible.get(self.cursor)?;
        self.apps.get(raw).cloned()
    }

    pub fn set_plan(&mut self, plan: UninstallPlan) {
        self.plan = Some(plan);
        self.view = UninstallView::Plan;
        self.plan_cursor = 0;
        self.rebuild_lines();
    }

    pub fn rebuild_lines(&mut self) {
        let Some(plan) = &self.plan else {
            self.lines.clear();
            self.dirty = false;
            return;
        };
        self.lines = (0..plan.items.len()).map(Line::Item).collect();
        self.plan_cursor = self.plan_cursor.min(self.lines.len().saturating_sub(1));
        self.dirty = false;
    }

    /// Tick or untick one line of the plan. Returns why the key did nothing,
    /// so the caller can say so instead of swallowing the press.
    pub fn toggle_plan_at(&mut self, line: usize) -> Option<&'static str> {
        let plan = self.plan.as_mut()?;
        let mut refused = None;
        match self.lines.get(line) {
            Some(Line::Item(index)) => {
                if let Some(item) = plan.items.get_mut(*index) {
                    // The bundle itself is the uninstall; keep it immovable.
                    let is_bundle = item.rule_id == "uninstall-bundle";
                    if is_bundle {
                        refused = Some("bundle");
                    } else if item.lock.is_some() {
                        refused = Some("locked");
                    } else {
                        item.selected = !item.selected;
                    }
                }
            }
            Some(Line::Group(_)) => {
                let target = !plan
                    .items
                    .iter()
                    .filter(|item| item.lock.is_none() && item.rule_id != "uninstall-bundle")
                    .all(|item| item.selected);
                for item in plan.items.iter_mut() {
                    if item.lock.is_none() && item.rule_id != "uninstall-bundle" {
                        item.selected = target;
                    }
                }
            }
            None => {}
        }
        refused
    }

    /// `a` inside a plan: tick or untick every leftover. The bundle keeps its
    /// permanent tick — "all" never means "keep the application".
    pub fn toggle_all_leftovers(&mut self) {
        let Some(plan) = self.plan.as_mut() else {
            return;
        };
        let target = !plan
            .items
            .iter()
            .filter(|item| item.lock.is_none() && item.rule_id != "uninstall-bundle")
            .all(|item| item.selected);
        for item in plan.items.iter_mut() {
            if item.lock.is_none() && item.rule_id != "uninstall-bundle" {
                item.selected = target;
            }
        }
    }

    pub fn plan_selected_size(&self) -> Bytes {
        let Some(plan) = &self.plan else {
            return Bytes::ZERO;
        };
        plan.items
            .iter()
            .filter(|item| item.selected && item.lock.is_none())
            .map(|item| item.on_disk())
            .sum()
    }

    pub fn remove_deleted(&mut self, removed_paths: &[PathBuf]) {
        self.apps
            .retain(|app| !removed_paths.iter().any(|path| path == &app.path));
        self.plan = None;
        self.view = UninstallView::List;
        self.lines.clear();
        // The filter existed to find the app that is now gone. Leaving it set
        // is how a finished uninstall reads as an empty, dead panel.
        self.filter.clear();
        self.cursor = self.cursor.min(self.visible().len().saturating_sub(1));
    }
}

#[derive(Default)]
pub struct StoragePanel {
    pub stack: Vec<Node>,
    pub cursor: usize,
    pub files: Vec<squid_engine::analyze::FileEntry>,
    pub show_files: bool,
    pub volumes: Vec<Volume>,
    pub root: PathBuf,
    pub scanned: bool,
}

impl StoragePanel {
    pub fn current(&self) -> Option<&Node> {
        self.stack.last()
    }

    pub fn rows(&self) -> usize {
        match self.current() {
            Some(node) => node.children.len() + usize::from(node.folded > 0),
            None => 0,
        }
    }

    pub fn child_at(&self, index: usize) -> Option<Node> {
        let node = self.current()?;
        node.display_children().into_iter().nth(index)
    }
}

pub struct StatsPanel {
    pub sampler: Sampler,
    pub snapshot: Option<SystemSnapshot>,
    pub trends: TrendStore,
    pub cursor: usize,
    pub last_refresh: Instant,
}

impl StatsPanel {
    pub fn new() -> StatsPanel {
        StatsPanel {
            sampler: Sampler::new(),
            snapshot: None,
            trends: TrendStore::default(),
            cursor: 0,
            last_refresh: Instant::now(),
        }
    }

    pub fn refresh(&mut self) {
        let snapshot = if self.snapshot.is_none() {
            self.sampler.prime()
        } else {
            self.sampler.refresh()
        };
        self.trends.record(&snapshot);
        self.snapshot = Some(snapshot);
        self.last_refresh = Instant::now();
    }
}

impl Default for StatsPanel {
    fn default() -> Self {
        StatsPanel::new()
    }
}

#[derive(Default)]
pub struct HistoryPanel {
    pub sessions: Vec<Session>,
    pub cursor: usize,
}

impl HistoryPanel {
    pub fn reload(&mut self) {
        // The same window the header totals are built from, so the session
        // count in the header cannot change under the user just because the
        // panel refreshed.
        self.sessions = squid_core::ledger::load_sessions(200);
        self.cursor = self.cursor.min(self.sessions.len().saturating_sub(1));
    }
}

#[derive(Clone, Debug)]
pub enum ConfirmAction {
    Clean,
    Purge,
    Uninstall,
    UndoSession(String),
}

/// What the user approved, carrying everything needed to carry it out.
///
/// This used to be two sibling fields — `pending` (items) and `pending_plan`
/// (the app) — and nothing ever filled the plan one. Approving an uninstall
/// therefore fell straight through a `let-else` and the keypress vanished:
/// dialog closed, no toast, no log, app still installed. One enum makes the
/// "uninstall approved but the plan is missing" state unrepresentable.
#[derive(Clone, Debug)]
// The item list is boxed where the approval is stored; the variants stay
// intentionally plain data for the dialog code.
#[allow(clippy::large_enum_variant)]
pub enum Approval {
    Items {
        items: Vec<CleanupItem>,
        purge: bool,
    },
    Plan(UninstallPlan),
    UndoSession(String),
}

#[derive(Clone, Debug)]
pub struct Confirm {
    pub title: String,
    pub lines: Vec<String>,
    pub action: ConfirmAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Good,
    Warn,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub until: Instant,
}

/// Everything that can be running at once. Only one job at a time is allowed,
/// which keeps the progress bar honest.
#[derive(Default)]
pub struct Jobs {
    pub clean: Option<JobHandle<ScanReport>>,
    pub purge: Option<JobHandle<ScanReport>>,
    pub apps: Option<JobHandle<Vec<AppInfo>>>,
    pub plan: Option<JobHandle<UninstallPlan>>,
    pub storage: Option<JobHandle<Node>>,
    pub files: Option<JobHandle<Vec<analyze::FileEntry>>>,
    pub delete: Option<JobHandle<DeleteOutcome>>,
    pub delete_target: Option<Panel>,
}

impl Jobs {
    pub fn any(&self) -> bool {
        self.clean.is_some()
            || self.purge.is_some()
            || self.apps.is_some()
            || self.plan.is_some()
            || self.storage.is_some()
            || self.files.is_some()
            || self.delete.is_some()
    }

    /// The work the user asked for with a key. One of these runs at a time, so
    /// the progress bar always answers to the key that was pressed.
    pub fn foreground(&self) -> bool {
        self.clean.is_some()
            || self.purge.is_some()
            || self.apps.is_some()
            || self.plan.is_some()
            || self.files.is_some()
            || self.delete.is_some()
    }

    pub fn occupied(&self, slot: Slot) -> bool {
        match slot {
            Slot::Clean => self.clean.is_some(),
            Slot::Purge => self.purge.is_some(),
            Slot::Apps => self.apps.is_some(),
            Slot::Plan => self.plan.is_some(),
            Slot::Storage => self.storage.is_some(),
            Slot::Files => self.files.is_some(),
            Slot::Delete => self.delete.is_some(),
        }
    }

    /// The foreground job worth naming in a refusal.
    pub fn foreground_slot(&self) -> Option<Slot> {
        [
            Slot::Delete,
            Slot::Clean,
            Slot::Purge,
            Slot::Apps,
            Slot::Plan,
            Slot::Files,
        ]
        .into_iter()
        .find(|&slot| self.occupied(slot))
    }
}

/// One keyboard entry point, one slot. Refusal is decided per slot, never
/// "is anything at all running": a disk map of a home folder takes tens of
/// seconds and used to hold the whole interface hostage, so the second
/// uninstall of a session answered "the previous task is still running".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Clean,
    Purge,
    Apps,
    Plan,
    Storage,
    Files,
    Delete,
}

pub struct App {
    pub engine: Engine,
    pub strings: Strings,
    pub panel: Panel,
    pub should_quit: bool,
    pub jobs: Jobs,
    pub progress: Option<Progress>,
    pub cancel: Option<CancelToken>,
    pub phase: String,
    pub done: u64,
    pub total: Option<u64>,
    pub subject: Option<PathBuf>,
    pub log: VecDeque<String>,
    pub clean: ScanPanel,
    pub purge: ScanPanel,
    pub uninstall: UninstallPanel,
    pub storage: StoragePanel,
    pub stats: StatsPanel,
    pub history: HistoryPanel,
    pub confirm: Option<Confirm>,
    pub toast: Option<Toast>,
    pub help: bool,
    pub filtering: bool,
    pub reclaimed: Bytes,
    /// The part of `reclaimed` that is only *in the Trash*, and therefore still
    /// occupying the volume until the Trash is emptied.
    pub to_trash: Bytes,
    pub sessions: usize,
    pub delete_mode: DeleteMode,
    /// What the dialog approved, waiting for `confirm_delete` to carry it out.
    pub pending: Option<Approval>,
    /// Set while a storage drill-down is running so the result is pushed,
    /// not swapped in.
    pub pending_drill: bool,
    /// The disk walk running *beside* the job the progress bar belongs to.
    /// Drained quietly: it may never take over the bar or the phase line.
    pub bg: Option<Progress>,
    /// Keyboard focus. `false` means the arrows drive the list, `true` means
    /// they drive the six entries in the sidebar.
    pub menu: bool,
    /// The panel a running scan is filling. Streamed buckets belong to the
    /// panel whose key started the walk, not to whichever panel happens to be
    /// on screen when one arrives.
    pub scan_owner: Option<Panel>,
}

/// `(all removed, still in the Trash)` over a stretch of the ledger.
///
/// Undone runs count for neither: [`Session::kept`] is the only honest summand.
fn totals(sessions: &[squid_core::ledger::Session]) -> (Bytes, Bytes) {
    let mut all = Bytes::ZERO;
    let mut trash = Bytes::ZERO;
    for session in sessions {
        let kept = session.kept();
        all += kept;
        if session.mode == DeleteMode::Trash {
            trash += kept;
        }
    }
    (all, trash)
}

impl App {
    pub fn new() -> squid_core::Result<App> {
        let engine = Engine::new();
        let strings = Strings::detect(engine.settings());
        let delete_mode = engine.settings().delete_mode;
        // The walk starts where the user's own bytes live. Rooting it at `/`
        // made the first thing the panel does a trawl across two million
        // inodes — minutes of "computing…" before a single bar — and most of
        // that tree is not something this panel can clean anyway.
        // `squid analyze` already defaults to the home folder; the panel agrees
        // with it, and its heading always names the folder being measured.
        let storage_root = squid_core::paths::home_dir();
        let sessions = squid_core::ledger::load_sessions(200);
        let (reclaimed, to_trash) = totals(&sessions);
        let mut app = App {
            engine,
            strings,
            panel: Panel::Cleanup,
            should_quit: false,
            jobs: Jobs::default(),
            progress: None,
            cancel: None,
            phase: String::new(),
            done: 0,
            total: None,
            subject: None,
            log: VecDeque::new(),
            clean: ScanPanel::default(),
            purge: ScanPanel::default(),
            uninstall: UninstallPanel::default(),
            storage: StoragePanel {
                root: storage_root,
                ..StoragePanel::default()
            },
            stats: StatsPanel::new(),
            history: HistoryPanel::default(),
            confirm: None,
            toast: None,
            help: false,
            filtering: false,
            reclaimed,
            to_trash,
            sessions: sessions.len(),
            delete_mode,
            pending: None,
            pending_drill: false,
            bg: None,
            // The interface opens where the choice is made. A list of junk
            // files is not the first thing anyone should have to ignore before
            // they can say which of the six functions they want; the arrows
            // start on the sidebar and `esc` hands them to the list.
            menu: true,
            scan_owner: None,
        };
        app.push_log(format!(
            "squid {} · {}",
            env!("CARGO_PKG_VERSION"),
            app.engine.platform().display_name()
        ));
        for error in app.engine.settings_errors().to_vec() {
            app.push_log(format!("config: {error}"));
        }
        // Ask the terminal what it can paint *now*, before a single cell is
        // drawn, rather than finding out from the first card that came out
        // wrong. Anything short of 24-bit falls back to the 256-colour palette
        // and a terminal with no palette at all to attributes alone.
        let colours = crate::theme::init();
        app.push_log(app.strings.t(
            &format!("colour · {}", colours.label()),
            &format!("色彩 · {}", colours.label()),
        ));
        // And which switch chose the language: one binary that is Chinese in
        // one terminal and English in another is a report nobody can act on
        // without knowing where the answer came from.
        let (locale, source) = squid_core::Locale::explain(app.engine.settings());
        app.push_log(app.strings.t(
            &format!("language · {} ({})", locale.code(), source.describe()),
            &format!("语言 · {}（{}）", locale.code(), source.describe_zh()),
        ));
        app.storage.volumes = app.engine.platform().volumes().unwrap_or_default();
        app.history.reload();
        app.open_menu();
        Ok(app)
    }

    /// Refusing to start a second job while one is running is right; refusing
    /// *silently* — or refusing on behalf of an unrelated job — is how a user
    /// ends up pressing return twice with nothing on screen. Every long-running
    /// entry point asks about *its own* slot and says what it found.
    fn busy_refusal(&mut self, slot: Slot) -> bool {
        if !self.slot_busy(slot) {
            return false;
        }
        let blocker = self
            .jobs
            .occupied(slot)
            .then_some(slot)
            .or_else(|| self.jobs.foreground_slot())
            .unwrap_or(slot);
        let name = self.slot_name(blocker);
        self.toast(
            self.strings.t(
                &format!("{name} is still running — wait, or press c to cancel"),
                &format!("{name}还在进行，请稍候，或按 c 取消"),
            ),
            ToastKind::Warn,
        );
        true
    }

    /// Can this slot start right now? The disk walk is the one job allowed to
    /// run beside a foreground job: it is read-only, it is what the storage
    /// panel needs, and refusing everything else while it re-walks a home
    /// folder is what looked like the interface locking up.
    fn slot_busy(&self, slot: Slot) -> bool {
        self.jobs.occupied(slot) || (slot != Slot::Storage && self.jobs.foreground())
    }

    fn slot_name(&self, slot: Slot) -> String {
        let (en, zh) = match slot {
            Slot::Clean => ("clean scan", "清理扫描"),
            Slot::Purge => ("deep clean scan", "深度清理扫描"),
            Slot::Apps => ("app list", "应用列表"),
            Slot::Plan => ("leftover search", "残留搜索"),
            Slot::Storage => ("disk scan", "磁盘扫描"),
            Slot::Files => ("large-file scan", "大文件扫描"),
            Slot::Delete => ("deletion", "删除"),
        };
        self.strings.t(en, zh).to_string()
    }

    pub fn busy(&self) -> bool {
        self.jobs.any()
    }

    /// Is the work that fills this panel still on its way?
    ///
    /// An empty panel has two opposite reasons for being empty — the list is
    /// still being read, or there is genuinely nothing there — and they want
    /// opposite answers on screen. Saying "未发现应用 / 按 s 重新扫描" while the
    /// inventory is still being read sends the user to a key that does nothing,
    /// which reads as the interface ignoring them.
    pub fn filling(&self, panel: Panel) -> bool {
        App::filling_slot(panel)
            .map(|slot| self.jobs.occupied(slot))
            .unwrap_or(false)
    }

    /// Which job, if any, is the one that populates this panel.
    fn filling_slot(panel: Panel) -> Option<Slot> {
        match panel {
            Panel::Cleanup => Some(Slot::Clean),
            Panel::Purge => Some(Slot::Purge),
            Panel::Uninstall => Some(Slot::Apps),
            Panel::Storage => Some(Slot::Storage),
            Panel::Stats | Panel::History => None,
        }
    }

    pub fn push_log(&mut self, text: impl Into<String>) {
        let text = text.into();
        if self.log.back().map(|last| last == &text).unwrap_or(false) {
            return;
        }
        if self.log.len() >= 200 {
            self.log.pop_front();
        }
        self.log.push_back(text);
    }

    pub fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toast = Some(Toast {
            text: text.into(),
            kind,
            until: Instant::now() + std::time::Duration::from_secs(4),
        });
    }

    /// Called ~4 times a second: drain events, collect finished jobs, refresh
    /// the meters.
    pub fn tick(&mut self) {
        if let Some(progress) = self.progress.clone() {
            for event in progress.drain() {
                self.apply_event(event);
            }
        }
        if let Some(progress) = self.bg.clone() {
            for event in progress.drain() {
                self.apply_bg_event(event);
            }
        }
        self.poll_jobs();
        if self.panel == Panel::Stats
            && (self.stats.snapshot.is_none()
                || self.stats.last_refresh.elapsed().as_secs_f64() > 1.0)
        {
            self.stats.refresh();
        }
        if let Some(toast) = &self.toast {
            if toast.until < Instant::now() {
                self.toast = None;
            }
        }
    }

    fn apply_event(&mut self, event: Event) {
        match event {
            Event::Phase { phase, detail } => {
                self.phase = format!("{phase} · {detail}");
                self.push_log(format!("{phase}: {detail}"));
            }
            Event::Tick {
                done,
                total,
                subject,
            } => {
                self.done = done;
                if total.is_some() {
                    self.total = total;
                }
                self.subject = subject;
            }
            Event::Bucket {
                category,
                total,
                count,
            } => {
                let panel = self.scan_owner.unwrap_or(self.panel);
                let panel = match panel {
                    Panel::Purge => &mut self.purge,
                    _ => &mut self.clean,
                };
                panel.buckets.insert(category, (total, count));
            }
            Event::Deleted { path, freed } => {
                self.push_log(format!(
                    "removed {} ({})",
                    squid_core::paths::pretty(&path),
                    freed.human()
                ));
            }
            Event::Note(text) => self.push_log(text),
            Event::Warning(text) => {
                let clipped = crate::theme::truncate(&text, 160, false);
                self.push_log(format!("! {clipped}"));
                self.toast(clipped, ToastKind::Warn);
            }
            Event::Done(report) => {
                let panel = self.scan_owner.unwrap_or(self.panel);
                let panel = match panel {
                    Panel::Purge => &mut self.purge,
                    _ => &mut self.clean,
                };
                panel.load(report);
            }
        }
    }

    /// A background disk walk shares the log but not the progress bar: its
    /// phases belong to nobody, so only its notes and warnings are kept.
    fn apply_bg_event(&mut self, event: Event) {
        match event {
            Event::Note(text) => self.push_log(text),
            Event::Warning(text) => {
                let clipped = crate::theme::truncate(&text, 160, false);
                self.push_log(format!("! {clipped}"));
            }
            _ => {}
        }
    }

    fn poll_jobs(&mut self) {
        if let Some(handle) = &self.jobs.clean {
            if let Some(result) = handle.poll() {
                self.jobs.clean = None;
                self.scan_owner = None;
                self.finish_job();
                self.apply_report(Panel::Cleanup, result);
            }
        }
        if let Some(handle) = &self.jobs.purge {
            if let Some(result) = handle.poll() {
                self.jobs.purge = None;
                self.scan_owner = None;
                self.finish_job();
                self.apply_report(Panel::Purge, result);
            }
        }
        if let Some(handle) = &self.jobs.apps {
            if let Some(result) = handle.poll() {
                self.jobs.apps = None;
                self.finish_job();
                match result {
                    Ok(apps) => {
                        let count = apps.len();
                        self.uninstall.apps = apps;
                        self.uninstall.cursor = 0;
                        self.toast(
                            self.strings.t(
                                &format!("{count} applications found"),
                                &format!("发现 {count} 个应用"),
                            ),
                            ToastKind::Good,
                        );
                    }
                    Err(err) => self.toast(err, ToastKind::Warn),
                }
            }
        }
        if let Some(handle) = &self.jobs.plan {
            if let Some(result) = handle.poll() {
                self.jobs.plan = None;
                self.finish_job();
                match result {
                    Ok(plan) => {
                        let items = plan.items.len();
                        let size = plan.items.iter().map(|item| item.on_disk()).sum::<Bytes>();
                        self.uninstall.set_plan(plan);
                        self.toast(
                            format!(
                                "{} · {items} · {}",
                                self.strings.t("leftovers found", "发现残留"),
                                size.human()
                            ),
                            ToastKind::Good,
                        );
                    }
                    Err(err) => self.toast(err, ToastKind::Warn),
                }
            }
        }
        if let Some(handle) = &self.jobs.storage {
            if let Some(result) = handle.poll() {
                self.jobs.storage = None;
                self.bg = None;
                self.finish_job();
                match result {
                    Ok(node) => {
                        if self.pending_drill {
                            self.pending_drill = false;
                            self.storage.stack.push(node);
                        } else {
                            self.storage.stack = vec![node];
                        }
                        self.storage.cursor = 0;
                        self.storage.scanned = true;
                        self.toast(self.strings.t("scan complete", "扫描完成"), ToastKind::Good);
                    }
                    Err(err) => self.toast(err, ToastKind::Warn),
                }
            }
        }
        if let Some(handle) = &self.jobs.files {
            if let Some(result) = handle.poll() {
                self.jobs.files = None;
                self.finish_job();
                match result {
                    Ok(files) => {
                        self.storage.files = files;
                        self.storage.show_files = true;
                    }
                    Err(err) => self.toast(err, ToastKind::Warn),
                }
            }
        }
        if let Some(handle) = &self.jobs.delete {
            if let Some(result) = handle.poll() {
                let target = self.jobs.delete_target.take().unwrap_or(Panel::Cleanup);
                self.jobs.delete = None;
                self.finish_job();
                match result {
                    Ok(outcome) => self.apply_deletion(target, outcome),
                    Err(err) => self.toast(err, ToastKind::Warn),
                }
            }
        }
    }

    /// Re-read the ledger and recompute the two figures the header prints.
    ///
    /// They were counters: `+= freed` after a delete, and nothing at all after
    /// an undo, so putting 24 GB back left the header promising it forever. The
    /// ledger is the only thing that knows what actually happened, so both
    /// numbers come from it — which also refreshes the history panel with the
    /// run that just finished instead of waiting for `r`.
    pub fn recount(&mut self) {
        self.history.reload();
        let (reclaimed, to_trash) = totals(&self.history.sessions);
        self.reclaimed = reclaimed;
        self.to_trash = to_trash;
        self.sessions = self.history.sessions.len();
    }

    fn finish_job(&mut self) {
        if self.jobs.foreground() {
            return;
        }
        // Nothing the user asked for is left. A disk walk that was running
        // beside it may as well use the bar now that it is free.
        if let Some(progress) = self.bg.take() {
            self.progress = Some(progress);
            self.phase = "storage".to_string();
            self.done = 0;
            self.total = None;
            self.subject = None;
            return;
        }
        // A disk walk can still be going, and it must not keep the last job's
        // gauge on screen: a bar that reads "delete · 4/4" after the toast
        // said "已删除" is indistinguishable from a wedged interface.
        self.progress = None;
        self.cancel = None;
        self.phase.clear();
        self.total = None;
        self.done = 0;
        self.subject = None;
    }

    fn apply_report(&mut self, panel: Panel, result: Result<ScanReport, String>) {
        match result {
            Ok(report) => {
                let items = self.strings.count(report.items.len());
                let size = report.total_all();
                let target = if panel == Panel::Purge {
                    &mut self.purge
                } else {
                    &mut self.clean
                };
                target.load(report);
                self.toast(
                    format!(
                        "{} · {}",
                        self.strings
                            .t(&format!("{items} found"), &format!("发现 {items}")),
                        size.human_long()
                    ),
                    ToastKind::Good,
                );
            }
            Err(err) => self.toast(err, ToastKind::Warn),
        }
    }

    fn apply_deletion(&mut self, target: Panel, outcome: DeleteOutcome) {
        let summary = outcome.summary.clone();
        for (path, reason) in &outcome.skipped {
            self.push_log(format!(
                "skipped {} — {reason}",
                squid_core::paths::pretty(path)
            ));
        }
        let freed = summary.freed_on_disk();
        // The worker wrote the session before it returned, so the ledger is
        // already the truth; read it rather than second-guessing it here.
        self.recount();
        let removed_count = self.strings.count(summary.removed);
        // 「已删除」 over a run that only renamed files into ~/.Trash is the same
        // lie the header used to tell, one screen earlier and one second
        // before the user checked `df`.
        let trashed = summary.mode == Some(DeleteMode::Trash);
        let message = format!(
            "{} · {}",
            if trashed {
                self.strings.bin_t(
                    &format!("{removed_count} to {{bin}}"),
                    &format!("移入{{bin}} {removed_count}"),
                )
            } else {
                self.strings.t(
                    &format!("{removed_count} removed"),
                    &format!("已删除 {removed_count}"),
                )
            },
            freed.human_fixed()
        );
        // A partial run must not look like a clean success: the bug that hid
        // "the .app was never deleted" was a green toast counting only the
        // leftovers, because a *skipped* item is not counted as a failure.
        let skipped = outcome.skipped.len();
        let toast = if skipped == 0 {
            message.clone()
        } else {
            format!(
                "{message} · {}",
                self.strings.t(
                    &format!("{skipped} skipped"),
                    &format!("{skipped} 项被跳过")
                )
            )
        };
        self.toast(
            toast,
            if summary.failed == 0 && skipped == 0 {
                ToastKind::Good
            } else {
                ToastKind::Warn
            },
        );
        self.push_log(message);
        for error in summary.errors.iter().take(5) {
            self.push_log(format!("! {error}"));
        }
        let removed: Vec<PathBuf> = self.removed_paths(&summary);
        match target {
            Panel::Cleanup => {
                self.clean
                    .items
                    .retain(|item| !removed.contains(&item.path));
                self.clean.summary = Some(summary.clone());
                self.clean.dirty = true;
                self.clean.rebuild_lines();
            }
            Panel::Purge => {
                self.purge
                    .items
                    .retain(|item| !removed.contains(&item.path));
                self.purge.summary = Some(summary.clone());
                self.purge.dirty = true;
                self.purge.rebuild_lines();
            }
            Panel::Uninstall => {
                self.uninstall.remove_deleted(&removed);
                self.uninstall.summary = Some(summary.clone());
            }
            _ => {}
        }
        self.storage.volumes = self.engine.platform().volumes().unwrap_or_default();
        self.refresh_storage_after(&removed);
        // A cleanup that really removed something just invalidated every number
        // on the panel it came from, so scan it again — the same job `s` starts.
        // Pruning the list by hand only hides the deleted rows; it cannot know
        // that the cache directory rebuilt itself, or that a rule now finds a
        // different set of files.
        if summary.removed > 0 && matches!(target, Panel::Cleanup | Panel::Purge) {
            self.push_log(
                self.strings
                    .t("rescanning after the delete", "清理完成，正在重新扫描"),
            );
            self.scan_panel(target, false);
        }
    }

    /// Keep the disk map honest after a delete without holding the interface
    /// hostage: the refresh runs beside whatever the user does next, it is
    /// skipped while another walk is already going, and it never throws away a
    /// drill-down the user is reading.
    fn refresh_storage_after(&mut self, removed: &[PathBuf]) {
        if !self.storage.scanned {
            return;
        }
        let touches_map = removed.iter().any(|path| {
            path.starts_with(&self.storage.root) || self.storage.root.starts_with(path)
        });
        if !touches_map {
            return;
        }
        if self.storage.stack.len() > 1 {
            self.push_log(self.strings.t(
                "disk map is stale — press r to rescan",
                "磁盘占用待刷新 — 按 r 重新扫描",
            ));
            return;
        }
        if self.jobs.storage.is_some() {
            return;
        }
        self.start_storage_scan();
    }

    fn removed_paths(&self, summary: &CleanSummary) -> Vec<PathBuf> {
        summary
            .session_id
            .as_deref()
            .and_then(squid_core::ledger::load_session)
            .map(|session| {
                session
                    .entries
                    .into_iter()
                    .map(|entry| entry.path)
                    .collect()
            })
            .unwrap_or_default()
    }

    // ---------------------------------------------------------------- actions

    pub fn scan(&mut self) {
        self.scan_panel(self.panel, true);
    }

    /// Start the scan for one panel. `ask` is what makes the `s` key say "one
    /// is already running" out loud, while a rescan the interface starts for
    /// itself after a delete stays silent and simply steps aside.
    fn scan_panel(&mut self, panel: Panel, ask: bool) {
        match panel {
            Panel::Cleanup => {
                if self.slot_busy(Slot::Clean) {
                    if ask {
                        self.busy_refusal(Slot::Clean);
                    }
                    return;
                }
                let (handle, progress) = job::spawn("clean", |engine, progress, cancel| {
                    let report =
                        clean::scan(engine, &clean::CleanOptions::default(), progress, cancel)?;
                    clean::publish_buckets(&report, progress);
                    Ok(clean::sort_for_display(report))
                });
                self.jobs.clean = Some(handle);
                self.scan_owner = Some(Panel::Cleanup);
                self.clean.clear();
                self.start_progress(progress, "clean");
            }
            Panel::Purge => {
                if self.slot_busy(Slot::Purge) {
                    if ask {
                        self.busy_refusal(Slot::Purge);
                    }
                    return;
                }
                let (handle, progress) = job::spawn("purge", |engine, progress, cancel| {
                    let report =
                        purge::scan(engine, &purge::PurgeOptions::default(), progress, cancel)?;
                    let mut report = report;
                    report.items.sort_by(|a, b| {
                        b.on_disk()
                            .cmp(&a.on_disk())
                            .then_with(|| a.path.cmp(&b.path))
                    });
                    Ok(report)
                });
                self.jobs.purge = Some(handle);
                self.scan_owner = Some(Panel::Purge);
                self.purge.clear();
                self.start_progress(progress, "purge");
            }
            Panel::Uninstall => {
                if self.slot_busy(Slot::Apps) {
                    if ask {
                        self.busy_refusal(Slot::Apps);
                    }
                    return;
                }
                let (handle, progress) = job::spawn("apps", |engine, progress, cancel| {
                    uninstall::scan_apps(engine, true, progress, cancel)
                });
                self.jobs.apps = Some(handle);
                self.start_progress(progress, "apps");
            }
            Panel::Storage => self.rescan_storage(),
            Panel::Stats => self.stats.refresh(),
            Panel::History => self.history.reload(),
        }
    }

    /// Hand the bar to a job and say what it is doing.
    fn set_bar(&mut self, progress: Progress, label: &str) {
        self.progress = Some(progress);
        self.phase = label.to_string();
        self.done = 0;
        self.total = None;
        self.subject = None;
        self.push_log(format!("· {label}"));
    }

    /// Hand the bar to a job the user asked for with a key.
    ///
    /// A disk walk that was holding the bar steps aside into [`App::bg`]
    /// instead of losing its handle: dropping it here is what left a finished
    /// delete printing its gauge, because the walk kept `jobs.any()` true and
    /// the gauge only cleared when *everything* had stopped.
    fn start_progress(&mut self, progress: Progress, label: &str) {
        if self.jobs.storage.is_some() && self.bg.is_none() && self.progress.is_some() {
            self.bg = self.progress.take();
        }
        self.set_bar(progress, label);
    }

    pub fn rescan_storage(&mut self) {
        if self.busy_refusal(Slot::Storage) {
            return;
        }
        self.start_storage_scan();
    }

    /// Start the walk without asking anyone. When the user is already waiting
    /// on a job the progress bar stays theirs and the walk reports to the log.
    fn start_storage_scan(&mut self) {
        let root = self.storage.root.clone();
        let (handle, progress) = job::spawn("storage", move |engine, progress, cancel| {
            let _ = engine;
            analyze::explore(&root, 1, 24, cancel, progress)
        });
        self.jobs.storage = Some(handle);
        if self.jobs.foreground() {
            self.bg = Some(progress);
            self.push_log("· storage");
        } else {
            self.set_bar(progress, "storage");
        }
    }

    pub fn drill_storage(&mut self) {
        if self.busy_refusal(Slot::Storage) {
            return;
        }
        let Some(node) = self.storage.child_at(self.storage.cursor) else {
            return;
        };
        if !node.is_dir {
            self.toast(
                self.strings.t("not a folder", "不是文件夹"),
                ToastKind::Info,
            );
            return;
        }
        let path = node.path.clone();
        let (handle, progress) = job::spawn("storage", move |engine, progress, cancel| {
            let _ = engine;
            analyze::explore(&path, 1, 24, cancel, progress)
        });
        self.jobs.storage = Some(handle);
        if self.jobs.foreground() {
            self.bg = Some(progress);
            self.push_log("· storage");
        } else {
            self.set_bar(progress, "storage");
        }
        self.storage.cursor = 0;
        self.pending_drill = true;
    }

    pub fn up_storage(&mut self) {
        if self.pending_drill {
            self.abandon(Slot::Storage);
        }
        if self.storage.stack.len() > 1 {
            self.storage.stack.pop();
            self.storage.cursor = 0;
        }
    }

    pub fn largest_files(&mut self) {
        if self.busy_refusal(Slot::Files) {
            return;
        }
        let root = self
            .storage
            .current()
            .map(|node| node.path.clone())
            .unwrap_or_else(|| self.storage.root.clone());
        let (handle, progress) = job::spawn("files", move |engine, progress, cancel| {
            let _ = engine;
            analyze::largest_files(
                &root,
                60,
                squid_core::Bytes(20 * 1024 * 1024),
                cancel,
                progress,
            )
        });
        self.jobs.files = Some(handle);
        self.start_progress(progress, "files");
    }

    pub fn build_plan(&mut self) {
        if self.busy_refusal(Slot::Plan) {
            return;
        }
        let Some(app) = self.uninstall.selected_app() else {
            self.toast(
                self.strings.t("pick an app first", "先选择一个应用"),
                ToastKind::Info,
            );
            return;
        };
        let name = app.name.clone();
        let (handle, progress) = job::spawn("plan", move |engine, progress, cancel| {
            uninstall::build_plan(engine, &app, progress, cancel)
        });
        self.jobs.plan = Some(handle);
        self.start_progress(progress, &format!("plan · {name}"));
    }

    pub fn ask_delete(&mut self) {
        if self.confirm.is_some() || self.busy_refusal(Slot::Delete) {
            return;
        }
        // The dialog heading is the *verb the user is approving*, once. The
        // first version prefixed every title with "Clean", so an uninstall
        // asked for permission to "Clean · Uninstall".
        let (items, action, title) = match self.panel {
            Panel::Cleanup => (
                self.clean.items.clone(),
                ConfirmAction::Clean,
                self.strings.t("Clean now", "立即清理"),
            ),
            Panel::Purge => (
                self.purge.items.clone(),
                ConfirmAction::Purge,
                self.strings.t("Deep clean now", "立即深度清理"),
            ),
            Panel::Uninstall => match &self.uninstall.plan {
                Some(plan) => (
                    plan.items.clone(),
                    ConfirmAction::Uninstall,
                    format!("{} · {}", self.strings.uninstall(), plan.app.name),
                ),
                None => {
                    self.toast(
                        self.strings.t(
                            "press enter to inspect an app first",
                            "先按回车检查某个应用",
                        ),
                        ToastKind::Info,
                    );
                    return;
                }
            },
            _ => {
                self.toast(
                    self.strings
                        .t("nothing to clean here", "这里没有可清理的内容"),
                    ToastKind::Info,
                );
                return;
            }
        };
        let chosen: Vec<CleanupItem> = items
            .into_iter()
            .filter(|item| item.selected && item.lock.is_none())
            .collect();
        if chosen.is_empty() {
            self.toast(
                self.strings
                    .t("select something first", "先勾选要删除的内容"),
                ToastKind::Info,
            );
            return;
        }
        let size: Bytes = chosen.iter().map(|item| item.on_disk()).sum();
        let mode = if self.delete_mode == DeleteMode::Trash {
            self.strings.bin_t("moved to {bin}", "移入{bin}")
        } else {
            self.strings.t("deleted permanently", "永久删除")
        };
        let biggest = chosen
            .iter()
            .take(6)
            .map(|item| format!("  {}", item.display_path()))
            .collect::<Vec<_>>();
        let mut lines = vec![
            format!(
                "{} · {}",
                self.strings.t(
                    &format!("{} items", chosen.len()),
                    &format!("{} 个项目", chosen.len())
                ),
                size.human_long()
            ),
            self.strings
                .t(&format!("Mode: {mode}"), &format!("方式：{mode}")),
        ];
        lines.extend(biggest);
        let plan = self.uninstall.plan.clone();
        let approval = match &action {
            ConfirmAction::Uninstall => match plan {
                Some(plan) => Approval::Plan(UninstallPlan {
                    items: chosen.clone(),
                    ..plan
                }),
                None => {
                    self.toast(
                        self.strings.t(
                            "the plan is gone, press enter again",
                            "清单已失效，请重新按回车",
                        ),
                        ToastKind::Warn,
                    );
                    return;
                }
            },
            ConfirmAction::Purge => Approval::Items {
                items: chosen,
                purge: true,
            },
            ConfirmAction::Clean => Approval::Items {
                items: chosen,
                purge: false,
            },
            ConfirmAction::UndoSession(_) => return,
        };
        self.pending = Some(approval);
        self.confirm = Some(Confirm {
            title,
            lines,
            action,
        });
    }
}

// ------------------------------------------------------------------ commands
//
// The dialog is the only gate between "selected" and "deleted", and even then
// `squid_engine::delete` re-checks every path against the safety policy.

impl App {
    /// Delete what the confirmation dialog was showing.
    pub fn confirm_delete(&mut self) {
        if self.slot_busy(Slot::Delete) {
            // Keep the dialog and the approval: the user said yes, the machine
            // is just not free yet. Dropping them here is what made the
            // original report read "确定之后什么都没有发生".
            self.toast(
                self.strings.t(
                    "still working — press enter again when it is done",
                    "上一个任务还在进行，等它结束后再按一次回车",
                ),
                ToastKind::Warn,
            );
            return;
        }
        let Some(approval) = self.pending.take() else {
            self.confirm = None;
            self.toast(
                self.strings.t(
                    "nothing was approved — press d again",
                    "没有待确认的内容，请重新按 d",
                ),
                ToastKind::Warn,
            );
            return;
        };
        self.confirm = None;
        let mode = self.delete_mode;
        match approval {
            Approval::Items { items, purge } => {
                let command = if purge { "squid purge" } else { "squid clean" }.to_string();
                let (handle, progress) = job::spawn("delete", move |engine, progress, cancel| {
                    let job = DeleteJob::new(items, mode, command);
                    delete::execute(engine, &job, progress, cancel)
                });
                self.jobs.delete = Some(handle);
                self.jobs.delete_target = Some(if purge { Panel::Purge } else { Panel::Cleanup });
                self.start_progress(progress, "delete");
            }
            Approval::Plan(plan) => {
                let name = plan.app.name.clone();
                let (handle, progress) = job::spawn("delete", move |engine, progress, cancel| {
                    uninstall::remove_plan(engine, &plan, mode, progress, cancel)
                });
                self.jobs.delete = Some(handle);
                self.jobs.delete_target = Some(Panel::Uninstall);
                self.start_progress(progress, &format!("uninstall · {name}"));
            }
            Approval::UndoSession(id) => match delete::restore_session(&id) {
                Ok((restored, skipped)) => {
                    let text = format!(
                        "{} · {} skipped",
                        self.strings.t(
                            &format!("restored {restored} items"),
                            &format!("已还原 {restored} 项")
                        ),
                        skipped
                    );
                    self.toast(text.clone(), ToastKind::Good);
                    self.push_log(text);
                    // The bytes came back, so the header must stop counting
                    // them; `restore` marked the record as undone for exactly
                    // this reason.
                    self.recount();
                }
                Err(err) => {
                    self.toast(err.to_string(), ToastKind::Warn);
                }
            },
        }
    }

    pub fn deny(&mut self) {
        self.confirm = None;
        self.pending = None;
    }

    pub fn toggle_mode(&mut self) {
        self.delete_mode = match self.delete_mode {
            DeleteMode::Trash => DeleteMode::Permanent,
            DeleteMode::Permanent => DeleteMode::Trash,
        };
        let text = match self.delete_mode {
            DeleteMode::Trash => self.strings.bin().to_string(),
            DeleteMode::Permanent => self.strings.t("Permanent", "永久删除"),
        };
        self.toast(
            self.strings.t(
                &format!("Delete mode: {text}"),
                &format!("删除方式：{text}"),
            ),
            ToastKind::Info,
        );
    }

    pub fn undo_last(&mut self) {
        let Some(session) = self
            .history
            .sessions
            .iter()
            .find(|session| !session.entries.is_empty())
            .or_else(|| self.history.sessions.first())
            .cloned()
        else {
            self.toast(
                self.strings.t("nothing to undo", "没有可还原的记录"),
                ToastKind::Info,
            );
            return;
        };
        self.ask_undo(session.id.clone());
    }

    pub fn ask_undo(&mut self, id: String) {
        if self.confirm.is_some() {
            return;
        }
        // A restore walks the same folders a delete is walking.
        if self.busy_refusal(Slot::Delete) {
            return;
        }
        let session = self
            .history
            .sessions
            .iter()
            .find(|item| item.id == id)
            .cloned();
        let lines = match session {
            Some(session) => vec![
                format!("{} · {}", session.command, session.freed().human_long()),
                self.strings.t(
                    &format!(
                        "{} items will come back from {{bin}}",
                        session.entries.len()
                    ),
                    &format!("{} 项将从{{bin}}还原", session.entries.len()),
                ),
            ],
            None => vec![id.clone()],
        };
        self.pending = Some(Approval::UndoSession(id.clone()));
        self.confirm = Some(Confirm {
            title: self.strings.t("Undo this session", "还原此会话"),
            lines,
            action: ConfirmAction::UndoSession(id),
        });
    }

    // ------------------------------------------------------------- navigation

    pub fn list_len(&self) -> usize {
        match self.panel {
            Panel::Cleanup => self.clean.lines.len(),
            Panel::Purge => self.purge.lines.len(),
            Panel::Uninstall => match self.uninstall.view {
                UninstallView::List => self.uninstall.visible().len(),
                UninstallView::Plan => self.uninstall.lines.len(),
            },
            Panel::Storage => {
                if self.storage.show_files {
                    self.storage.files.len()
                } else {
                    self.storage.rows()
                }
            }
            Panel::Stats => self
                .stats
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.top_processes.len())
                .unwrap_or(0),
            Panel::History => self.history.sessions.len(),
        }
    }

    fn cursor(&self) -> usize {
        match self.panel {
            Panel::Cleanup => self.clean.cursor,
            Panel::Purge => self.purge.cursor,
            Panel::Uninstall => match self.uninstall.view {
                UninstallView::List => self.uninstall.cursor,
                UninstallView::Plan => self.uninstall.plan_cursor,
            },
            Panel::Storage => self.storage.cursor,
            Panel::Stats => self.stats.cursor,
            Panel::History => self.history.cursor,
        }
    }

    fn set_cursor(&mut self, value: usize) {
        match self.panel {
            Panel::Cleanup => self.clean.cursor = value,
            Panel::Purge => self.purge.cursor = value,
            Panel::Uninstall => match self.uninstall.view {
                UninstallView::List => self.uninstall.cursor = value,
                UninstallView::Plan => self.uninstall.plan_cursor = value,
            },
            Panel::Storage => self.storage.cursor = value,
            Panel::Stats => self.stats.cursor = value,
            Panel::History => self.history.cursor = value,
        }
    }

    pub fn move_cursor(&mut self, delta: i32) {
        let len = self.list_len();
        if len == 0 {
            self.set_cursor(0);
            return;
        }
        let current = self.cursor().min(len - 1);
        let next = if delta < 0 {
            current.saturating_sub((-delta) as usize)
        } else {
            (current + delta as usize).min(len - 1)
        };
        self.set_cursor(next);
    }

    pub fn page_cursor(&mut self, step: isize) {
        let len = self.list_len();
        if len == 0 {
            return;
        }
        let current = self.cursor() as isize;
        let next = (current + step).clamp(0, len as isize - 1) as usize;
        self.set_cursor(next);
    }

    pub fn jump_end(&mut self) {
        let len = self.list_len();
        self.set_cursor(len.saturating_sub(1));
    }

    pub fn toggle(&mut self) {
        let cursor = self.cursor();
        match self.panel {
            Panel::Cleanup => self.clean.toggle_at(cursor),
            Panel::Purge => self.purge.toggle_at(cursor),
            Panel::Uninstall => match self.uninstall.view {
                UninstallView::List => self.toast(
                    self.strings.t(
                        "space toggles items, enter inspects an app",
                        "空格勾选，回车查看应用",
                    ),
                    ToastKind::Info,
                ),
                UninstallView::Plan => match self.uninstall.toggle_plan_at(cursor) {
                    Some("bundle") => self.toast(
                        self.strings.t(
                            "the application itself is always removed",
                            "应用本体始终会被删除",
                        ),
                        ToastKind::Info,
                    ),
                    Some(reason) => self.toast(
                        self.strings.t(
                            &format!("this item is locked ({reason})"),
                            &format!("这一项被锁定了（{reason}）"),
                        ),
                        ToastKind::Info,
                    ),
                    None => {}
                },
            },
            Panel::Storage | Panel::Stats | Panel::History => {}
        }
    }

    pub fn toggle_all(&mut self) {
        match self.panel {
            Panel::Cleanup => self.clean.toggle_all(),
            Panel::Purge => self.purge.toggle_all(),
            Panel::Uninstall => match self.uninstall.view {
                UninstallView::List => {}
                UninstallView::Plan => self.uninstall.toggle_all_leftovers(),
            },
            _ => {}
        }
    }

    pub fn collapse(&mut self) {
        let cursor = self.cursor();
        match self.panel {
            Panel::Cleanup => self.clean.collapse_at(cursor, false),
            Panel::Purge => self.purge.collapse_at(cursor, false),
            Panel::Uninstall => {
                if self.uninstall.view == UninstallView::Plan {
                    self.leave_plan();
                }
            }
            // Out of the largest-files list first, and only then out of the
            // folder: the two are one level apart, not one gesture apart.
            Panel::Storage => {
                if self.storage.show_files {
                    self.leave_files();
                } else {
                    self.up_storage();
                }
            }
            _ => {}
        }
    }

    pub fn expand(&mut self) {
        let cursor = self.cursor();
        match self.panel {
            Panel::Cleanup => self.clean.collapse_at(cursor, true),
            Panel::Purge => self.purge.collapse_at(cursor, true),
            Panel::Storage => self.drill_storage(),
            _ => {}
        }
    }

    /// Enter: the one obvious action for whatever is under the cursor.
    pub fn primary(&mut self) {
        match self.panel {
            Panel::Cleanup | Panel::Purge => {
                let scan = match self.panel {
                    Panel::Cleanup => &self.clean,
                    _ => &self.purge,
                };
                if scan.items.is_empty() {
                    self.scan();
                    return;
                }
                let cursor = self.cursor();
                match scan.lines.get(cursor).copied() {
                    Some(Line::Group(_)) | None => self.toggle(),
                    Some(Line::Item(_)) => self.toggle(),
                }
            }
            Panel::Uninstall => match self.uninstall.view {
                UninstallView::List => self.build_plan(),
                UninstallView::Plan => self.ask_delete(),
            },
            Panel::Storage => {
                if self.storage.show_files {
                    self.toast(
                        self.strings
                            .t("files are read-only here", "此处文件仅供查看"),
                        ToastKind::Info,
                    );
                } else if self.storage.current().is_none() {
                    self.rescan_storage();
                } else {
                    self.drill_storage();
                }
            }
            Panel::Stats => {}
            Panel::History => {
                let Some(session) = self.history.sessions.get(self.history.cursor).cloned() else {
                    self.history.reload();
                    return;
                };
                self.ask_undo(session.id);
            }
        }
    }

    pub fn cancel_jobs(&mut self) {
        if let Some(job) = &self.jobs.clean {
            job.cancel();
        }
        if let Some(job) = &self.jobs.purge {
            job.cancel();
        }
        if let Some(job) = &self.jobs.apps {
            job.cancel();
        }
        if let Some(job) = &self.jobs.plan {
            job.cancel();
        }
        if let Some(job) = &self.jobs.storage {
            job.cancel();
        }
        if let Some(job) = &self.jobs.files {
            job.cancel();
        }
        if let Some(job) = &self.jobs.delete {
            job.cancel();
            self.push_log("cancelling delete…");
        }
    }

    pub fn select_panel(&mut self, panel: Panel) {
        if panel == self.panel {
            return;
        }
        self.panel = panel;
        self.help = false;
        self.filtering = false;
        match panel {
            Panel::Uninstall if self.uninstall.apps.is_empty() && !self.slot_busy(Slot::Apps) => {
                self.scan()
            }
            Panel::Storage if self.storage.stack.is_empty() && !self.slot_busy(Slot::Storage) => {
                self.rescan_storage()
            }
            Panel::Stats if self.stats.snapshot.is_none() => self.stats.refresh(),
            Panel::History => self.history.reload(),
            _ => {}
        }
    }

    pub fn cycle(&mut self, forward: bool) {
        let next = if forward {
            self.panel.next()
        } else {
            self.panel.prev()
        };
        self.select_panel(next);
    }

    // ------------------------------------------------------------------ keys

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Action {
        if modifiers.contains(KeyModifiers::CONTROL) && matches!(code, KeyCode::Char('c')) {
            return Action::Quit;
        }
        if self.filtering {
            return self.handle_filter_key(code);
        }
        if self.confirm.is_some() {
            return match code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    self.confirm_delete();
                    Action::Redraw
                }
                // The dialog promises 「回车 / y 确认 · n / esc 取消」 on screen.
                // `esc` used to land in the catch-all below, which redrew and
                // left the dialog standing: the way out of a second-level
                // confirmation was to press yes. `q` closes it too rather than
                // quitting the app from inside a question.
                KeyCode::Char('n')
                | KeyCode::Char('N')
                | KeyCode::Esc
                | KeyCode::Backspace
                | KeyCode::Char('q')
                | KeyCode::Left => {
                    self.deny();
                    self.toast(self.strings.t("cancelled", "已取消"), ToastKind::Warn);
                    Action::Redraw
                }
                _ => Action::Redraw,
            };
        }
        if self.help {
            return match code {
                KeyCode::Esc
                | KeyCode::Char('?')
                | KeyCode::Char('h')
                | KeyCode::Char('q')
                | KeyCode::Enter => {
                    self.help = false;
                    Action::Redraw
                }
                _ => Action::Redraw,
            };
        }
        if self.menu {
            return self.handle_menu_key(code);
        }
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => Action::Quit,
            KeyCode::Tab => {
                self.cycle(true);
                Action::Redraw
            }
            KeyCode::BackTab => {
                self.cycle(false);
                Action::Redraw
            }
            KeyCode::Backspace => {
                if self.panel == Panel::Storage {
                    self.up_storage();
                }
                Action::Redraw
            }
            KeyCode::Char(digit @ '1'..='6') => {
                let index = (digit as u8 - b'1') as usize;
                if let Some(panel) = Panel::ALL.get(index).copied() {
                    self.select_panel(panel);
                }
                Action::Redraw
            }
            // Option/Control + arrow drives the six functions from anywhere,
            // because `esc` first is one keystroke too many when you are only
            // hopping between panels. Plain arrows stay with the list.
            code @ (KeyCode::Up | KeyCode::Down)
                if modifiers.intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
            {
                self.step_panel(if code == KeyCode::Down { 1 } else { -1 });
                Action::Redraw
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_cursor(1);
                Action::Redraw
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_cursor(-1);
                Action::Redraw
            }
            KeyCode::PageDown => {
                self.page_cursor(15);
                Action::Redraw
            }
            KeyCode::PageUp => {
                self.page_cursor(-15);
                Action::Redraw
            }
            KeyCode::Home => {
                self.set_cursor(0);
                Action::Redraw
            }
            KeyCode::End => {
                self.jump_end();
                Action::Redraw
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.collapse();
                Action::Redraw
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.expand();
                Action::Redraw
            }
            KeyCode::Char(' ') => {
                self.toggle();
                Action::Redraw
            }
            KeyCode::Enter => {
                self.primary();
                Action::Redraw
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.scan();
                Action::Redraw
            }
            KeyCode::Char('d') | KeyCode::Char('x') => {
                self.ask_delete();
                Action::Redraw
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.toggle_all();
                Action::Redraw
            }
            KeyCode::Char('f') => {
                if self.panel == Panel::Uninstall {
                    self.filtering = true;
                    self.toast(
                        self.strings
                            .t("filter apps — type to search", "筛选应用 — 直接输入"),
                        ToastKind::Info,
                    );
                } else if self.panel == Panel::Storage {
                    // The hint bar has been advertising `f files` on the storage
                    // panel all along; the key itself did nothing at all.
                    self.largest_files();
                }
                Action::Redraw
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                match self.panel {
                    Panel::History => self.history.reload(),
                    Panel::Storage => {
                        self.storage.show_files = false;
                        self.rescan_storage();
                    }
                    Panel::Stats => self.stats.refresh(),
                    Panel::Cleanup | Panel::Purge => self.rescan_storage(),
                    Panel::Uninstall => {}
                }
                Action::Redraw
            }
            KeyCode::Char('m') => {
                self.toggle_mode();
                Action::Redraw
            }
            KeyCode::Char('u') | KeyCode::Char('U') => {
                if self.panel == Panel::History {
                    if let Some(session) = self.history.sessions.get(self.history.cursor).cloned() {
                        self.ask_undo(session.id);
                    }
                } else {
                    self.undo_last();
                }
                Action::Redraw
            }
            KeyCode::Char('c') if self.busy() => {
                self.cancel_jobs();
                self.toast(self.strings.t("cancelling…", "正在取消…"), ToastKind::Warn);
                Action::Redraw
            }
            KeyCode::Esc => {
                self.escape();
                Action::Redraw
            }
            KeyCode::Char('?') => {
                self.help = true;
                Action::Redraw
            }
            _ => Action::Redraw,
        }
    }

    /// `esc` walks back exactly **one** step, because every one of these views
    /// is a level the user walked into and expects to walk out of the same way:
    ///
    /// ```text
    /// confirm a delete  →  the view it was asked from
    /// uninstall plan    →  the list of applications
    /// largest files     →  the folder they were listed from
    /// a folder          →  the folder above it
    /// the list          →  the six functions in the sidebar
    /// ```
    ///
    /// It used to leap two levels at once on the disk map (files *and* a folder)
    /// and to give up entirely inside a confirmation. A long-running job must
    /// still never steal the key — a background disk walk used to swallow every
    /// `esc` for minutes; `c` is what cancels.
    ///
    /// The other half of the same complaint is the job that finishes *after*
    /// the key: leaving a level also throws away the work that level was
    /// waiting on (see [`App::abandon`]), or the view reopens itself and `esc`
    /// looks dead.
    fn escape(&mut self) {
        self.pending = None;
        if self.menu {
            self.menu = false;
            return;
        }
        match self.panel {
            Panel::Uninstall if self.uninstall.view == UninstallView::Plan => self.leave_plan(),
            Panel::Storage => {
                if self.storage.show_files {
                    self.leave_files();
                } else if self.pending_drill {
                    // The folder was asked for and never arrived; `esc` is the
                    // answer to that request, not a step sideways.
                    self.abandon(Slot::Storage);
                } else if self.storage.stack.len() > 1 {
                    self.up_storage();
                } else {
                    self.open_menu();
                }
            }
            _ => self.open_menu(),
        }
    }

    /// Leave the plan of an application and hand the list back.
    fn leave_plan(&mut self) {
        self.uninstall.view = UninstallView::List;
        self.uninstall.plan = None;
        self.uninstall.lines.clear();
        self.uninstall.plan_cursor = 0;
        self.abandon(Slot::Plan);
    }

    /// Leave the largest-files list of a folder, second level of the disk map.
    fn leave_files(&mut self) {
        self.storage.show_files = false;
        self.storage.cursor = 0;
        self.abandon(Slot::Files);
    }

    /// Throw away the work of a level the user has just walked out of.
    ///
    /// Backing this: a job that finished after `esc` used to repaint the view
    /// the key had just left — the leftover plan appeared again, the file list
    /// came back, the folder you had escaped from opened itself — which is
    /// precisely what "esc does nothing in the second level" looks like from the
    /// chair. The worker is told to stop and its handle dropped, so nothing is
    /// applied and nothing is toasted about a job the user already abandoned.
    fn abandon(&mut self, slot: Slot) {
        match slot {
            Slot::Plan => {
                if let Some(handle) = self.jobs.plan.take() {
                    handle.cancel();
                }
            }
            Slot::Files => {
                if let Some(handle) = self.jobs.files.take() {
                    handle.cancel();
                }
            }
            Slot::Storage => {
                self.pending_drill = false;
                if let Some(handle) = self.jobs.storage.take() {
                    handle.cancel();
                }
            }
            _ => return,
        }
        self.finish_job();
    }

    /// Give the arrow keys to the six entries in the sidebar. The list keeps
    /// its own scroll position, so leaving and coming back is harmless, and
    /// `q` still quits from here.
    fn open_menu(&mut self) {
        self.menu = true;
        self.toast(
            self.strings.t(
                "menu — up and down to choose, enter to go back",
                "功能菜单 — 上下键选择，回车返回",
            ),
            ToastKind::Info,
        );
    }

    /// Move the sidebar selection. Clamped rather than wrapping: the list is
    /// short and knowing where the end is matters more than the loop.
    fn step_panel(&mut self, delta: i32) {
        let last = Panel::ALL.len().saturating_sub(1);
        let current = Panel::ALL
            .iter()
            .position(|panel| *panel == self.panel)
            .unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, last as i32) as usize;
        if let Some(panel) = Panel::ALL.get(next).copied() {
            self.select_panel(panel);
        }
    }

    fn handle_menu_key(&mut self, code: KeyCode) -> Action {
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => Action::Quit,
            KeyCode::Char('?') => {
                self.help = true;
                Action::Redraw
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.step_panel(-1);
                Action::Redraw
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.step_panel(1);
                Action::Redraw
            }
            KeyCode::PageUp => {
                self.step_panel(-3);
                Action::Redraw
            }
            KeyCode::PageDown => {
                self.step_panel(3);
                Action::Redraw
            }
            KeyCode::Home => {
                if let Some(panel) = Panel::ALL.first().copied() {
                    self.select_panel(panel);
                }
                Action::Redraw
            }
            KeyCode::End => {
                if let Some(panel) = Panel::ALL.last().copied() {
                    self.select_panel(panel);
                }
                Action::Redraw
            }
            KeyCode::Char(digit @ '1'..='6') => {
                let index = (digit as u8 - b'1') as usize;
                if let Some(panel) = Panel::ALL.get(index).copied() {
                    self.select_panel(panel);
                }
                Action::Redraw
            }
            KeyCode::Tab => {
                self.cycle(true);
                Action::Redraw
            }
            KeyCode::BackTab => {
                self.cycle(false);
                Action::Redraw
            }
            KeyCode::Char('c') if self.busy() => {
                self.cancel_jobs();
                self.toast(self.strings.t("cancelling…", "正在取消…"), ToastKind::Warn);
                Action::Redraw
            }
            KeyCode::Char('m') => {
                self.toggle_mode();
                Action::Redraw
            }
            KeyCode::Enter
            | KeyCode::Esc
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char('h')
            | KeyCode::Char('l') => {
                self.menu = false;
                Action::Redraw
            }
            // Everything that is not the arrows is an *order*, not a choice of
            // where the arrows go. `s` on the very first screen used to be
            // swallowed here — the interface now opens on the sidebar, so the
            // key the hint bar advertises would have done nothing until the
            // user found `esc`. Hand the keys back to the panel and let it
            // answer them, the way it did before the sidebar had focus.
            code => {
                self.menu = false;
                self.handle_key(code, KeyModifiers::empty())
            }
        }
    }

    fn handle_filter_key(&mut self, code: KeyCode) -> Action {
        match code {
            KeyCode::Esc => {
                self.filtering = false;
                self.uninstall.filter.clear();
                self.uninstall.cursor = 0;
                Action::Redraw
            }
            KeyCode::Enter => {
                self.filtering = false;
                Action::Redraw
            }
            KeyCode::Backspace => {
                self.uninstall.filter.pop();
                self.uninstall.cursor = 0;
                Action::Redraw
            }
            KeyCode::Char(ch) if !ch.is_control() => {
                self.uninstall.filter.push(ch);
                self.uninstall.cursor = 0;
                Action::Redraw
            }
            _ => Action::Redraw,
        }
    }
}

/// What the event loop should do after a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Redraw,
    Quit,
}

use crossterm::event::{KeyCode, KeyModifiers};

#[cfg(test)]
mod tests {
    use super::*;
    use squid_core::ItemKind;
    use std::path::Path;
    use std::time::{Duration, Instant};

    /// The ledger is user data: point it at a scratch directory so a test
    /// session never shows up in the user's real `squid history`.
    fn isolate_state() {
        use std::sync::Once;
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let dir = std::env::temp_dir().join(format!("squid-tui-state-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            std::env::set_var("SQUID_STATE_DIR", dir);
        });
    }

    /// A sandbox under `~/Library/Caches`, the way the engine tests do it:
    /// that is inside the cleanup anchors *and* inside the home directory, so
    /// the safety policy treats these paths exactly like real junk it was
    /// asked to delete — no special cases needed.
    fn scratch(tag: &str) -> PathBuf {
        let dir = squid_core::paths::home_dir()
            .join("Library/Caches")
            .join(format!(
                "squid-tui-test-{}-{tag}-{}",
                std::process::id(),
                squid_core::ledger::new_session_id()
            ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn fake_app(dir: &Path) -> (AppInfo, Vec<CleanupItem>) {
        let bundle = dir.join("SquidTest.app");
        let binary = bundle.join("Contents/MacOS/squid-test");
        std::fs::create_dir_all(binary.parent().expect("parent")).expect("bundle");
        std::fs::write(&binary, b"mach-o-ish").expect("binary");
        let prefs = dir.join("Preferences/com.nexsquid.test.plist");
        std::fs::create_dir_all(prefs.parent().expect("prefs dir")).expect("prefs dir");
        std::fs::write(&prefs, b"defaults").expect("prefs");

        let info = AppInfo {
            name: "SquidTest".to_string(),
            bundle_id: Some("com.nexsquid.test".to_string()),
            path: bundle.clone(),
            version: Some("1.0".to_string()),
            size: Bytes(0),
            source: "test".to_string(),
            running: false,
            has_sibling_install: false,
            publisher: None,
            installed_ms: None,
        };
        let items = vec![
            CleanupItem::new(
                bundle.clone(),
                ItemKind::Path,
                Category::Leftovers,
                "SquidTest",
            ),
            CleanupItem::new(
                prefs.clone(),
                ItemKind::File,
                Category::Leftovers,
                "preferences",
            ),
        ];
        (info, items)
    }

    /// The dialog is the only gate before a delete, so approving it has to
    /// actually reach the engine. This is the regression test for the version
    /// where an approved uninstall closed the dialog and did nothing at all.
    #[test]
    fn approving_an_uninstall_really_uninstalls() {
        isolate_state();
        let dir = scratch("uninstall");
        let (info, items) = fake_app(&dir);
        let bundle = info.path.clone();
        let prefs = items[1].path.clone();

        let mut app = App::new().expect("app");
        app.delete_mode = DeleteMode::Permanent;
        app.panel = Panel::Uninstall;
        app.uninstall.view = UninstallView::Plan;
        app.uninstall.plan = Some(UninstallPlan {
            app: info,
            items,
            warnings: Vec::new(),
        });

        app.ask_delete();
        assert!(app.confirm.is_some(), "d must open the confirmation dialog");
        assert!(
            matches!(app.pending, Some(Approval::Plan(_))),
            "the dialog must carry the plan it is approving"
        );

        app.confirm_delete();
        let handle = app
            .jobs
            .delete
            .take()
            .expect("confirming must start a delete job");
        let started = Instant::now();
        let outcome = loop {
            if let Some(value) = handle.poll() {
                break value.expect("delete job");
            }
            assert!(
                started.elapsed() < Duration::from_secs(60),
                "the delete job never reported back"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(
            outcome.summary.removed, 2,
            "errors={:?} skipped={:?}",
            outcome.summary.errors, outcome.skipped
        );
        assert!(!bundle.exists(), "the bundle should be gone");
        assert!(
            !prefs.exists(),
            "the leftover preference file should be gone"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// After the app is gone the panel must not stay filtered down to the one
    /// app that used to match — that read as "nothing happened".
    #[test]
    fn a_finished_uninstall_leaves_the_list_readable() {
        let mut app = App::new().expect("app");
        let path = std::path::PathBuf::from("/Applications/SquidTest.app");
        app.uninstall.apps = vec![AppInfo {
            name: "SquidTest".into(),
            bundle_id: Some("com.example.SquidTest".into()),
            path: path.clone(),
            version: None,
            size: Bytes(0),
            source: "test".into(),
            running: false,
            has_sibling_install: false,
            publisher: None,
            installed_ms: None,
        }];
        app.uninstall.filter = "Squid".into();
        assert_eq!(app.uninstall.visible().len(), 1);

        app.uninstall.remove_deleted(&[path]);

        assert!(
            app.uninstall.filter.is_empty(),
            "the filter that found the deleted app must be cleared"
        );
        assert!(app.uninstall.apps.is_empty());
        assert_eq!(app.uninstall.view, UninstallView::List);
    }

    /// Deleting from the Cleanup panel shares the same code path, and the
    /// `UndoSession`/`Items` arms must not be able to cross wires.
    #[test]
    fn approving_a_cleaning_dialog_carries_the_items() {
        isolate_state();
        let dir = scratch("clean");
        let file = dir.join("junk.bin");
        std::fs::write(&file, vec![3u8; 2048]).expect("junk");

        let mut app = App::new().expect("app");
        app.delete_mode = DeleteMode::Permanent;
        app.panel = Panel::Cleanup;
        let mut item = CleanupItem::new(file.clone(), ItemKind::File, Category::SystemJunk, "junk");
        item.size = Bytes(2048);
        app.clean.items = vec![item];

        app.ask_delete();
        assert!(matches!(
            app.pending,
            Some(Approval::Items { purge: false, .. })
        ));
        app.confirm_delete();
        let handle = app.jobs.delete.take().expect("a delete job");
        let started = Instant::now();
        let outcome = loop {
            if let Some(value) = handle.poll() {
                break value.expect("delete job");
            }
            assert!(started.elapsed() < Duration::from_secs(60));
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(
            outcome.summary.removed, 1,
            "errors={:?} skipped={:?}",
            outcome.summary.errors, outcome.skipped
        );
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Nothing approved, nothing deleted — and the user hears about it rather
    /// than watching the dialog disappear in silence.
    #[test]
    fn confirming_without_an_approval_says_so() {
        isolate_state();
        let mut app = App::new().expect("app");
        app.confirm = Some(Confirm {
            title: String::new(),
            lines: Vec::new(),
            action: ConfirmAction::Uninstall,
        });
        app.confirm_delete();
        assert!(app.jobs.delete.is_none(), "nothing should have started");
        assert!(app.toast.is_some(), "the user must get feedback");
    }

    /// A job that is still running swallows keys. That part is correct — a
    /// second scan while one is walking the disk would be worse — but it has
    /// to say so, because "pressed d, nothing happened" is indistinguishable
    /// from "the button is broken".
    #[test]
    fn a_busy_app_answers_instead_of_ignoring_you() {
        isolate_state();
        let mut app = App::new().expect("app");
        let (handle, _progress) = crate::job::spawn("test-busy", |_engine, _progress, _cancel| {
            std::thread::sleep(Duration::from_secs(30));
            Ok(Vec::<AppInfo>::new())
        });
        app.jobs.apps = Some(handle);
        assert!(app.busy());

        app.panel = Panel::Uninstall;
        app.ask_delete();
        assert!(app.toast.is_some(), "the refusal must be visible");
        assert!(app.pending.is_none(), "and nothing may be armed behind it");
        assert!(app.confirm.is_none(), "nor may a dialog open");

        app.jobs.apps = None;
    }

    /// Saying yes while the machine is still busy keeps the approval alive:
    /// the user's answer is not thrown away, it just has to wait.
    #[test]
    fn confirming_while_busy_keeps_the_answer() {
        isolate_state();
        let mut app = App::new().expect("app");
        let (handle, _progress) = crate::job::spawn("test-busy2", |_engine, _progress, _cancel| {
            std::thread::sleep(Duration::from_secs(30));
            Ok(Vec::<AppInfo>::new())
        });
        app.jobs.apps = Some(handle);
        app.pending = Some(Approval::Items {
            items: Vec::new(),
            purge: false,
        });

        app.confirm_delete();
        assert!(
            app.pending.is_some(),
            "a busy machine must not eat the user's yes"
        );
        assert!(app.toast.is_some(), "and the wait has to be explained");
    }
    /// The refusal names the job it is waiting on. "上一个任务还在进行" with no
    /// name sent users to hunt for a task that was only a disk walk.
    #[test]
    fn a_refusal_names_the_job() {
        isolate_state();
        let mut app = App::new().expect("app");
        let (handle, _progress) = crate::job::spawn("apps", |_engine, _progress, _cancel| {
            std::thread::sleep(Duration::from_secs(30));
            Ok(Vec::<AppInfo>::new())
        });
        app.jobs.apps = Some(handle);

        app.panel = Panel::Uninstall;
        app.ask_delete();
        let text = app.toast.take().expect("a reason").text;
        assert!(
            text.contains("应用列表") || text.contains("app list"),
            "the refusal must name the job: {text}"
        );

        if let Some(handle) = app.jobs.apps.take() {
            handle.cancel();
        }
    }

    fn idle_node(tag: &str) -> Node {
        let dir = scratch(tag);
        let node = analyze::explore(&dir, 0, 8, &CancelToken::new(), &Progress::new())
            .expect("measure an empty folder");
        let _ = std::fs::remove_dir_all(&dir);
        node
    }

    /// A disk map of a home folder takes tens of seconds. It used to be "a job"
    /// like any other, so the whole interface answered "上一个任务还在进行" while
    /// it walked. It is the one job allowed to run beside the one you pressed a
    /// key for.
    /// "未发现应用" while the inventory is still being read is a lie, and a lie
    /// the user can act on: they press `s`, nothing happens (the slot is busy),
    /// and the panel looks ignored. The empty state has to know whether the
    /// list is on its way.
    #[test]
    fn an_empty_panel_says_whether_it_is_still_being_filled() {
        assert_eq!(App::filling_slot(Panel::Cleanup), Some(Slot::Clean));
        assert_eq!(App::filling_slot(Panel::Uninstall), Some(Slot::Apps));
        assert_eq!(App::filling_slot(Panel::Storage), Some(Slot::Storage));
        assert_eq!(App::filling_slot(Panel::History), None);

        isolate_state();
        let mut app = App::new().expect("app");
        assert!(!app.filling(Panel::Uninstall), "at rest nothing is loading");

        let (handle, _progress) = crate::job::spawn("apps", move |_engine, _progress, _cancel| {
            std::thread::sleep(Duration::from_secs(30));
            Ok(Vec::new())
        });
        app.jobs.apps = Some(handle);
        assert!(
            app.filling(Panel::Uninstall),
            "the app list is on its way, so the panel is not empty yet"
        );
        assert!(
            !app.filling(Panel::Cleanup),
            "a job must not make every panel claim to be working"
        );
        if let Some(handle) = app.jobs.apps.take() {
            handle.cancel();
        }
    }

    #[test]
    fn a_disk_scan_leaves_the_other_panels_usable() {
        isolate_state();
        let mut app = App::new().expect("app");
        let node = idle_node("map");
        let (handle, _progress) =
            crate::job::spawn("storage", move |_engine, _progress, _cancel| {
                std::thread::sleep(Duration::from_secs(30));
                Ok(node)
            });
        app.jobs.storage = Some(handle);

        assert!(app.slot_busy(Slot::Storage), "the disk slot is taken");
        app.panel = Panel::Uninstall;
        app.scan();
        assert!(
            app.jobs.apps.is_some(),
            "the app list must not wait for the disk map"
        );

        // Two foreground jobs at once is still not allowed — the progress bar
        // answers to one key press — and the refusal says which one.
        assert!(app.busy_refusal(Slot::Delete));
        let text = app.toast.take().expect("a reason").text;
        assert!(
            text.contains("应用列表") || text.contains("app list"),
            "the refusal must name the foreground job: {text}"
        );

        if let Some(handle) = app.jobs.apps.take() {
            handle.cancel();
        }
        if let Some(handle) = app.jobs.storage.take() {
            handle.cancel();
        }
    }

    /// The report this came from: uninstall one app, try the next, be told the
    /// previous task is still running. It was the automatic disk-map refresh,
    /// and it must now run beside the user instead of in front of them.
    #[test]
    fn the_map_refresh_after_an_uninstall_does_not_lock_the_next_one() {
        isolate_state();
        let dir = scratch("chain");
        let (first, first_items) = fake_app(&dir);
        let second_dir = dir.join("second");
        std::fs::create_dir_all(&second_dir).expect("second folder");
        let (second, second_items) = fake_app(&second_dir);
        let second_bundle = second.path.clone();

        let mut app = App::new().expect("app");
        app.delete_mode = DeleteMode::Permanent;
        app.storage.root = dir.clone();
        app.storage.scanned = true;

        let outcome = uninstall_one(&mut app, first, first_items);
        app.apply_deletion(Panel::Uninstall, outcome);
        assert!(
            app.jobs.storage.is_some(),
            "the disk map has to refresh itself after a delete"
        );

        // The bug: this second call answered "上一个任务还在进行".
        let outcome = uninstall_one(&mut app, second, second_items);
        app.apply_deletion(Panel::Uninstall, outcome);
        assert!(
            !second_bundle.exists(),
            "the second app must really be gone"
        );

        if let Some(handle) = app.jobs.storage.take() {
            handle.cancel();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Drive one uninstall the way the keyboard does and hand back the outcome,
    /// so a test can chain two of them like a user does.
    fn uninstall_one(app: &mut App, info: AppInfo, items: Vec<CleanupItem>) -> DeleteOutcome {
        app.panel = Panel::Uninstall;
        app.uninstall.view = UninstallView::Plan;
        app.uninstall.plan = Some(UninstallPlan {
            app: info,
            items,
            warnings: Vec::new(),
        });
        app.ask_delete();
        assert!(
            app.confirm.is_some(),
            "nothing may refuse the next uninstall: {:?}",
            app.toast.as_ref().map(|toast| toast.text.clone())
        );
        app.confirm_delete();
        let handle = app
            .jobs
            .delete
            .take()
            .expect("the delete must start, not be refused");
        let started = Instant::now();
        loop {
            if let Some(value) = handle.poll() {
                return value.expect("delete job");
            }
            assert!(
                started.elapsed() < Duration::from_secs(60),
                "the delete job never reported back"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The sidebar is a keyboard destination — and it is where the keyboard
    /// *starts*. Opening on a list of junk that has to be ignored before the
    /// first choice is possible costs a keystroke on every single session.
    #[test]
    fn the_keyboard_opens_on_the_sidebar() {
        isolate_state();
        let mut app = App::new().expect("app");
        assert!(app.menu, "the arrows start on the six functions");
        assert!(app.toast.is_some(), "and the screen says so");
        assert!(
            app.log
                .iter()
                .any(|line| line.contains("colour") || line.contains("色彩")),
            "start-up says which colour depth the terminal got: {:?}",
            app.log
        );

        // From here on a job is running so no panel jump starts a real scan;
        // moving the selection must never be refused because of it.
        let (handle, _progress) = crate::job::spawn("apps", |_engine, _progress, _cancel| {
            std::thread::sleep(Duration::from_secs(30));
            Ok(Vec::<AppInfo>::new())
        });
        app.jobs.apps = Some(handle);

        app.handle_key(KeyCode::Down, KeyModifiers::empty());
        assert_eq!(app.panel, Panel::Uninstall, "down is one entry");
        app.handle_key(KeyCode::Char('j'), KeyModifiers::empty());
        assert_eq!(app.panel, Panel::Purge, "j works too");
        app.handle_key(KeyCode::Up, KeyModifiers::empty());
        assert_eq!(app.panel, Panel::Uninstall, "up walks back");
        for _ in 0..10 {
            app.handle_key(KeyCode::Up, KeyModifiers::empty());
        }
        assert_eq!(app.panel, Panel::Cleanup, "the menu clamps, never wraps");

        app.handle_key(KeyCode::Char('3'), KeyModifiers::empty());
        assert_eq!(app.panel, Panel::Purge, "1-6 still jump");
        assert!(app.menu, "and a digit leaves you in the menu");

        app.handle_key(KeyCode::Enter, KeyModifiers::empty());
        assert!(!app.menu, "enter hands the keys back to the list");
        assert_eq!(app.panel, Panel::Purge, "on the function you chose");

        app.handle_key(KeyCode::Esc, KeyModifiers::empty());
        assert!(app.menu, "and esc gives them back to the sidebar");

        assert!(
            matches!(
                app.handle_key(KeyCode::Char('q'), KeyModifiers::empty()),
                Action::Quit
            ),
            "q quits from the menu as well"
        );

        if let Some(handle) = app.jobs.apps.take() {
            handle.cancel();
        }
    }

    /// While the list has focus `esc` still means *back* on the storage panel —
    /// the menu must not steal the key the drill-down depends on.
    #[test]
    fn esc_still_walks_up_the_disk_map() {
        isolate_state();
        let mut app = App::new().expect("app");
        app.menu = false;
        app.panel = Panel::Storage;
        let root = idle_node("up-root");
        let mut child = idle_node("up-child");
        child.path = root.path.join("Library");
        app.storage.stack = vec![root, child];

        app.handle_key(KeyCode::Esc, KeyModifiers::empty());
        assert_eq!(app.storage.stack.len(), 1, "esc left one folder");
        assert!(!app.menu, "and did not open the menu");

        app.handle_key(KeyCode::Esc, KeyModifiers::empty());
        assert!(app.menu, "at the top of the map esc opens the menu");
    }

    /// The screen opens on the sidebar, so the keys the hint bar advertises
    /// have to work from there. `s` used to be swallowed by the menu, which
    /// left a first-time user staring at 「尚未扫描 — 按 s」 while the one key
    /// on the screen did nothing: an order is not a choice of where the
    /// arrows go, so anything that is not navigation is handed to the panel.
    #[test]
    fn an_order_key_is_not_swallowed_by_the_sidebar() {
        isolate_state();
        let mut app = App::new().expect("app");
        assert!(app.menu, "the screen opens on the six functions");

        app.handle_key(KeyCode::Char('s'), KeyModifiers::empty());
        assert!(
            app.jobs.clean.is_some(),
            "s starts the scan the screen just asked for: {:?}",
            app.toast.as_ref().map(|toast| toast.text.clone())
        );
        assert!(!app.menu, "and the list has the keys while it runs");
        app.cancel_jobs();

        // Help is a layer of its own: esc peels it off and stops there,
        // rather than walking out of the sidebar as well.
        app.menu = true;
        app.handle_key(KeyCode::Char('?'), KeyModifiers::empty());
        assert!(app.help, "? still opens the help from the menu");
        assert!(app.menu, "and leaves the focus where it was");
        app.handle_key(KeyCode::Esc, KeyModifiers::empty());
        assert!(!app.help, "esc closes the help");
        assert!(app.menu, "without walking past it into the list");

        // A key the sidebar has no use for belongs to the panel on screen.
        app.handle_key(KeyCode::Char('a'), KeyModifiers::empty());
        assert!(!app.menu, "a selects; it is not ignored on the way");
    }

    /// Every view below the top level is entered on purpose, so `esc` has to
    /// lead back out of it — one level per press. A confirmation that answers
    /// to nothing, a plan you can only leave by uninstalling from it, and a
    /// disk map that threw away two levels at once are all the same bug: the
    /// keyboard walking the user past a screen they meant to step back from.
    #[test]
    fn esc_walks_back_one_level_at_a_time() {
        isolate_state();

        // The plan of an application goes back to the list of applications.
        let mut app = App::new().expect("app");
        app.menu = false;
        app.panel = Panel::Uninstall;
        let dir = scratch("esc-plan");
        let (info, items) = fake_app(&dir);
        app.uninstall.set_plan(UninstallPlan {
            app: info,
            items,
            warnings: Vec::new(),
        });
        assert_eq!(app.uninstall.view, UninstallView::Plan, "entered the app");
        app.handle_key(KeyCode::Esc, KeyModifiers::empty());
        assert_eq!(app.uninstall.view, UninstallView::List, "esc leaves it");
        assert!(!app.menu, "and stops there, on the list of apps");
        app.handle_key(KeyCode::Esc, KeyModifiers::empty());
        assert!(app.menu, "the next esc reaches the sidebar");

        // The confirmation is the one screen that *must* answer to esc.
        let mut app = App::new().expect("app");
        app.menu = false;
        let dialog = || Confirm {
            title: "delete?".to_string(),
            lines: vec!["3 items".to_string()],
            action: ConfirmAction::Clean,
        };
        for key in [KeyCode::Esc, KeyCode::Char('n')] {
            app.confirm = Some(dialog());
            app.pending = Some(Approval::Items {
                items: Vec::new(),
                purge: false,
            });
            app.handle_key(key, KeyModifiers::empty());
            assert!(app.confirm.is_none(), "{key:?} must close the dialog");
            assert!(app.pending.is_none(), "with the approval it carried");
            assert!(
                app.toast.is_some(),
                "and say what happened instead of vanishing"
            );
        }

        // Largest files, then the folder it came from, then the menu: three
        // presses, one level each.
        let mut app = App::new().expect("app");
        app.menu = false;
        app.panel = Panel::Storage;
        let root = idle_node("back-root");
        let mut child = idle_node("back-child");
        child.path = root.path.join("Library");
        app.storage.stack = vec![root, child];
        app.storage.show_files = true;

        app.handle_key(KeyCode::Esc, KeyModifiers::empty());
        assert!(!app.storage.show_files, "esc leaves the largest-files list");
        assert_eq!(
            app.storage.stack.len(),
            2,
            "and does not take the folder along with it"
        );
        app.handle_key(KeyCode::Esc, KeyModifiers::empty());
        assert_eq!(app.storage.stack.len(), 1, "the next esc leaves the folder");
        app.handle_key(KeyCode::Esc, KeyModifiers::empty());
        assert!(app.menu, "the third one reaches the sidebar");
    }

    /// The second half of 「二、三级功能里按 esc 无法取消或回到上级」: the level
    /// *was* left, and then the work that level had been waiting for landed and
    /// opened it again. Three cases were reported and are all fixed the same
    /// way — leaving a level throws its in-flight job away ([`App::abandon`]).
    #[test]
    fn a_level_that_is_still_building_stays_left() {
        isolate_state();
        // Long enough that the orphaned job has certainly answered by the time
        // the assertions run: it is its *answer* that must not reach the screen.
        const WAIT: Duration = Duration::from_millis(700);
        let settle = |app: &mut App| {
            let started = Instant::now();
            while started.elapsed() < WAIT {
                app.tick();
                std::thread::sleep(Duration::from_millis(20));
            }
        };

        // 1 · the leftover plan of an application.
        let mut app = App::new().expect("app");
        app.menu = false;
        app.panel = Panel::Uninstall;
        app.uninstall.view = UninstallView::Plan;
        let (handle, progress) = crate::job::spawn("plan", |_engine, _progress, _cancel| {
            std::thread::sleep(Duration::from_millis(150));
            Ok(UninstallPlan::default())
        });
        app.jobs.plan = Some(handle);
        app.set_bar(progress, "plan");
        app.handle_key(KeyCode::Esc, KeyModifiers::empty());
        assert_eq!(app.uninstall.view, UninstallView::List, "esc left the plan");
        assert!(
            app.jobs.plan.is_none(),
            "and stopped waiting for its answer"
        );
        settle(&mut app);
        assert_eq!(
            app.uninstall.view,
            UninstallView::List,
            "the plan that finished afterwards did not walk back into it"
        );
        assert!(app.uninstall.plan.is_none(), "no stray plan was kept");
        assert!(app.progress.is_none(), "nor its progress bar");

        // 2 · the largest-files list of a folder on the disk map.
        let mut app = App::new().expect("app");
        app.menu = false;
        app.panel = Panel::Storage;
        app.storage.stack = vec![idle_node("abandon-files")];
        app.storage.show_files = true;
        let (handle, progress) = crate::job::spawn("files", |_engine, _progress, _cancel| {
            std::thread::sleep(Duration::from_millis(150));
            Ok(Vec::new())
        });
        app.jobs.files = Some(handle);
        app.set_bar(progress, "files");
        app.handle_key(KeyCode::Esc, KeyModifiers::empty());
        assert!(!app.storage.show_files, "esc left the list of files");
        settle(&mut app);
        assert!(
            !app.storage.show_files,
            "and the listing that arrived afterwards did not reopen it"
        );

        // 3 · a folder whose scan was still on its way.
        let mut app = App::new().expect("app");
        app.menu = false;
        app.panel = Panel::Storage;
        let here = idle_node("abandon-drill");
        let away = here.path.join("Library");
        app.storage.stack = vec![here];
        let expected = app.storage.stack[0].path.clone();
        let (handle, progress) = crate::job::spawn("storage", move |_, _, _| {
            std::thread::sleep(Duration::from_millis(150));
            let mut node = idle_node("abandon-drill-landed");
            node.path = away;
            Ok(node)
        });
        app.jobs.storage = Some(handle);
        app.pending_drill = true;
        app.set_bar(progress, "storage");
        app.handle_key(KeyCode::Esc, KeyModifiers::empty());
        assert!(!app.pending_drill, "esc called off the drill-down");
        assert!(app.jobs.storage.is_none(), "and its answer");
        settle(&mut app);
        assert_eq!(
            app.storage.stack[0].path, expected,
            "the folder never opened itself"
        );
    }

    /// A background disk walk is invisible work; it must not hold the keyboard
    /// hostage. `esc` reaches the sidebar through it, and it is not cancelled
    /// on the way, because cancelling a scan the user never started is its own
    /// kind of "上一个任务还在进行".
    #[test]
    fn esc_reaches_the_sidebar_through_a_running_disk_walk() {
        isolate_state();
        let mut app = App::new().expect("app");
        app.menu = false;
        let sentinel = idle_node("esc-walk");
        let (handle, progress) = crate::job::spawn("storage", move |_engine, _progress, cancel| {
            std::thread::sleep(Duration::from_millis(250));
            let mut node = sentinel;
            node.path = PathBuf::from(format!("cancelled={}", cancel.is_cancelled()));
            Ok(node)
        });
        app.jobs.storage = Some(handle);
        app.set_bar(progress, "storage");

        app.handle_key(KeyCode::Esc, KeyModifiers::empty());
        assert!(
            app.menu,
            "esc must open the menu while the map walks: {:?}",
            app.toast.as_ref().map(|toast| toast.text.clone())
        );

        let handle = app.jobs.storage.take().expect("the walk is still there");
        let started = Instant::now();
        let walked = loop {
            if let Some(value) = handle.poll() {
                break value.expect("the walk finished");
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the walk never reported back"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(
            walked.path.to_string_lossy(),
            "cancelled=false",
            "esc must not cancel the walk on its way past"
        );
    }

    /// The gauge answers to the key that was pressed. Once that job is done the
    /// bar is cleared or handed to the walk that was waiting — it never keeps
    /// printing "delete · 4/4" under a "已删除" toast because an unrelated disk
    /// walk is still holding `busy()`.
    #[test]
    fn a_finished_job_never_leaves_its_gauge_on_screen() {
        isolate_state();
        let mut app = App::new().expect("app");
        let node = idle_node("bar");
        let (walk, walk_progress) =
            crate::job::spawn("storage", move |_engine, _progress, _cancel| {
                std::thread::sleep(Duration::from_secs(30));
                Ok(node)
            });
        app.jobs.storage = Some(walk);
        app.set_bar(walk_progress, "storage");

        // Pressing a key takes the bar; the walk steps aside with its handle.
        let (other, other_progress) = crate::job::spawn("files", |_engine, _progress, _cancel| {
            Ok(Vec::<analyze::FileEntry>::new())
        });
        app.jobs.files = Some(other);
        app.start_progress(other_progress, "files");
        assert!(
            app.bg.is_some(),
            "the disk walk keeps its events instead of vanishing"
        );

        let started = Instant::now();
        let value = loop {
            if let Some(value) = app.jobs.files.as_ref().expect("job").poll() {
                break value;
            }
            assert!(started.elapsed() < Duration::from_secs(5), "job hung");
            std::thread::sleep(Duration::from_millis(10));
        }
        .expect("the files job");
        assert!(value.is_empty());
        app.jobs.files = None;
        app.finish_job();
        assert_ne!(app.phase, "files", "the finished job's label must go");

        // And with no walk parked in the background, the bar clears outright.
        app.bg = None;
        app.phase = "delete".to_string();
        app.progress = Some(Progress::new());
        app.done = 4;
        app.finish_job();
        assert!(
            app.progress.is_none() && app.phase.is_empty() && app.total.is_none(),
            "a bar over a still-running disk walk is a lie: {:?}",
            app.phase
        );

        app.jobs.storage.take().expect("walk").cancel();
    }

    /// The six entries in the sidebar answer the arrow keys from anywhere, not
    /// only after `esc` — but the plain arrows still drive the list, because a
    /// cleanup list you cannot scroll is worse than one more modifier.
    #[test]
    fn option_arrows_switch_functions_from_anywhere() {
        isolate_state();
        let mut app = App::new().expect("app");
        app.menu = false;
        assert_eq!(app.panel, Panel::Cleanup);

        app.handle_key(KeyCode::Down, KeyModifiers::ALT);
        assert_eq!(app.panel, Panel::Uninstall, "⌥↓ moved down the menu");
        assert!(!app.menu, "and the list kept the keyboard");

        app.handle_key(KeyCode::Up, KeyModifiers::ALT);
        assert_eq!(app.panel, Panel::Cleanup, "⌥↑ moved back up");

        app.handle_key(KeyCode::Up, KeyModifiers::ALT);
        assert_eq!(app.panel, Panel::Cleanup, "the menu clamps, never wraps");

        app.handle_key(KeyCode::Down, KeyModifiers::CONTROL);
        assert_eq!(app.panel, Panel::Uninstall, "ctrl+↓ counts too");
        if let Some(handle) = app.jobs.apps.take() {
            handle.cancel();
        }
        app.handle_key(KeyCode::Char('1'), KeyModifiers::empty());
        assert_eq!(app.panel, Panel::Cleanup);

        // Plain arrows are still the list's.
        let junk = |name: &str| {
            CleanupItem::new(
                PathBuf::from(format!("/tmp/squid-arrows-{name}")),
                ItemKind::File,
                Category::SystemJunk,
                name,
            )
        };
        app.clean.items = vec![junk("one"), junk("two")];
        app.clean.rebuild_lines();
        assert_eq!(app.clean.lines.len(), 3, "one header and two rows");
        app.handle_key(KeyCode::Down, KeyModifiers::empty());
        assert_eq!(app.clean.cursor, 1, "the plain arrow moved one row down");
        app.handle_key(KeyCode::Down, KeyModifiers::empty());
        assert_eq!(app.clean.cursor, 2, "and again");
        assert_eq!(app.panel, Panel::Cleanup, "without switching function");
    }

    fn cached(path: &str, bytes: u64) -> CleanupItem {
        let mut item = CleanupItem::new(
            path.into(),
            ItemKind::Path,
            Category::AppCaches,
            "app-caches",
        );
        item.allocated = Bytes(bytes);
        item
    }

    /// The "清理完成后，分组的数据不变" bug: the delete pruned `items`, but every
    /// group header and sidebar row still read its total from `buckets`, which
    /// only ever arrived with the scan.
    #[test]
    fn a_pruned_list_recomputes_its_group_totals() {
        let mut panel = ScanPanel {
            items: vec![
                cached("/c/big", 6_000_000_000),
                cached("/c/small", 1_000_000_000),
            ],
            scanned_at: Some(0),
            ..ScanPanel::default()
        };
        panel.rebuild_lines();
        assert_eq!(
            panel.buckets[&Category::AppCaches],
            (Bytes(7_000_000_000), 2)
        );

        panel.items.retain(|item| item.path == Path::new("/c/big"));
        panel.rebuild_lines();
        assert_eq!(
            panel.buckets[&Category::AppCaches],
            (Bytes(6_000_000_000), 1),
            "the header must follow the rows it counts"
        );
    }

    /// A scan in flight has buckets before it has items: an empty list is not
    /// evidence that everything went to zero.
    #[test]
    fn an_empty_list_does_not_erase_the_buckets_of_a_running_scan() {
        let mut panel = ScanPanel::default();
        panel
            .buckets
            .insert(Category::AppCaches, (Bytes(4_000_000_000), 9));
        panel.sync_buckets();
        assert_eq!(panel.buckets.len(), 1);
    }

    /// "清理动作完成后，分组的数据不变" — and the answer to it is a fresh scan.
    /// Pruning the rows by hand cannot know that a cache rebuilt itself while
    /// the user was reading the header, so the panel scans itself again exactly
    /// like the `s` key does.
    #[test]
    fn a_finished_cleanup_scans_itself_again() {
        isolate_state();
        let dir = scratch("rescan");
        let file = dir.join("junk.bin");
        std::fs::write(&file, vec![7u8; 4096]).expect("junk");

        let mut app = App::new().expect("app");
        app.delete_mode = DeleteMode::Permanent;
        app.panel = Panel::Cleanup;
        let mut item = CleanupItem::new(file.clone(), ItemKind::File, Category::SystemJunk, "junk");
        item.size = Bytes(4096);
        app.clean.items = vec![item];
        app.clean.scanned_at = Some(0);
        app.clean
            .buckets
            .insert(Category::SystemJunk, (Bytes(4096), 1));
        app.ask_delete();
        app.confirm_delete();
        let handle = app.jobs.delete.take().expect("a delete job");
        let started = Instant::now();
        let outcome = loop {
            if let Some(value) = handle.poll() {
                break value.expect("delete job");
            }
            assert!(started.elapsed() < Duration::from_secs(60));
            std::thread::sleep(Duration::from_millis(20));
        };
        app.apply_deletion(Panel::Cleanup, outcome);
        assert!(
            app.jobs.clean.is_some(),
            "the panel rescans itself instead of leaving the old totals behind: {:?}",
            app.toast.as_ref().map(|toast| toast.text.clone())
        );
        let handle = app.jobs.clean.take().expect("the rescan job");
        let started = Instant::now();
        let report = loop {
            if let Some(value) = handle.poll() {
                break value.expect("rescan job");
            }
            assert!(
                started.elapsed() < Duration::from_secs(120),
                "the rescan never reported back"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(
            report.items.iter().all(|item| item.path != file),
            "the file that was deleted must not come back"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
