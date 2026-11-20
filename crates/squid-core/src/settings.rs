//! Persistent user settings: whitelist, purge roots, delete semantics.

use crate::bytes::Bytes;
use crate::error::{Result, SquidError};
use crate::model::DeleteMode;
use crate::paths;
use globset::{Glob, GlobMatcher};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Trash or Permanent. CleanMyMac-like safety default is Trash for user
    /// data and Permanent for regenerable caches; the CLI flag overrides both.
    pub delete_mode: DeleteMode,
    /// Glob patterns matched against absolute paths. Matched paths are never
    /// selected, even if a rule finds them.
    pub whitelist: Vec<String>,
    /// Rules the user switched off entirely (`squid clean --disable caches-browser`).
    pub disabled_rules: Vec<String>,
    /// Where `squid purge` looks for projects. `~` is expanded.
    pub purge_roots: Vec<String>,
    /// Skip project artifacts that were touched within this many days: an
    /// active build is not junk yet.
    pub purge_min_age_days: u64,
    /// Cap the file count per project-artifact probe so a huge monorepo does
    /// not stall the scan.
    pub purge_max_entries: u32,
    /// Show allocated (on-disk) size instead of logical size.
    pub show_allocated: bool,
    /// `en` or `zh`. Falls back to $LANG when unset.
    pub language: Option<String>,
    /// Number of history sessions to keep.
    pub history_keep: usize,
    /// Whether the desktop app's first-run permission card has been dismissed.
    /// Older configs have no such field and therefore see the card once.
    pub desktop_setup_reviewed: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            delete_mode: DeleteMode::Trash,
            whitelist: Vec::new(),
            disabled_rules: Vec::new(),
            purge_roots: vec![
                "~/code".into(),
                "~/Code".into(),
                "~/dev".into(),
                "~/Dev".into(),
                "~/develop".into(),
                "~/projects".into(),
                "~/Projects".into(),
                "~/repos".into(),
                "~/Repos".into(),
                "~/src".into(),
                "~/work".into(),
                "~/Work".into(),
                "~/Desktop".into(),
                "~/Documents".into(),
                "~/.codex/worktrees".into(),
            ],
            purge_min_age_days: 7,
            purge_max_entries: 200_000,
            show_allocated: true,
            language: None,
            history_keep: 60,
            desktop_setup_reviewed: false,
        }
    }
}

impl Settings {
    /// `SQUID_CONFIG_DIR` wins: handy for portable installs, CI, and tests that
    /// must not touch the real profile.
    pub fn config_dir() -> PathBuf {
        if let Some(dir) = override_dir("SQUID_CONFIG_DIR") {
            return dir;
        }
        dirs::config_dir()
            .map(|d| d.join("squid"))
            .unwrap_or_else(|| paths::home_dir().join(".config").join("squid"))
    }

    /// `SQUID_STATE_DIR` wins — see `config_dir`.
    pub fn state_dir() -> PathBuf {
        if let Some(dir) = override_dir("SQUID_STATE_DIR") {
            return dir;
        }
        dirs::state_dir()
            .or_else(dirs::config_dir)
            .map(|d| d.join("squid"))
            .unwrap_or_else(|| paths::home_dir().join(".local/state/squid"))
    }

    pub fn path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    /// Never fails: a broken config file is reported through `errors`.
    pub fn load() -> (Settings, Vec<String>) {
        let mut errors = Vec::new();
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Settings>(&text) {
                Ok(settings) => (settings, errors),
                Err(err) => {
                    errors.push(format!(
                        "{}: {err} (falling back to defaults)",
                        path.display()
                    ));
                    (Settings::default(), errors)
                }
            },
            Err(_) => (Settings::default(), errors),
        }
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| SquidError::io(parent, e))?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| SquidError::Other(e.to_string()))?;
        std::fs::write(&path, text).map_err(|e| SquidError::io(&path, e))?;
        Ok(path)
    }

    pub fn add_whitelist(&mut self, pattern: impl Into<String>) {
        let pattern = pattern.into();
        if !self.whitelist.contains(&pattern) {
            self.whitelist.push(pattern);
        }
    }

    pub fn remove_whitelist(&mut self, pattern: &str) -> bool {
        let before = self.whitelist.len();
        self.whitelist.retain(|p| p != pattern);
        self.whitelist.len() != before
    }

    pub fn disable_rule(&mut self, rule_id: impl Into<String>) {
        let id = rule_id.into();
        if !self.disabled_rules.contains(&id) {
            self.disabled_rules.push(id);
        }
    }

    pub fn rule_enabled(&self, rule_id: &str) -> bool {
        !self.disabled_rules.iter().any(|r| r == rule_id)
    }

    pub fn whitelist_matchers(&self) -> Vec<GlobMatcher> {
        self.whitelist
            .iter()
            .filter_map(|raw| {
                let expanded = paths::expand(raw).to_string_lossy().into_owned();
                Glob::new(&expanded).ok().map(|g| g.compile_matcher())
            })
            .collect()
    }

    /// One raw whitelist line as a matcher — the shape the audit layer
    /// needs when it must answer about a single pattern, not the list.
    pub fn whitelist_matcher_for(raw: &str) -> Option<GlobMatcher> {
        let expanded = paths::expand(raw).to_string_lossy().into_owned();
        Glob::new(&expanded).ok().map(|g| g.compile_matcher())
    }

    pub fn is_whitelisted(&self, path: &Path, matchers: &[GlobMatcher]) -> bool {
        if self.whitelist.is_empty() {
            return false;
        }
        let text = path.to_string_lossy();
        matchers.iter().any(|m| m.is_match(&*text))
    }

    pub fn purge_root_paths(&self) -> Vec<PathBuf> {
        self.purge_roots
            .iter()
            .map(|p| paths::expand(p))
            .filter(|p| p.is_dir())
            .collect()
    }
}

/// Locale for UI strings; only two languages exist so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locale {
    En,
    Zh,
}

impl Locale {
    /// Which language the interface should speak.
    ///
    /// 1. `language` in `config.toml` — the user said it out loud.
    /// 2. `SQUID_LANG` (also reachable as `squid --lang zh`).
    /// 3. `LC_ALL`, `LC_MESSAGES`, `LANG`, in that order — the classic
    ///    Posix answer, and the reason iTerm2 shows Chinese: it exports
    ///    `LANG=zh_CN.UTF-8`.
    /// 4. The system language, which Terminal.app never puts in the
    ///    environment. It only exports `LC_CTYPE`, so step 3 has nothing to
    ///    say there and squid used to fall through to English.
    pub fn detect(settings: &Settings) -> Locale {
        Self::explain(settings).0
    }

    /// The language *and* the switch that decided it. `squid doctor` and the
    /// interface log print both, because "this terminal is Chinese and that
    /// one is English" is a question about provenance, not about the answer.
    pub fn explain(settings: &Settings) -> (Locale, LocaleSource) {
        explain_with(settings, &|key| std::env::var(key).ok())
    }

    /// Two-letter tag for reports.
    pub fn code(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::Zh => "zh",
        }
    }
}

/// Which knob picked the language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocaleSource {
    /// `language` in `config.toml`.
    Config,
    /// One of `SQUID_LANG`, `LC_ALL`, `LC_MESSAGES`, `LANG`, named by the one
    /// that answered.
    Environment(&'static str),
    /// The desktop language that never reaches the environment (macOS
    /// keeps it in `defaults`, and Terminal.app exports none of the above).
    System,
    /// Nothing had a say; English is the fallback.
    Fallback,
}

impl LocaleSource {
    pub fn describe(self) -> String {
        match self {
            LocaleSource::Config => "config.toml `language`".to_string(),
            LocaleSource::Environment(key) => format!("environment ${key}"),
            LocaleSource::System => "system language setting".to_string(),
            LocaleSource::Fallback => "built-in default".to_string(),
        }
    }

    pub fn describe_zh(self) -> String {
        match self {
            LocaleSource::Config => "config.toml 的 language".to_string(),
            LocaleSource::Environment(key) => format!("环境变量 {key}"),
            LocaleSource::System => "系统语言设置".to_string(),
            LocaleSource::Fallback => "内置默认值".to_string(),
        }
    }
}

/// The documented order, with `get` standing in for the environment, and the
/// name of whatever answered kept next to the answer.
fn explain_with(
    settings: &Settings,
    get: &dyn Fn(&str) -> Option<String>,
) -> (Locale, LocaleSource) {
    if let Some(locale) = settings.language.as_deref().and_then(parse_locale) {
        return (locale, LocaleSource::Config);
    }
    if let Some((key, locale)) = answered_by(get) {
        return (locale, LocaleSource::Environment(key));
    }
    if let Some(locale) = system_locale() {
        return (locale, LocaleSource::System);
    }
    (Locale::En, LocaleSource::Fallback)
}

/// Read one locale token (`zh_CN.UTF-8`, `C`, `POSIX`, `en-GB`).
/// `None` means the token carries no language information, so the search
/// moves on to the next variable instead of answering English by default.
fn parse_locale(raw: &str) -> Option<Locale> {
    let token = raw.trim().to_lowercase().replace('-', "_");
    if token.is_empty() || is_neutral(&token) {
        return None;
    }
    if token.starts_with("zh") {
        return Some(Locale::Zh);
    }
    Some(Locale::En)
}

/// The C locale and its aliases describe an encoding, not a language.
fn is_neutral(token: &str) -> bool {
    token == "c" || token == "posix" || token.starts_with("c.") || token.starts_with("c_")
}

/// The variables that may carry a language, highest authority first.
const ENV_KEYS: [&str; 4] = ["SQUID_LANG", "LC_ALL", "LC_MESSAGES", "LANG"];

/// The first variable that carries a language, named.
fn answered_by(get: &dyn Fn(&str) -> Option<String>) -> Option<(&'static str, Locale)> {
    ENV_KEYS.iter().find_map(|key| {
        get(key)
            .as_deref()
            .and_then(parse_locale)
            .map(|locale| (*key, locale))
    })
}

/// The environment, read through `get` so the rule can be tested without
/// mutating the process.
#[cfg(test)]
fn from_variables(get: impl Fn(&str) -> Option<String>) -> Option<Locale> {
    answered_by(&get).map(|(_, locale)| locale)
}

/// Ask the operating system what language the user picked. macOS keeps that
/// in `defaults`, not in the environment, and Terminal.app exports neither.
#[cfg(target_os = "macos")]
fn system_locale() -> Option<Locale> {
    for key in ["AppleLocale", "AppleLanguages"] {
        let Ok(output) = std::process::Command::new("defaults")
            .args(["read", "-g", key])
            .output()
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        // `AppleLanguages` is a property list — ("zh-Hans-CN",) — so the
        // first quoted (or bare) token is the preferred language.
        for piece in text.split(['"', ',', '(', ')']) {
            let piece = piece.trim();
            if piece.is_empty() {
                continue;
            }
            if let Some(locale) = parse_locale(piece) {
                return Some(locale);
            }
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn system_locale() -> Option<Locale> {
    // Linux and Windows describe the language through the environment only,
    // and step 3 of `detect` has already read every bit of it.
    None
}

/// Convenience for reports that want the "freed" number in display units.
pub fn display_size(settings: &Settings, logical: Bytes, allocated: Bytes) -> Bytes {
    if settings.show_allocated && !allocated.is_zero() {
        allocated
    } else {
        logical
    }
}

fn override_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    /// The useful half of the answer is *which* switch said it: the same
    /// binary was Chinese in iTerm2 and English in Terminal.app, and every
    /// explanation of that starts with where the language came from.
    #[test]
    fn the_language_names_the_switch_that_decided_it() {
        let get = env(&[("LANG", "zh_CN.UTF-8")]);
        assert_eq!(
            explain_with(&Settings::default(), &get),
            (Locale::Zh, LocaleSource::Environment("LANG")),
            "iTerm2 exports LANG, Terminal.app does not"
        );
        let get = env(&[("SQUID_LANG", "en"), ("LANG", "zh_CN.UTF-8")]);
        assert_eq!(
            explain_with(&Settings::default(), &get),
            (Locale::En, LocaleSource::Environment("SQUID_LANG"))
        );
        let settings = Settings {
            language: Some("en".into()),
            ..Settings::default()
        };
        assert_eq!(
            explain_with(&settings, &get),
            (Locale::En, LocaleSource::Config),
            "the config file outranks the environment"
        );
        assert_eq!(
            LocaleSource::Environment("LANG").describe_zh(),
            "环境变量 LANG"
        );
    }

    #[test]
    fn a_locale_variable_picks_the_language() {
        assert_eq!(
            from_variables(env(&[("LANG", "zh_CN.UTF-8")])),
            Some(Locale::Zh)
        );
        assert_eq!(
            from_variables(env(&[("LC_ALL", "zh-Hans-CN")])),
            Some(Locale::Zh)
        );
        assert_eq!(
            from_variables(env(&[("LANG", "en_GB.UTF-8")])),
            Some(Locale::En)
        );
    }

    /// Terminal.app exports none of these; it only sets `LC_CTYPE`. Treating
    /// "anything that is not zh" as English is what made the same build
    /// answer Chinese in iTerm2 and English in Terminal.
    #[test]
    fn the_c_locale_carries_no_language() {
        for value in ["C", "c", "POSIX", "C.UTF-8", ""] {
            assert_eq!(
                from_variables(env(&[("LANG", value), ("LC_ALL", value)])),
                None,
                "{value} must not decide the language"
            );
        }
        assert_eq!(from_variables(env(&[])), None, "no variables at all");
        assert_eq!(
            from_variables(env(&[("LC_CTYPE", "C.UTF-8")])),
            None,
            "LC_CTYPE is not a language"
        );
    }

    /// Empty variables keep the search walking down the list instead of
    /// answering on the first blank hit.
    #[test]
    fn an_empty_variable_does_not_shadow_a_later_one() {
        assert_eq!(
            from_variables(env(&[
                ("LC_ALL", ""),
                ("LC_MESSAGES", ""),
                ("LANG", "zh_CN.UTF-8")
            ])),
            Some(Locale::Zh)
        );
    }

    #[test]
    fn squid_lang_beats_the_desktop_locale() {
        assert_eq!(
            from_variables(env(&[("SQUID_LANG", "en"), ("LANG", "zh_CN.UTF-8")])),
            Some(Locale::En)
        );
        assert_eq!(
            from_variables(env(&[("SQUID_LANG", "zh-CN"), ("LANG", "en_US.UTF-8")])),
            Some(Locale::Zh)
        );
    }

    #[test]
    fn the_config_file_outvotes_everything_else() {
        let language = |value: &str| Settings {
            language: Some(value.into()),
            ..Settings::default()
        };
        assert_eq!(Locale::detect(&language("zh")), Locale::Zh);
        assert_eq!(Locale::detect(&language("en")), Locale::En);
        let settings = language("");
        // Unset means "ask the machine", never "assume English".
        assert_eq!(
            Locale::detect(&settings),
            Locale::detect(&Settings::default())
        );
    }

    #[test]
    fn an_existing_config_has_not_reviewed_the_desktop_setup() {
        let older = "history_keep = 60\n";
        let restored: Settings = toml::from_str(older).unwrap();
        assert!(!restored.desktop_setup_reviewed);
    }
}
