//! Data-driven cleanup rules.
//!
//! A rule is a declarative description of *where* junk lives; the engine turns
//! it into concrete [`squid_core::CleanupItem`]s. Keeping the knowledge in TOML
//! means new patterns can be added (or disabled by users) without recompiling,
//! and — importantly — it keeps the dangerous path list auditable in one place.
//!
//! Patterns are intentionally expressed as globs with a *literal anchor*: the
//! longest leading path without wildcards. That anchor becomes the safety
//! anchor passed to [`squid_core::SafetyPolicy`], so a bug in glob expansion
//! can never produce a deletion outside the directory the rule owns.
//!
//! Pattern syntax
//! --------------
//! * `~/…` and `$HOME/…` expand to the home directory.
//! * `${VAR}` / `$VAR` expand from the environment (unset ⇒ rule yields nothing).
//! * `$(brew --prefix)` resolves the Homebrew prefix once per run.
//! * `*`, `?` and `{a,b}` work per path component; `*` never crosses `/`.
//! * `**` matches any number of components (used sparingly).

use globset::{Glob, GlobBuilder, GlobMatcher, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use squid_core::paths;
use squid_core::settings::Settings;
use squid_core::Category;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// What should happen to a matched path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleMode {
    /// Remove the matched entry itself (directory trees included).
    #[default]
    Path,
    /// Remove the *children* of the matched directory but keep the directory —
    /// required for lock files and folders apps expect to exist (Caches, Trash).
    Contents,
    /// Only remove regular files; directories are descended into.
    Files,
}

impl RuleMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleMode::Path => "path",
            RuleMode::Contents => "contents",
            RuleMode::Files => "files",
        }
    }
}

fn default_true() -> bool {
    true
}

fn category_from_str<'de, D>(deserializer: D) -> Result<Category, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;
    let raw = String::deserialize(deserializer)?;
    Category::parse(&raw).ok_or_else(|| D::Error::custom(format!("unknown category `{raw}`")))
}

/// One cleanup rule as written in `rules/*.toml`.
#[derive(Clone, Debug, Deserialize)]
pub struct Rule {
    pub id: String,

    #[serde(deserialize_with = "category_from_str")]
    pub category: Category,

    #[serde(default)]
    pub title: String,

    pub patterns: Vec<String>,

    #[serde(default)]
    pub mode: RuleMode,

    /// Platforms this rule applies to (`macos`, `linux`, `windows`).
    /// Empty means "every platform".
    #[serde(default)]
    pub os: Vec<String>,

    /// Globs (matched against the absolute path *and* the path relative to the
    /// pattern anchor) that must never be deleted.
    #[serde(default)]
    pub protect: Vec<String>,

    /// Skip entries modified more recently than this many days ago.
    #[serde(default)]
    pub min_age_days: u64,

    #[serde(default = "default_true")]
    pub select_by_default: bool,

    /// Rule touches paths the current user cannot write: surface it, but keep
    /// it unselected unless the user opted in.
    #[serde(default)]
    pub needs_admin: bool,

    /// Process names that make this rule unsafe right now.
    #[serde(default)]
    pub skip_if_running: Vec<String>,

    /// Show a warning marker in the UI.
    #[serde(default)]
    pub risky: bool,

    #[serde(default)]
    pub note: Option<String>,
}

impl Rule {
    pub fn display_title(&self) -> &str {
        if self.title.is_empty() {
            &self.id
        } else {
            &self.title
        }
    }

    pub fn applies_to_current_os(&self) -> bool {
        if self.os.is_empty() {
            return true;
        }
        self.os.iter().any(|os| os == current_os_tag())
    }

    pub fn min_age(&self) -> Option<std::time::Duration> {
        if self.min_age_days == 0 {
            None
        } else {
            Some(std::time::Duration::from_secs(self.min_age_days * 86_400))
        }
    }
}

fn current_os_tag() -> &'static str {
    match squid_platform::PlatformId::current() {
        squid_platform::PlatformId::MacOS => "macos",
        squid_platform::PlatformId::Linux => "linux",
        squid_platform::PlatformId::Windows => "windows",
        squid_platform::PlatformId::Unknown => "unknown",
    }
}

#[derive(Deserialize)]
struct RuleFile {
    #[serde(default)]
    rule: Vec<Rule>,
}

/// The rules that apply to this machine, in display order.
#[derive(Clone, Debug, Default)]
pub struct RuleSet {
    rules: Vec<Rule>,
    errors: Vec<String>,
}

impl RuleSet {
    /// Built-in rules for the host OS plus anything the user dropped into
    /// `<config dir>/rules/*.toml`, filtered by `settings.disabled_rules`.
    pub fn load(settings: &Settings) -> RuleSet {
        let mut set = RuleSet::builtins();
        set.merge_user_rules();
        set.rules
            .retain(|rule| rule.applies_to_current_os() && settings.rule_enabled(&rule.id));
        set.rules
            .sort_by(|a, b| a.category.cmp(&b.category).then_with(|| a.id.cmp(&b.id)));
        set
    }

    /// Everything shipped for the current platform, unfiltered.
    pub fn builtins() -> RuleSet {
        let mut set = RuleSet::default();
        if cfg!(target_os = "macos") {
            set.parse_str(EMBEDDED_MACOS, "builtin:macos.toml");
        }
        if cfg!(target_os = "windows") {
            set.parse_str(EMBEDDED_WINDOWS, "builtin:windows.toml");
        }
        set
    }

    fn merge_user_rules(&mut self) {
        let dir = Settings::config_dir().join("rules");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().map(|ext| ext == "toml").unwrap_or(false))
            .collect();
        files.sort();
        for file in files {
            let Ok(text) = std::fs::read_to_string(&file) else {
                self.errors
                    .push(format!("{}: could not read", file.display()));
                continue;
            };
            let label = file.display().to_string();
            self.parse_str(&text, &label);
        }
    }

    fn parse_str(&mut self, text: &str, label: &str) {
        match toml::from_str::<RuleFile>(text) {
            Ok(file) => self.rules.extend(file.rule),
            Err(err) => self.errors.push(format!("{label}: {err}")),
        }
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn by_category(&self, category: Category) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|rule| rule.category == category)
            .collect()
    }

    pub fn categories(&self) -> Vec<Category> {
        let mut seen: Vec<Category> = Vec::new();
        for rule in &self.rules {
            if !seen.contains(&rule.category) {
                seen.push(rule.category);
            }
        }
        seen.sort();
        seen
    }

    pub fn get(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.id == id)
    }
}

// ─────────────────────────────── expansion ────────────────────────────────

/// One path a rule matched, plus the anchor that authorises deleting it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub path: PathBuf,
    /// Literal directory the pattern lives under; deletions must stay inside.
    pub anchor: PathBuf,
}

/// The only command substitutions rules may use. Deliberately a fixed list:
/// rules are data, and data must not be able to run arbitrary programs.
fn run_substitution(command: &str) -> Option<String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(known) = guard.get(command) {
            return known.clone();
        }
    }
    let allowed = matches!(
        command,
        "brew --prefix" | "brew --repository" | "getconf DARWIN_USER_CACHE_DIR"
    );
    let value = if allowed {
        let mut parts = command.split_whitespace();
        let program = parts.next()?;
        std::process::Command::new(program)
            .args(parts)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|out| !out.is_empty() && out.starts_with('/'))
    } else {
        None
    };
    if let Ok(mut guard) = cache.lock() {
        guard.insert(command.to_string(), value.clone());
    }
    value
}

/// Resolve `$HOME`, environment variables and the one command substitution we
/// allow. Returns `None` when a required variable or tool is missing.
pub fn resolve_variables(pattern: &str) -> Option<String> {
    let mut text = pattern.trim().to_string();
    if text.is_empty() {
        return None;
    }
    while let Some(start) = text.find("$(") {
        let rest = &text[start + 2..];
        let end = rest.find(')')?;
        let command = rest[..end].trim().to_string();
        let replacement = run_substitution(&command)?;
        text = format!("{}{}{}", &text[..start], replacement, &rest[end + 1..]);
    }
    if text == "~" || text.starts_with("~/") || text.starts_with("~\\") {
        text = text.replacen('~', &paths::home_dir().to_string_lossy(), 1);
    }
    let mut cursor = 0usize;
    while let Some(offset) = text[cursor..].find("${") {
        let pos = cursor + offset;
        let rest = &text[pos + 2..];
        let end = rest.find('}')?;
        let name = rest[..end].to_string();
        let value = env_value(&name)?;
        text = format!("{}{}{}", &text[..pos], value, &rest[end + 1..]);
        // Continue after the substituted value so replacement text is never
        // re-scanned (an env var may itself contain `$`).
        cursor = pos + value.len();
    }
    if text.starts_with('$') {
        let head = text
            .find(['/', '\\'])
            .map(|idx| text[1..idx].to_string())
            .unwrap_or_else(|| text[1..].to_string());
        let value = env_value(&head)?;
        text = format!("{}{}", value, &text[1 + head.len()..]);
    }
    Some(text)
}

fn env_value(name: &str) -> Option<String> {
    if name == "HOME" {
        return Some(paths::home_dir().to_string_lossy().into_owned());
    }
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn has_glob(component: &str) -> bool {
    component
        .chars()
        .any(|c| matches!(c, '*' | '?' | '[' | '{'))
}

fn glob_matcher(component: &str) -> Option<GlobMatcher> {
    GlobBuilder::new(component)
        .literal_separator(true)
        .backslash_escape(true)
        // Windows filenames are matched case-insensitively by the OS; rules
        // must not silently miss entries that differ only in casing.
        .case_insensitive(cfg!(windows))
        .build()
        .ok()
        .map(|glob| glob.compile_matcher())
}

/// The literal directory in front of the first wildcard component.
///
/// `~/Library/Caches/Google/*` → `~/Library/Caches/Google`; a pattern without
/// wildcards anchors on its parent so `SafetyPolicy::check` still sees depth ≥ 1.
pub fn anchor_for(pattern: &str) -> Option<PathBuf> {
    let resolved = resolve_variables(pattern)?;
    let resolved = resolved.trim_end_matches('/');
    if resolved.is_empty() {
        return None;
    }
    let path = Path::new(resolved);
    let mut anchor = PathBuf::new();
    let mut saw_glob = false;
    for component in path.components() {
        let text = match component {
            Component::RootDir => {
                // Windows spells `C:\…` as Prefix("C:") then RootDir; fold
                // them into a rooted `C:\` anchor instead of drive-relative
                // `C:/`. On Unix this is the leading `/`.
                let existing = anchor.as_os_str().to_string_lossy().into_owned();
                if !existing.is_empty() && !existing.contains(['/', '\\']) {
                    anchor = PathBuf::from(format!("{existing}\\", existing = existing));
                } else {
                    anchor.push("/");
                }
                continue;
            }
            Component::CurDir => continue,
            // Drive/UNC prefixes are part of the anchor on Windows.
            Component::Prefix(prefix) => {
                anchor.push(prefix.as_os_str());
                continue;
            }
            Component::Normal(text) => text.to_string_lossy().into_owned(),
            Component::ParentDir => return None,
        };
        if has_glob(&text) {
            saw_glob = true;
            break;
        }
        anchor.push(text);
    }
    if !saw_glob {
        anchor = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("/"));
    }
    if anchor.as_os_str().is_empty() {
        anchor = PathBuf::from("/");
    }
    Some(anchor)
}

/// Expand one glob pattern into the existing paths it matches.
///
/// Expansion walks the filesystem component by component, so a wildcard can
/// only ever enumerate directories that really exist below the anchor.
pub fn expand_pattern(pattern: &str) -> Vec<Match> {
    let Some(anchor) = anchor_for(pattern) else {
        return Vec::new();
    };
    let Some(resolved) = resolve_variables(pattern) else {
        return Vec::new();
    };
    let resolved = resolved.trim_end_matches('/');
    let components: Vec<String> = Path::new(resolved)
        .components()
        .filter_map(|c| match c {
            Component::Normal(text) => Some(text.to_string_lossy().into_owned()),
            Component::RootDir => Some("/".to_string()),
            _ => None,
        })
        .collect();

    let mut frontier: Vec<PathBuf> = if anchor.exists() {
        vec![anchor.clone()]
    } else {
        return Vec::new();
    };

    let Some(glob_at) = components.iter().position(|component| has_glob(component)) else {
        // A wildcard-free pattern names exactly one path. `anchor_for` handed us
        // its *parent* so the safety layer has a directory to vouch for, and the
        // frontier would still hold that parent — returning it here would turn
        // "~/.cargo/.package-cache" into "delete all of ~/.cargo".
        let literal = PathBuf::from(resolved);
        return match literal.symlink_metadata() {
            Ok(_) => vec![Match {
                path: literal,
                anchor,
            }],
            Err(_) => Vec::new(),
        };
    };
    let mut cursor = glob_at;

    while cursor < components.len() {
        let component = components[cursor].as_str();
        if component == "**" {
            let mut next = Vec::new();
            for dir in &frontier {
                next.extend(descendants(dir, 6));
            }
            next.sort();
            next.dedup();
            frontier = next;
            cursor += 1;
            if frontier.is_empty() {
                break;
            }
            continue;
        }
        let Some(matcher) = glob_matcher(component) else {
            return Vec::new();
        };
        let mut next = Vec::new();
        for dir in &frontier {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !matcher.is_match(&name) {
                    continue;
                }
                let path = dir.join(&name);
                let is_last = cursor + 1 == components.len();
                if is_last || path.is_dir() {
                    next.push(path);
                }
            }
        }
        next.sort();
        next.dedup();
        frontier = next;
        cursor += 1;
        if frontier.is_empty() {
            break;
        }
    }

    frontier.retain(|path| path.symlink_metadata().is_ok());
    frontier
        .into_iter()
        .map(|path| Match {
            path,
            anchor: anchor.clone(),
        })
        .collect()
}

fn descendants(dir: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if max_depth == 0 {
        return out;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        out.push(path.clone());
        if is_dir {
            out.extend(descendants(&path, max_depth - 1));
        }
    }
    out
}

/// Compiled `protect` list of one rule.
#[derive(Clone, Debug, Default)]
pub struct Protector {
    absolute: Vec<GlobMatcher>,
    relative: Vec<GlobMatcher>,
}

impl Protector {
    pub fn new(rule: &Rule) -> Protector {
        let mut absolute = Vec::new();
        let mut relative = Vec::new();
        for raw in &rule.protect {
            #[cfg_attr(not(windows), allow(unused_mut))]
            let mut resolved = resolve_variables(raw).unwrap_or_else(|| raw.clone());
            #[cfg(windows)]
            {
                // Matching below funnels through `/`-normalised text; make the
                // patterns speak the same language.
                resolved = resolved.replace('\\', "/");
            }
            let is_absolute = resolved.starts_with('/')
                || (cfg!(windows)
                    && resolved.as_bytes().get(1) == Some(&b':')
                    && resolved.as_bytes()[0].is_ascii_alphabetic());
            if is_absolute {
                if let Some(matcher) = glob_matcher(&resolved) {
                    absolute.push(matcher);
                    continue;
                }
            }
            match GlobBuilder::new(&format!("**/{resolved}"))
                .case_insensitive(cfg!(windows))
                .build()
            {
                Ok(glob) => relative.push(glob.compile_matcher()),
                Err(_) => {
                    if let Some(matcher) = glob_matcher(raw) {
                        relative.push(matcher);
                    }
                }
            }
        }
        Protector { absolute, relative }
    }

    pub fn is_empty(&self) -> bool {
        self.absolute.is_empty() && self.relative.is_empty()
    }

    /// `true` when the path (or any ancestor below the anchor) is protected, so
    /// protecting `Homebrew` also protects everything inside it.
    pub fn is_protected(&self, path: &Path, anchor: &Path) -> bool {
        if self.absolute.is_empty() && self.relative.is_empty() {
            return false;
        }
        let text = path.to_string_lossy().into_owned();
        #[cfg(windows)]
        let text = text.replace('\\', "/");
        if self.absolute.iter().any(|m| m.is_match(&*text)) {
            return true;
        }
        for ancestor in path.ancestors() {
            // Case-insensitive containment on Windows; `None` also covers the
            // anchor itself, whose next ancestor leaves the anchor anyway.
            let Some(rel) = paths::relative_below(anchor, ancestor) else {
                break;
            };
            let rel = rel.to_string_lossy().into_owned();
            #[cfg(windows)]
            let rel = rel.replace('\\', "/");
            if self.relative.iter().any(|m| m.is_match(&*rel)) {
                return true;
            }
        }
        false
    }
}

/// A rule bundled with its compiled matchers — what the engine actually uses.
#[derive(Clone, Debug)]
pub struct PreparedRule {
    pub rule: Rule,
    protector: Protector,
}

impl PreparedRule {
    pub fn new(rule: Rule) -> PreparedRule {
        let protector = Protector::new(&rule);
        PreparedRule { rule, protector }
    }

    pub fn is_protected(&self, path: &Path, anchor: &Path) -> bool {
        self.protector.is_protected(path, anchor)
    }

    /// Every existing path this rule claims, minus protected ones.
    pub fn matches(&self) -> Vec<Match> {
        let mut out: Vec<Match> = Vec::new();
        for pattern in &self.rule.patterns {
            for matched in expand_pattern(pattern) {
                if self.is_protected(&matched.path, &matched.anchor) {
                    continue;
                }
                if !out.contains(&matched) {
                    out.push(matched);
                }
            }
        }
        out
    }
}

pub fn prepare_all(set: &RuleSet) -> Vec<PreparedRule> {
    set.rules().iter().cloned().map(PreparedRule::new).collect()
}

/// Utility kept for tests and `squid rules --explain`.
pub fn globset_of(patterns: &[String]) -> Option<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    let mut any = false;
    for pattern in patterns {
        if let Ok(glob) = Glob::new(pattern) {
            builder.add(glob);
            any = true;
        }
    }
    if !any {
        return None;
    }
    builder.build().ok()
}

const EMBEDDED_MACOS: &str = include_str!("../rules/macos.toml");
const EMBEDDED_WINDOWS: &str = include_str!("../rules/windows.toml");

#[cfg(test)]
mod tests {
    #[test]
    fn wildcard_free_patterns_match_the_file_not_the_parent() {
        let root = std::env::temp_dir().join(format!(
            "squid-literal-{}",
            squid_core::ledger::new_session_id()
        ));
        if let Err(error) = std::fs::create_dir_all(&root) {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                eprintln!("skipping: scratch write denied ({error})");
                return;
            }
            panic!("scratch dir: {error}");
        }
        let file = root.join(".package-cache");
        std::fs::write(&file, b"x").unwrap();

        let matches = expand_pattern(file.to_str().unwrap());
        assert_eq!(matches.len(), 1, "the file itself must be the match");
        assert_eq!(matches[0].path, file);
        // …and the anchor stays the directory the safety layer vouches for.
        assert_eq!(matches[0].anchor, root);

        let missing = root.join("does-not-exist");
        assert!(expand_pattern(missing.to_str().unwrap()).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    use super::*;

    /// Skips (None) when an endpoint protector refuses scratch writes on a
    /// freshly built unsigned test binary; that is environmental, not a bug.
    fn temp_dir(name: &str) -> Option<PathBuf> {
        let root = std::env::temp_dir().join(format!("squid-rules-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        match std::fs::create_dir_all(&root) {
            Ok(()) => Some(root),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                eprintln!("skipping: scratch write denied ({error})");
                None
            }
            Err(error) => panic!("scratch dir: {error}"),
        }
    }

    #[test]
    fn embedded_rules_parse() {
        let set = RuleSet::builtins();
        assert!(
            set.errors().is_empty(),
            "builtin rules failed: {:?}",
            set.errors()
        );
        assert!(
            set.len() > 20,
            "expected dozens of rules, got {}",
            set.len()
        );
        let mut ids: Vec<&str> = set.rules().iter().map(|r| r.id.as_str()).collect();
        ids.sort();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate rule ids");
    }

    #[test]
    fn every_rule_has_a_safe_anchor() {
        for rule in RuleSet::builtins().rules() {
            for pattern in &rule.patterns {
                let anchor = anchor_for(pattern)
                    .unwrap_or_else(|| panic!("{}: no anchor for {pattern}", rule.id));
                assert!(
                    anchor.is_absolute(),
                    "{}: anchor {anchor:?} is not absolute",
                    rule.id
                );
                assert!(
                    !paths::is_root_like(&anchor),
                    "{}: anchors at a filesystem root",
                    rule.id
                );
            }
        }
    }

    #[test]
    fn windows_rules_parse_on_every_host() {
        // The Windows host check only runs in CI's cross-check job, so the
        // TOML itself must at least parse on every platform.
        let mut set = RuleSet::default();
        set.parse_str(EMBEDDED_WINDOWS, "builtin:windows.toml");
        assert!(
            set.errors().is_empty(),
            "windows rules failed: {:?}",
            set.errors()
        );
        assert!(
            set.len() > 20,
            "expected dozens of rules, got {}",
            set.len()
        );
        let mut ids: Vec<&str> = set.rules().iter().map(|r| r.id.as_str()).collect();
        ids.sort();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate rule ids");
        for rule in set.rules() {
            assert_eq!(
                rule.os,
                vec!["windows".to_string()],
                "{}: wrong os tag",
                rule.id
            );
        }
    }

    #[test]
    fn env_variables_resolve_case_insensitively_and_stop_at_separators() {
        std::env::set_var("SQUID_TEST_VAR", "C:\\Test Root");
        // `$VAR` at the head stops at the first separator, either flavour.
        assert_eq!(
            resolve_variables("$SQUID_TEST_VAR\\sub\\*").as_deref(),
            Some("C:\\Test Root\\sub\\*")
        );
        // Multiple `${VAR}` occurrences all resolve.
        assert_eq!(
            resolve_variables("${SQUID_TEST_VAR}\\a\\${SQUID_TEST_VAR}\\b").as_deref(),
            Some("C:\\Test Root\\a\\C:\\Test Root\\b")
        );
        assert!(resolve_variables("$SQUID_TEST_UNDEFINED\\x").is_none());
        std::env::remove_var("SQUID_TEST_VAR");
    }

    #[cfg(windows)]
    #[test]
    fn windows_anchors_are_never_at_a_drive_root() {
        let mut set = RuleSet::default();
        set.parse_str(EMBEDDED_WINDOWS, "builtin:windows.toml");
        for rule in set.rules() {
            for pattern in &rule.patterns {
                let anchor = anchor_for(pattern)
                    .unwrap_or_else(|| panic!("{}: no anchor for {pattern}", rule.id));
                assert!(
                    anchor.is_absolute(),
                    "{}: anchor {anchor:?} is not absolute",
                    rule.id
                );
                assert!(
                    !paths::is_root_like(&anchor),
                    "{}: anchors at a volume root: {anchor:?}",
                    rule.id
                );
                assert!(
                    anchor.components().count() >= 2,
                    "{}: anchor too shallow: {anchor:?}",
                    rule.id
                );
            }
        }
    }

    #[test]
    fn anchors_handle_slashes_and_literal_paths() {
        assert_eq!(
            anchor_for("/Volumes/*/").unwrap(),
            PathBuf::from("/Volumes")
        );
        assert_eq!(
            anchor_for("~/Library/Caches/*").unwrap(),
            paths::home_dir().join("Library/Caches")
        );
        assert_eq!(
            anchor_for("~/.cargo/.package-cache").unwrap(),
            paths::home_dir().join(".cargo")
        );
        assert_eq!(
            anchor_for("$HOME/go/pkg/mod/cache/*").unwrap(),
            paths::home_dir().join("go/pkg/mod/cache")
        );
    }

    #[test]
    fn wildcards_do_not_cross_separators() {
        let Some(root) = temp_dir("wildcards") else {
            return;
        };
        std::fs::create_dir_all(root.join("s1/deep")).unwrap();
        std::fs::create_dir_all(root.join("other")).unwrap();
        let matches = expand_pattern(&format!("{}/s*/", root.display()));
        assert_eq!(matches.len(), 1, "matches: {matches:?}");
        assert_eq!(matches[0].path, root.join("s1"));
        assert_eq!(matches[0].anchor, root);
        assert_eq!(expand_pattern(&format!("{}/s*/*", root.display())).len(), 1);
        assert_eq!(expand_pattern(&format!("{}/s*", root.display())).len(), 1);
        assert!(expand_pattern(&format!("{}/nope/*", root.display())).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn suffix_globs_match_files() {
        let Some(root) = temp_dir("suffix") else {
            return;
        };
        std::fs::write(root.join("a.dmg"), b"x").unwrap();
        std::fs::write(root.join("b.pkg"), b"x").unwrap();
        std::fs::write(root.join("c.txt"), b"x").unwrap();
        let images = expand_pattern(&format!("{}/*.dmg", root.display()));
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].path, root.join("a.dmg"));
        let all = expand_pattern(&format!("{}/*", root.display()));
        assert_eq!(all.len(), 3);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn protect_blocks_subtrees() {
        let rule = Rule {
            id: "t".into(),
            category: Category::AppCaches,
            title: String::new(),
            patterns: vec!["~/Library/Caches/*".into()],
            mode: RuleMode::Path,
            os: vec![],
            protect: vec!["Homebrew".into(), "com.apple.*".into()],
            min_age_days: 0,
            select_by_default: true,
            needs_admin: false,
            skip_if_running: vec![],
            risky: false,
            note: None,
        };
        let prepared = PreparedRule::new(rule);
        let anchor = paths::home_dir().join("Library/Caches");
        assert!(prepared.is_protected(&anchor.join("Homebrew"), &anchor));
        assert!(prepared.is_protected(&anchor.join("Homebrew/Downloads/x"), &anchor));
        assert!(prepared.is_protected(&anchor.join("com.apple.iconservices"), &anchor));
        assert!(!prepared.is_protected(&anchor.join("Slack/cache"), &anchor));
    }

    #[test]
    fn missing_tool_disarms_pattern() {
        // `brew` may or may not exist; either way expansion must not panic and
        // must never yield a truncated path such as `/Library/Caches/…`.
        for matched in expand_pattern("$(brew --prefix)/Library/Caches/Homebrew/*") {
            assert!(matched
                .anchor
                .to_string_lossy()
                .ends_with("/Library/Caches/Homebrew"));
        }
        assert!(expand_pattern("$(definitely-not-a-real-binary --prefix)/x/*").is_empty());
        assert!(anchor_for("$(definitely-not-a-real-binary --prefix)/x/*").is_none());
    }
}
