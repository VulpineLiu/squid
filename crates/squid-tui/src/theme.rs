//! Colours, glyphs and the two string tables the interface knows.
//!
//! The layout is a deliberate CleanMyMac homage: dark background, one accent
//! colour, rounded cards, a big number where the space was reclaimed. Only the
//! chrome is translated — data (paths, app names) is never rewritten.

use ratatui::style::{Color, Modifier, Style};
use squid_core::settings::{Locale, Settings};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Zh,
}

#[derive(Clone, Debug)]
pub struct Strings {
    pub lang: Lang,
    /// What the host OS calls the place deleted files wait in. Copy about
    /// where a file went, or how to get it back, is a claim about the operating
    /// system, not about the language.
    pub bin_noun: squid_platform::BinNoun,
}

impl Strings {
    pub fn detect(settings: &Settings) -> Strings {
        Strings {
            lang: match Locale::detect(settings) {
                Locale::Zh => Lang::Zh,
                Locale::En => Lang::En,
            },
            bin_noun: squid_platform::current().bin_noun(),
        }
    }

    /// Strings in a chosen language, whatever the terminal and settings say.
    /// The layout tests need to measure both languages on one machine.
    pub fn for_lang(lang: Lang) -> Strings {
        Strings {
            lang,
            bin_noun: squid_platform::current().bin_noun(),
        }
    }

    pub fn t(&self, en: &str, zh: &str) -> String {
        match self.lang {
            Lang::En => en.to_string(),
            Lang::Zh => zh.to_string(),
        }
    }

    /// The host's own word for the recycle bin, in the current language.
    pub fn bin(&self) -> &'static str {
        match self.lang {
            Lang::En => self.bin_noun.en(),
            Lang::Zh => self.bin_noun.zh(),
        }
    }

    /// `t`, with every `{bin}` replaced by the host's word for the recycle
    /// bin. Windows calls it 回收站 / "Recycle Bin"; promising a Windows user
    /// that they can 「清空废纸篓」 to get space back names a control that does
    /// not exist on their machine.
    pub fn bin_t(&self, en: &str, zh: &str) -> String {
        self.t(en, zh).replace("{bin}", self.bin())
    }

    pub fn cleanup(&self) -> String {
        self.t("Cleanup", "清理")
    }

    pub fn uninstall(&self) -> String {
        self.t("Uninstall", "卸载")
    }

    pub fn purge(&self) -> String {
        self.t("Purge", "深度清理")
    }

    pub fn storage(&self) -> String {
        self.t("Storage", "存储空间")
    }

    pub fn system(&self) -> String {
        self.t("System", "系统状态")
    }

    pub fn history(&self) -> String {
        self.t("History", "历史")
    }

    pub fn scan(&self) -> String {
        self.t("Scan", "扫描")
    }

    /// The clean header says how much *could* go, not what already went:
    /// 「已删除」over a list the user has not acted on yet reads as a report of
    /// something that already happened, so the label is 「可删除」.
    pub fn deleted(&self) -> String {
        self.t("deletable", "可删除")
    }

    /// A count with its own shape, so `1634` and `24.3 GB` side by side can
    /// never be read as one broken decimal (`1634·24.3 GB`). Chinese gets a
    /// unit, English gets digit grouping; both stay short enough for a table.
    pub fn count(&self, items: usize) -> String {
        let grouped = squid_core::bytes::format_thousands(items as u64);
        match self.lang {
            Lang::Zh => format!("{grouped} 项"),
            Lang::En => grouped,
        }
    }

    /// The nine group names are ours, not the user's data, so Chinese gets
    /// Chinese names: 「Developer Files」 in the middle of an otherwise Chinese
    /// sidebar is the one place where the interface looked untranslated.
    pub fn category(&self, category: squid_core::Category) -> String {
        use squid_core::Category as C;
        (match self.lang {
            // The recycle bin is the one group named after an OS feature
            // instead of after our own rules, so in either language it takes
            // the operating system's word for it.
            Lang::En => match category {
                C::Trash => self.bin(),
                _ => category.title(),
            },
            Lang::Zh => match category {
                C::SystemJunk => "系统垃圾",
                C::AppCaches => "应用缓存",
                C::BrowserCaches => "浏览器缓存",
                C::Logs => "日志与崩溃报告",
                C::Developer => "开发文件",
                C::ProjectArtifacts => "构建产物",
                C::LanguagePacks => "多余语言包",
                C::Leftovers => "卸载残留",
                C::UpdateInstallers => "升级安装包",
                C::Duplicates => "重复文件",
                C::AiTools => "AI 工具与模型",
                C::Privacy => "隐私痕迹",
                C::Trash => self.bin(),
            },
        })
        .to_string()
    }

    pub fn free(&self) -> String {
        self.t("free", "可用")
    }

    pub fn quit(&self) -> String {
        self.t("quit", "退出")
    }
}

/// How much colour this terminal may be trusted with.
///
/// Decided once, at start-up, by [`color_mode`]. Everything below is authored
/// in 24-bit RGB and degraded on the way out, so the design stays a single
/// source of truth whatever the host can paint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    /// 24-bit RGB, exactly as authored.
    TrueColor,
    /// The xterm 256-colour palette: every RGB value becomes its nearest entry
    /// (grey ramp where the colour is a grey, 6×6×6 cube where it is not).
    Color256,
    /// No usable palette — `NO_COLOR` is set, or the terminal is `dumb` and
    /// nothing else vouched for a palette. Colour is dropped entirely and
    /// emphasis rides on attributes alone.
    Monochrome,
}

impl ColorMode {
    /// The token for the log line the interface opens with.
    pub fn label(self) -> &'static str {
        match self {
            ColorMode::TrueColor => "truecolor",
            ColorMode::Color256 => "256 colors",
            ColorMode::Monochrome => "no colour",
        }
    }

    pub fn is_truecolor(self) -> bool {
        self == ColorMode::TrueColor
    }
}

/// The colours of the interface.
///
/// The palette is authored in 24-bit RGB, because that is the design the
/// reference app is built from. Plenty of terminals cannot parse a
/// `48;2;r;g;b` sequence though — Apple Terminal is the one most people meet
/// first, and it paints nothing at all, which turns a dark, calm dashboard
/// into black-on-white noise. Every colour therefore goes through
/// `Palette::resolve`, which quietly swaps in the closest entry of the
/// xterm-256 palette when the host cannot do truecolour. The RGB values stay
/// the single source of truth either way.
pub struct Palette;

/// The six levels each channel can take in the xterm 6x6x6 colour cube.
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// How far apart the channels may be before a colour counts as grey, in which
/// case the 24-step grey ramp matches it far better than the cube does.
const GREY_LIMIT: i16 = 28;

fn cube_level(channel: u8) -> usize {
    CUBE.iter()
        .enumerate()
        .min_by_key(|(_, step)| (i16::from(channel) - i16::from(**step)).abs())
        .map_or(0, |(index, _)| index)
}

/// The 256-colour index closest to `rgb`.
fn nearest_index(rgb: (u8, u8, u8)) -> u8 {
    let (r, g, b) = rgb;
    if (i16::from(r.max(g).max(b)) - i16::from(r.min(g).min(b))) <= GREY_LIMIT {
        let mean = ((u16::from(r) + u16::from(g) + u16::from(b)) / 3) as u8;
        if (13..=243).contains(&mean) {
            let step = (u16::from(mean).saturating_sub(8).min(230) / 10).min(23) as u8;
            return 232 + step;
        }
    }
    let index = 16 + 36 * cube_level(r) + 6 * cube_level(g) + cube_level(b);
    u8::try_from(index).unwrap_or(255)
}

fn env_is<F: Fn(&str) -> Option<String>>(get: &F, key: &str, wanted: &[&str]) -> bool {
    get(key).is_some_and(|value| wanted.iter().any(|want| value.eq_ignore_ascii_case(want)))
}

/// Decide whether the host terminal may be trusted with 24-bit colour.
///
/// 24-bit colour is only ever handed out on a positive statement: the
/// `SQUID_TRUECOLOR` override, then `COLORTERM`, then the name of the terminal
/// program, then a `TERM` that names 24-bit support itself. Anything else gets
/// the 256-colour palette, which every terminal worth the name has had since
/// about 2008 — the safe guess is the smaller palette, because a terminal that
/// is shown a `48;2;r;g;b` sequence it cannot parse prints the numbers as
/// text. `TERM=xterm-256color` is deliberately not a yes: it is exactly what
/// Apple Terminal exports, and that one paints nothing at all.
fn decide_truecolor(get: impl Fn(&str) -> Option<String>) -> bool {
    if truecolor_refused(&get) {
        return false;
    }
    if matches!(
        get("SQUID_TRUECOLOR").as_deref(),
        Some("always") | Some("on") | Some("1")
    ) {
        return true;
    }
    let get = &get;
    if env_is(get, "COLORTERM", &["truecolor", "24bit"]) {
        return true;
    }
    if env_is(get, "TERM_PROGRAM", &["Apple_Terminal"]) {
        return false;
    }
    if env_is(
        get,
        "TERM_PROGRAM",
        &[
            "iTerm.app",
            "WezTerm",
            "vscode",
            "ghostty",
            "Warp",
            "Hyper",
            "Terminus",
            "contour",
            "foot",
            "rio",
        ],
    ) {
        return true;
    }
    // A `TERM` that says 256 says nothing about 24-bit; only one that says
    // 24-bit is taken as an answer.
    let term = get("TERM").unwrap_or_default().to_lowercase();
    term.contains("truecolor") || term.contains("direct") || term.contains("24bit")
}

/// An explicit `SQUID_TRUECOLOR=never|off|0`.
///
/// Worth its own question, because the terminal library reaches the same
/// conclusion by a different route (`COLORTERM` is exactly what it reads) and
/// would otherwise argue with the one switch the user set on purpose. Saying
/// "no 24-bit" and being answered with 24-bit is how a remote tmux session
/// ends up printing escape sequences as text.
fn truecolor_refused(get: &impl Fn(&str) -> Option<String>) -> bool {
    matches!(
        get("SQUID_TRUECOLOR").as_deref(),
        Some("never") | Some("off") | Some("0")
    )
}

/// Would this terminal be given 24-bit colour, with `colours` standing in for
/// what the terminal library itself reports?
fn decide_mode_with(get: &impl Fn(&str) -> Option<String>, colours: u16) -> ColorMode {
    // An explicit "no colour, thanks" outranks anything the terminal claims,
    // so it is asked before anything else.
    if no_colour_wanted(get) {
        return ColorMode::Monochrome;
    }
    // `TERM_PROGRAM` is missing in a bare ssh session or a fresh tmux server,
    // yet the terminal may still advertise 24-bit through `TERM=xterm-truecolor`
    // or `COLORTERM`. crossterm reads exactly those, and only ever answers with
    // its full count when one of them says so.
    if decide_truecolor(get) || (colours == u16::MAX && !truecolor_refused(get)) {
        return ColorMode::TrueColor;
    }
    // And if nothing at all said anything good about the palette, it is not
    // there: a terminal that will not even name itself gets no colour.
    if term_has_no_palette(get) {
        return ColorMode::Monochrome;
    }
    ColorMode::Color256
}

/// [`decide_mode_with`] with nothing to report from the terminal library.
#[cfg(test)]
fn decide_mode(get: impl Fn(&str) -> Option<String>) -> ColorMode {
    let get = &get;
    decide_mode_with(get, 0)
}

/// `NO_COLOR` (https://no-color.org) means: draw with attributes, not with
/// colour. Any non-empty value counts; an empty one means nothing.
fn no_colour_wanted(get: &impl Fn(&str) -> Option<String>) -> bool {
    get("NO_COLOR").is_some_and(|value| !value.trim().is_empty())
}

/// A terminal that does not even name itself, or names itself `dumb`, has no
/// palette to speak of — as long as nothing else vouched for one.
fn term_has_no_palette(get: &impl Fn(&str) -> Option<String>) -> bool {
    matches!(
        get("TERM").unwrap_or_default().to_lowercase().as_str(),
        "" | "dumb" | "none"
    )
}

/// Read the environment once. Called from [`init`] at start-up so the answer
/// is known before a single cell is painted, not lazily halfway through the
/// first frame.
fn probe() -> ColorMode {
    let get = |key: &str| std::env::var(key).ok();
    let colours = crossterm::style::available_color_count();
    decide_mode_with(&get, colours)
}

/// The colour depth of this process, probed once and remembered.
pub fn color_mode() -> ColorMode {
    #[cfg(test)]
    if let Some(forced) = FORCED.get() {
        return *forced;
    }
    static PROBED: OnceLock<ColorMode> = OnceLock::new();
    *PROBED.get_or_init(probe)
}

/// Pin the palette for the whole process.
///
/// Rendering tests draw a card and then ask what colour a cell came out in,
/// which is a question about the *host*, and the host is whatever terminal
/// `cargo test` happens to be running in — `NO_COLOR`, `TERM=dumb`, CI, an
/// ssh session. They say which depth they mean instead, and every one of them
/// says the same one, so the answer is settled before the first frame.
#[cfg(test)]
pub fn force_color_mode(mode: ColorMode) {
    FORCED.set(mode).ok();
}

#[cfg(test)]
static FORCED: OnceLock<ColorMode> = OnceLock::new();

/// Ask for the colour depth now, rather than on the first colour used.
pub fn init() -> ColorMode {
    color_mode()
}

/// Whether this process will emit 24-bit colour, probed once and remembered.
pub fn truecolor_supported() -> bool {
    color_mode().is_truecolor()
}

impl Palette {
    /// The card behind a list, and the band every second row of it wears.
    const CARD_RGB: (u8, u8, u8) = (24, 27, 33);
    const CARD_ALT_RGB: (u8, u8, u8) = (32, 36, 43);
    /// The page the cards sit on.
    const PAGE_RGB: (u8, u8, u8) = (16, 18, 23);
    /// The grey the cursor row is filled with where there is no accent to use.
    const CURSOR_GREY_RGB: (u8, u8, u8) = (168, 176, 190);

    fn resolve(color: Color) -> Color {
        Self::resolve_in(color, color_mode())
    }

    /// [`Self::resolve`] with the depth stated, so one process can pin down
    /// every rendering at once ([`color_mode`] is cached for its whole life).
    fn resolve_in(color: Color, mode: ColorMode) -> Color {
        match (color, mode) {
            (Color::Rgb(r, g, b), ColorMode::Color256) => Color::Indexed(nearest_index((r, g, b))),
            (_, ColorMode::Monochrome) => Color::Reset,
            _ => color,
        }
    }

    /// Ask for a colour rather than storing one, so the host gets the say.
    fn rgb(r: u8, g: u8, b: u8) -> Color {
        Self::resolve(Color::Rgb(r, g, b))
    }

    fn rgb_in(rgb: (u8, u8, u8), mode: ColorMode) -> Color {
        Self::resolve_in(Color::Rgb(rgb.0, rgb.1, rgb.2), mode)
    }

    /// The band that follows `previous` in a striped list.
    ///
    /// Two nearby greys land on the same entry of the 24-step ramp all the
    /// time — 24/27/33 and 32/36/43 are both "index 234" — and the stripe would
    /// simply not exist. One step further along the ramp is the same idea, only
    /// one the host can actually draw.
    fn band_after(previous: Color, rgb: (u8, u8, u8)) -> Color {
        Self::band_after_in(previous, rgb, color_mode())
    }

    fn band_after_in(previous: Color, rgb: (u8, u8, u8), mode: ColorMode) -> Color {
        let color = Self::rgb_in(rgb, mode);
        match (color, previous) {
            (Color::Indexed(index), Color::Indexed(seen)) if index == seen => {
                Color::Indexed(index.saturating_add(1))
            }
            _ => color,
        }
    }

    pub fn accent() -> Color {
        Self::rgb(86, 182, 246)
    }

    pub fn accent_dim() -> Color {
        Self::rgb(56, 116, 165)
    }

    pub fn background() -> Color {
        Self::rgb_in(Self::PAGE_RGB, color_mode())
    }

    pub fn card_bg() -> Color {
        Self::rgb_in(Self::CARD_RGB, color_mode())
    }

    /// The alternating band of a data list. See [`Palette::band_after`] for why
    /// this is not just another RGB constant.
    pub fn card_bg_alt() -> Color {
        Self::band_after(Self::card_bg(), Self::CARD_ALT_RGB)
    }

    pub fn text() -> Color {
        Self::rgb(224, 228, 236)
    }

    pub fn muted_fg() -> Color {
        Self::rgb(122, 130, 146)
    }

    pub fn good() -> Color {
        Self::rgb(94, 214, 154)
    }

    pub fn warn() -> Color {
        Self::rgb(246, 193, 90)
    }

    pub fn bad() -> Color {
        Self::rgb(240, 118, 118)
    }

    pub fn purple() -> Color {
        Self::rgb(167, 139, 250)
    }

    pub fn teal() -> Color {
        Self::rgb(94, 234, 212)
    }

    pub fn base() -> Style {
        Style::default().fg(Self::text()).bg(Self::background())
    }

    pub fn card() -> Style {
        Style::default().fg(Self::text()).bg(Self::card_bg())
    }

    pub fn title() -> Style {
        Style::default()
            .fg(Self::accent())
            .bg(Self::background())
            .add_modifier(Modifier::BOLD)
    }

    pub fn muted() -> Style {
        Style::default().fg(Self::muted_fg()).bg(Self::background())
    }

    /// The row the keyboard is on.
    ///
    /// Its foreground is authored too: asking for ANSI black would hand the
    /// readable half of the highlight to whatever the user happens to have
    /// mapped to colour 0, and a light-grey "black" makes the cursor row
    /// unreadable on a light-blue bar. Nothing in the theme depends on the
    /// user's palette, which is also why 256 colours are matched by number
    /// rather than by name.
    ///
    /// Where the terminal renders 24-bit colour this is the accent bar of the
    /// reference design. Where it can only do 256 colours — Apple Terminal,
    /// `TERM=xterm`, ssh to an old box — the row is filled with a **grey bar**
    /// instead: the same "one row is lit" reading, with no reliance on the
    /// accent surviving a palette the size of a crayon box. Where there is no
    /// palette at all the row is drawn **inverted**, because reverse video is
    /// the one highlight that needs no colour whatsoever.
    pub fn selected() -> Style {
        Self::selected_for(color_mode())
    }

    /// [`Self::selected`] with the capability stated explicitly, so the tests
    /// can pin down every rendering in one process ([`color_mode`] is decided
    /// once per process and cached).
    pub fn selected_for(mode: ColorMode) -> Style {
        match mode {
            ColorMode::TrueColor => Style::default()
                .fg(Self::rgb_in(Self::PAGE_RGB, mode))
                .bg(Self::accent())
                .add_modifier(Modifier::BOLD),
            ColorMode::Color256 => Style::default()
                .fg(Self::rgb_in(Self::PAGE_RGB, mode))
                .bg(Self::rgb_in(Self::CURSOR_GREY_RGB, mode))
                .add_modifier(Modifier::BOLD),
            ColorMode::Monochrome => {
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
            }
        }
    }

    /// One row of a list on the right of the screen.
    ///
    /// The cursor row is the highlight; the rows around it alternate between
    /// the card and one step lighter, so a row can be followed across a wide
    /// card and the data does not sit as one flat grey slab. `index` is the
    /// index of the row in the list, not on the screen: the band belongs to
    /// the row and stays with it while the list scrolls.
    pub fn row(highlight: bool, index: usize) -> Style {
        if highlight {
            return Self::selected();
        }
        let band = if index % 2 == 0 {
            Self::card_bg()
        } else {
            Self::card_bg_alt()
        };
        Style::default().fg(Self::text()).bg(band)
    }

    /// The list style behind [`Palette::row`]: the widget paints the gaps
    /// between cards, the rows paint everything else.
    pub fn list() -> Style {
        Style::default().bg(Self::card_bg())
    }

    pub fn locked() -> Style {
        Style::default().fg(Self::muted_fg())
    }

    /// Colour a usage ratio the way the system monitor does: cool until 70 %,
    /// amber until 90 %, red beyond.
    pub fn load(ratio: f64) -> Style {
        let color = if ratio > 0.9 {
            Self::bad()
        } else if ratio > 0.7 {
            Self::warn()
        } else {
            Self::good()
        };
        Style::default().fg(color).bg(Self::card_bg())
    }

    pub fn size_color(bytes: squid_core::Bytes) -> Style {
        let mib = bytes.as_u64() / (1024 * 1024);
        if mib >= 1024 {
            Style::default().fg(Self::bad())
        } else if mib >= 128 {
            Style::default().fg(Self::warn())
        } else {
            Style::default().fg(Self::muted_fg())
        }
    }
}

/// Rounded box, the single most recognisable part of the reference UI.
pub fn card(title: &str) -> ratatui::widgets::Block<'static> {
    ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(
            Style::default()
                .fg(Palette::accent_dim())
                .bg(Palette::background()),
        )
        .title_top(format!(" {title} "))
        .title_style(Palette::title())
        .style(Palette::card())
}

pub fn flat_block() -> ratatui::widgets::Block<'static> {
    ratatui::widgets::Block::default().style(Palette::card())
}

/// Checkbox glyph, mirroring the round toggles of the reference app.
pub fn checkbox(selected: bool, locked: bool) -> (&'static str, Style) {
    if locked {
        // U+1F512 looks right and is the one glyph the real Windows console
        // cannot carry: any astral-plane character comes back out of a
        // ConPTY/conhost session as U+FFFD plus the glyph, i.e. one stray
        // cell that shifts the whole row. `⊘` is in the BMP, one cell wide,
        // and was measured to survive the same session.
        ("⊘", Palette::locked())
    } else if selected {
        (
            "●",
            Style::default()
                .fg(Palette::accent())
                .bg(Palette::card_bg())
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            "○",
            Style::default()
                .fg(Palette::muted_fg())
                .bg(Palette::card_bg()),
        )
    }
}

pub const TRIANGLE_OPEN: &str = "▾";
pub const TRIANGLE_CLOSED: &str = "▸";

/// Fit text to `width` display columns, keeping the tail of paths.
pub fn truncate(text: &str, width: usize, keep_tail: bool) -> String {
    use unicode_width::UnicodeWidthChar;
    let target = width.max(1);
    let width_of = |s: &str| -> usize { s.chars().map(|c| c.width().unwrap_or(0)).sum() };
    if width_of(text) <= target {
        return text.to_string();
    }
    if target <= 3 {
        return ".".repeat(target);
    }
    let mut out = String::new();
    if keep_tail {
        let mut budget = target - 3;
        for ch in text.chars().rev() {
            let w = ch.width().unwrap_or(0);
            if w > budget {
                break;
            }
            budget -= w;
            out.insert(0, ch);
        }
        format!("...{out}")
    } else {
        let mut budget = target - 1;
        for ch in text.chars() {
            let w = ch.width().unwrap_or(0);
            if w > budget {
                break;
            }
            budget -= w;
            out.push(ch);
        }
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_respects_the_column_budget() {
        let cut = truncate(
            "/Users/tester/Library/Caches/com.example.heavy.app",
            20,
            true,
        );
        assert!(cut.chars().count() <= 20);
        assert!(cut.starts_with("..."));
        assert!(cut.ends_with("heavy.app"));
        let head = truncate("Browser Caches", 8, false);
        assert!(head.chars().count() <= 8);
        assert!(head.ends_with('…'));
        assert_eq!(truncate("short", 40, false), "short");
    }

    #[test]
    fn wide_characters_count_as_two_columns() {
        let cut = truncate("清理缓存与日志文件", 6, false);
        assert!(cut.chars().count() <= 4, "{cut}");
    }

    #[test]
    fn the_grey_ramp_catches_the_background_tones() {
        assert_eq!(nearest_index((16, 18, 23)), 233);
        assert_eq!(nearest_index((24, 27, 33)), 234);
        assert_eq!(nearest_index((224, 228, 236)), 254);
        assert_eq!(nearest_index((122, 130, 146)), 244);
    }

    #[test]
    fn saturated_colours_take_the_nearest_cube_entry() {
        for rgb in [
            (86, 182, 246),
            (94, 214, 154),
            (246, 193, 90),
            (240, 118, 118),
            (167, 139, 250),
            (94, 234, 212),
        ] {
            let index = nearest_index(rgb);
            assert!((16..232).contains(&index), "{rgb:?} -> {index}");
            let drift = (i32::from(dequantise(index).0) - i32::from(rgb.0)).abs()
                + (i32::from(dequantise(index).1) - i32::from(rgb.1)).abs()
                + (i32::from(dequantise(index).2) - i32::from(rgb.2)).abs();
            assert!(drift <= 96, "{rgb:?} -> {index} drifts by {drift}");
        }
    }

    fn dequantise(index: u8) -> (u8, u8, u8) {
        let index = index - 16;
        (
            CUBE[(index / 36) as usize],
            CUBE[(index % 36 / 6) as usize],
            CUBE[(index % 6) as usize],
        )
    }

    #[test]
    fn apple_terminal_gets_the_fallback_palette() {
        let env = |key: &str| match key {
            "TERM" => Some("xterm-256color".to_string()),
            "TERM_PROGRAM" => Some("Apple_Terminal".to_string()),
            _ => None,
        };
        assert!(!decide_truecolor(env));
        assert_eq!(decide_mode(env), ColorMode::Color256);
    }

    #[test]
    fn modern_terminals_keep_their_full_palette() {
        let env = |key: &str| match key {
            "TERM" => Some("xterm-256color".to_string()),
            "TERM_PROGRAM" => Some("iTerm.app".to_string()),
            _ => None,
        };
        assert!(decide_truecolor(env));
        let env = |key: &str| (key == "COLORTERM").then(|| "truecolor".to_string());
        assert!(decide_truecolor(env));
    }

    /// ...and a terminal that simply never says anything is not given 24-bit
    /// colour on trust. 「不支持 truecolor 就退回 256 色」 is the whole request:
    /// the smaller palette is the safe answer, since a terminal that cannot
    /// parse a truecolour sequence prints it as text.
    #[test]
    fn a_silent_terminal_is_not_given_truecolour_on_trust() {
        for term in ["xterm", "xterm-color", "screen", "rxvt-unicode", "vt100"] {
            let env = move |key: &str| (key == "TERM").then(|| term.to_string());
            assert!(!decide_truecolor(env), "{term} does not advertise 24-bit");
            assert_eq!(decide_mode(env), ColorMode::Color256, "{term}");
        }
        // One that does name 24-bit in `TERM` gets it.
        let env = |key: &str| (key == "TERM").then(|| "xterm-truecolor".to_string());
        assert!(decide_truecolor(env));
        assert_eq!(decide_mode(env), ColorMode::TrueColor);
    }

    /// The answer to 「Terminal.app 里看不出选中，能不能用灰度？」: the cursor row
    /// must survive a terminal with no usable palette.
    #[test]
    fn the_cursor_row_survives_a_colourless_terminal() {
        let rich = Palette::selected_for(ColorMode::TrueColor);
        assert!(
            rich.add_modifier.contains(Modifier::BOLD),
            "the accent bar is bold"
        );
        assert!(
            !rich.add_modifier.contains(Modifier::REVERSED),
            "a capable terminal paints a filled bar instead of inverting"
        );

        let plain = Palette::selected_for(ColorMode::Color256);
        assert!(
            plain.add_modifier.contains(Modifier::BOLD),
            "the grey bar is bold, so it is visible in a greyscale theme too"
        );
        assert!(
            !plain.add_modifier.contains(Modifier::REVERSED),
            "256 colours get a real grey bar, not a terminal-dependent inversion"
        );

        // The two states must be distinguishable even if every colour collapses.
        let grey = Palette::selected_for(ColorMode::Monochrome);
        assert_ne!(
            rich.bg, plain.bg,
            "the fallback may not be merely a different colour"
        );
        assert!(
            grey.add_modifier.contains(Modifier::REVERSED),
            "with no palette at all the row inverts — reverse video needs no colour"
        );
        assert_eq!(grey.bg, None, "and no colour is emitted at all");
    }

    /// Each palette keeps to itself. A 24-bit terminal is handed authored RGB
    /// and nothing else, because an index is a request to look the user's own
    /// palette up; a 256-colour terminal is handed indices and nothing else,
    /// because an RGB triple it cannot parse is printed as text.
    #[test]
    fn each_palette_stays_inside_its_own_sequences() {
        // The palette helpers read the process-wide mode for the colours they
        // author themselves, so the mode is pinned before anything is asked.
        force_color_mode(ColorMode::TrueColor);
        let rich = Palette::selected_for(ColorMode::TrueColor);
        assert!(
            matches!(rich.bg, Some(Color::Rgb(..))),
            "the bar is authored"
        );
        assert!(
            matches!(rich.fg, Some(Color::Rgb(..))),
            "and so is the text on it: {:?}",
            rich.fg
        );

        let plain = Palette::selected_for(ColorMode::Color256);
        assert!(matches!(plain.bg, Some(Color::Indexed(_))));
        assert!(matches!(plain.fg, Some(Color::Indexed(_))));

        // Everything the theme can hand out obeys the same rule.
        for colour in [
            Palette::resolve_in(Color::Rgb(86, 182, 246), ColorMode::TrueColor),
            Palette::resolve_in(Color::Rgb(24, 27, 33), ColorMode::TrueColor),
        ] {
            assert!(matches!(colour, Color::Rgb(..)), "{colour:?}");
        }
        force_color_mode(ColorMode::Color256);
        for colour in [
            Palette::resolve_in(Color::Rgb(86, 182, 246), ColorMode::Color256),
            Palette::resolve_in(Color::Indexed(33), ColorMode::Color256),
        ] {
            assert!(matches!(colour, Color::Indexed(_)), "{colour:?}");
        }

        // A named ANSI colour already *is* a palette entry, in 256 colours as
        // in 24-bit ones, so there is nothing to translate and nothing to
        // garble: it is a `38;5;n` sequence either way, never a triple.
        for mode in [ColorMode::TrueColor, ColorMode::Color256] {
            assert_eq!(
                Palette::resolve_in(Color::LightBlue, mode),
                Color::LightBlue,
                "{mode:?}"
            );
        }
    }

    /// 「256 色时右侧的数据要高亮（灰度）显示」: the two bands a list alternates
    /// between have to survive the collapse to 256 colours, or the rows read as
    /// one flat slab and the cursor row has nothing to stand out from.
    #[test]
    fn the_two_bands_of_a_list_stay_apart_in_256_colours() {
        // The authored greys are one ramp step apart, which is exactly the pair
        // that would otherwise land on the same index.
        let base = nearest_index(Palette::CARD_RGB);
        let alt = nearest_index(Palette::CARD_ALT_RGB);
        assert_eq!(
            base, alt,
            "the test is only worth anything if the naive answer collides"
        );
        let card = Palette::resolve_in(Color::Rgb(24, 27, 33), ColorMode::Color256);
        let band = Palette::band_after_in(card, Palette::CARD_ALT_RGB, ColorMode::Color256);
        assert_ne!(card, band, "the two bands must not be the same colour");
        assert_eq!(band, Color::Indexed(base + 1), "one step up the ramp");
        assert!(
            matches!(band, Color::Indexed(_)),
            "and stay on the grey ramp: {band:?}"
        );

        // The page, the card, and the band are three different steps.
        let page = Palette::resolve_in(Color::Rgb(16, 18, 23), ColorMode::Color256);
        for (left, right) in [(page, card), (card, band), (page, band)] {
            assert_ne!(left, right, "{left:?} and {right:?} would be one colour");
        }

        // And the cursor row is lit against both of them.
        let cursor = Palette::selected_for(ColorMode::Color256).bg;
        assert_ne!(cursor, Some(card));
        assert_ne!(cursor, Some(band));
    }

    /// In a terminal with no palette the stripes must not turn into a wall of
    /// SGR sequences: colour is dropped, and only the attributes are left.
    #[test]
    fn a_monochrome_terminal_gets_no_colour_at_all() {
        assert_eq!(
            Palette::resolve_in(Color::Rgb(86, 182, 246), ColorMode::Monochrome),
            Color::Reset
        );
        let env = |key: &str| (key == "NO_COLOR").then(|| "1".to_string());
        assert_eq!(decide_mode(env), ColorMode::Monochrome);
        let env = |key: &str| (key == "TERM").then(|| "dumb".to_string());
        assert_eq!(decide_mode(env), ColorMode::Monochrome);
        // `NO_COLOR` outranks an emphatic yes from the terminal itself.
        let env = |key: &str| match key {
            "NO_COLOR" => Some("1".to_string()),
            "COLORTERM" => Some("truecolor".to_string()),
            _ => None,
        };
        assert_eq!(decide_mode(env), ColorMode::Monochrome);
        // An empty NO_COLOR means nothing, as the convention says.
        let env = |key: &str| match key {
            "NO_COLOR" => Some("".to_string()),
            "COLORTERM" => Some("truecolor".to_string()),
            _ => None,
        };
        assert_eq!(decide_mode(env), ColorMode::TrueColor);
    }

    /// A terminal that reports 24-bit through its library, with nothing in the
    /// environment to say so, still gets the full palette.
    #[test]
    fn the_terminal_library_can_vouch_for_truecolour() {
        let env = |key: &str| (key == "TERM").then(|| "screen-256color".to_string());
        assert_eq!(decide_mode_with(&env, u16::MAX), ColorMode::TrueColor);
        assert_eq!(decide_mode_with(&env, 256), ColorMode::Color256);
    }

    /// Checked vs unchecked is carried by the glyph and its weight, not only by
    /// its colour, for the same reason.
    #[test]
    fn a_checked_box_differs_from_an_empty_one_without_colour() {
        let (on, on_style) = checkbox(true, false);
        let (off, off_style) = checkbox(false, false);
        assert_ne!(on, off, "the glyphs differ");
        assert!(
            on_style.add_modifier.contains(Modifier::BOLD),
            "the filled box is heavier"
        );
        assert!(!off_style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn the_override_outranks_the_probing() {
        let env = |key: &str| match key {
            "SQUID_TRUECOLOR" => Some("always".to_string()),
            "TERM_PROGRAM" => Some("Apple_Terminal".to_string()),
            _ => None,
        };
        assert!(decide_truecolor(env));
        let env = |key: &str| match key {
            "SQUID_TRUECOLOR" => Some("never".to_string()),
            "COLORTERM" => Some("truecolor".to_string()),
            _ => None,
        };
        assert!(!decide_truecolor(env));
    }

    /// The user's `SQUID_TRUECOLOR=never` has to outrank the terminal library
    /// too — it reads the very `COLORTERM` that made the library say yes, so
    /// asking it afterwards is how the override came to be ignored.
    #[test]
    fn saying_no_outranks_the_terminal_librarys_yes() {
        let env = |key: &str| match key {
            "SQUID_TRUECOLOR" => Some("never".to_string()),
            "COLORTERM" => Some("truecolor".to_string()),
            "TERM" => Some("xterm-256color".to_string()),
            _ => None,
        };
        assert_eq!(
            decide_mode_with(&env, u16::MAX),
            ColorMode::Color256,
            "the palette drops to 256, which is what the override is for"
        );
        let env = |key: &str| match key {
            "SQUID_TRUECOLOR" => Some("always".to_string()),
            "TERM" => Some("xterm".to_string()),
            _ => None,
        };
        assert_eq!(decide_mode_with(&env, 0), ColorMode::TrueColor);
    }

    #[test]
    fn strings_pick_the_requested_language() {
        let mut settings = Settings {
            language: Some("zh".into()),
            ..Settings::default()
        };
        let strings = Strings::detect(&settings);
        assert_eq!(strings.cleanup(), "清理");
        settings.language = Some("en".into());
        assert_eq!(Strings::detect(&settings).cleanup(), "Cleanup");
    }

    /// `1634·24.3 GB` was read as one broken decimal, and 「已删除」over a list
    /// nobody has acted on yet was read as a claim about the past.
    #[test]
    fn counts_and_labels_cannot_be_mistaken_for_a_size() {
        let zh = Strings::for_lang(Lang::Zh);
        let en = Strings::for_lang(Lang::En);
        assert_eq!(zh.count(1634), "1,634 项");
        assert_eq!(en.count(1634), "1,634");
        assert_eq!(zh.count(5), "5 项");
        assert_eq!(zh.deleted(), "可删除");
        assert_eq!(en.deleted(), "deletable");
    }

    /// Nine groups, two languages: the Chinese interface must not fall back to
    /// English halfway, and a name has to survive the 24-column group row.
    #[test]
    fn every_group_has_a_name_in_its_own_language() {
        use squid_core::Category;
        let zh = Strings::for_lang(Lang::Zh);
        let en = Strings::for_lang(Lang::En);
        for category in Category::ALL {
            let name = zh.category(category);
            assert!(!name.is_empty(), "{category:?} lost its name");
            assert!(
                name.chars().any(|c| c as u32 > 0x2fff),
                "{category:?} is still English in Chinese: {name}"
            );
            assert!(
                unicode_width::UnicodeWidthStr::width(name.as_str()) <= 24,
                "{name} will not fit the group row"
            );
            let expected_en = match category {
                // One group is named after an OS feature, so even the English
                // name comes from the operating system: a Windows user has a
                // Recycle Bin and no Trash to open.
                Category::Trash => squid_platform::current().bin_noun().en().to_string(),
                _ => category.title().to_string(),
            };
            assert_eq!(en.category(category), expected_en);
        }
        assert_eq!(zh.category(Category::Developer), "开发文件");
        assert_eq!(zh.category(Category::LanguagePacks), "多余语言包");
    }

    /// Where a deleted file went, and how to get it back, is a claim about the
    /// operating system. Windows has no Trash folder, so copy that promises
    /// 「清空废纸篓」 there sends the user looking for something that is not there.
    #[test]
    fn the_recycle_bin_is_called_what_the_os_calls_it() {
        let en = Strings::for_lang(Lang::En);
        let zh = Strings::for_lang(Lang::Zh);
        let noun = squid_platform::current().bin_noun();
        assert_eq!(en.bin(), noun.en());
        assert_eq!(zh.bin(), noun.zh());
        assert_eq!(
            en.bin_t("moved to {bin}", "移入{bin}"),
            format!("moved to {}", noun.en())
        );
        assert_eq!(
            zh.bin_t("· 清空{bin}后才真正释放", "· 清空{bin}后才真正释放"),
            format!("· 清空{}后才真正释放", noun.zh())
        );
        // The group row is 24 columns; the longest name the OS can hand us has
        // to survive it in both languages.
        for name in [noun.en(), noun.zh()] {
            assert!(
                unicode_width::UnicodeWidthStr::width(name) <= 24,
                "{name} will not fit the group row"
            );
        }
    }
}
