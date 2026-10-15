//! Every pixel of the interface, and nothing else.
//!
//! Nothing in this file mutates [`App`] and nothing touches the disk: it reads
//! state and draws it. The layout follows the reference app — a coloured card
//! per group, one accent colour, a big number for the space you get back.

use crate::app::{
    App, Confirm, Line as AppLine, Panel, ScanPanel, Toast, ToastKind, UninstallView,
};
use crate::theme::{
    card as theme_card, checkbox, truncate, Palette, TRIANGLE_CLOSED, TRIANGLE_OPEN,
};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, List, ListState, Paragraph, Sparkline, Wrap};
use ratatui::Frame;
use squid_core::{Bytes, Category, CleanupItem};
use squid_platform::Device;
use unicode_width::UnicodeWidthStr;

// ------------------------------------------------------------------- helpers

fn block(title: &str) -> Block<'static> {
    theme_card(title)
}

fn plain_block() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(Palette::accent_dim()))
        .style(Palette::card())
}

fn pad(text: &str, width: usize) -> String {
    let used = text.width();
    if used >= width {
        truncate(text, width, false)
    } else {
        format!("{text}{}", " ".repeat(width - used))
    }
}

fn right(text: &str, width: usize) -> String {
    let used = text.width();
    if used >= width {
        truncate(text, width, false)
    } else {
        format!("{}{text}", " ".repeat(width - used))
    }
}

fn spans_width(spans: &[Span<'static>]) -> usize {
    spans.iter().map(|span| span.content.width()).sum()
}

/// Left-aligned spans, then filler, then right-aligned spans.
///
/// When the two halves do not fit, the left one gives ground. A row that spills
/// puts them back to back and the terminal cuts off whatever came last, so the
/// right half — the state the user is acting on, like the delete mode — is the
/// part that disappears. That is how "empties only with the Recycle Bin"
/// swallowed "delete to · Recycle Bin" on a Windows box at 100 columns: the
/// operating system's name for the recycle bin is longer there than on macOS.
fn joined(left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let room = width.saturating_sub(spans_width(&right) + 1);
    let left = clip_left(left, room);
    let gap = width.saturating_sub(spans_width(&left) + spans_width(&right));
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(gap)));
    spans.extend(right);
    close_gaps(&mut spans);
    Line::from(spans)
}

/// Fit the left half of a row into `room` columns, giving up whole trailing
/// spans before cutting one in half, so an ellipsis lands between words where
/// it can.
fn clip_left(left: Vec<Span<'static>>, room: usize) -> Vec<Span<'static>> {
    if spans_width(&left) <= room {
        return left;
    }
    let mut kept: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for span in left {
        let wide = span.content.width();
        if used + wide > room {
            let tail = room - used;
            if tail > 1 {
                let cut = crate::theme::truncate(&span.content, tail, false);
                kept.push(Span::styled(cut, span.style));
            }
            break;
        }
        used += wide;
        kept.push(span);
    }
    kept
}

/// Give the unstyled spans of a row the row's own style.
///
/// The columns of a row are separated by plain `Span::raw(" ")`, which inherit
/// the card background, and `joined` adds a filler span the same way. On a
/// highlighted row that leaves thin unhighlighted slits through the bar: easy
/// to miss in colour, but in a terminal without a usable palette — where the
/// cursor is drawn as reverse video — the bar would come out striped. When
/// every visible span already agrees on one style, that style *is* the row's
/// style, so the gaps get it too.
fn close_gaps(spans: &mut [Span<'static>]) {
    let mut shared: Option<Style> = None;
    for span in spans.iter() {
        if span.style == Style::new() || span.content.trim().is_empty() {
            continue;
        }
        match shared {
            None => shared = Some(span.style),
            Some(known) if known == span.style => {}
            // Mixed styles: this row is not one flat bar, leave it alone.
            Some(_) => return,
        }
    }
    let Some(style) = shared else { return };
    for span in spans.iter_mut() {
        if span.style == Style::new() {
            span.style = style;
        }
    }
}

fn bar_text(ratio: f64, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let filled = ((ratio.clamp(0.0, 1.0)) * width as f64).round() as usize;
    let mut out = String::with_capacity(width);
    for index in 0..width {
        out.push(if index < filled { '█' } else { '░' });
    }
    out
}

fn dim(highlight: bool, fg: ratatui::style::Color) -> Style {
    if highlight {
        Palette::selected()
    } else {
        Style::default().fg(fg)
    }
}

/// The style one row of the data pane wears.
///
/// The row under the cursor is the highlight; the rows around it alternate
/// between the card and one step lighter. That does two jobs. A wide row can be
/// followed to its right-hand column, and on a terminal that can only paint 256
/// colours — where the accent is an educated guess and the cursor is a grey bar
/// — the bands are emphasis that never had to survive a small palette. Without
/// them a list is one flat slab with one bright row in it.
fn band(row: usize, highlight: bool) -> Style {
    Palette::row(highlight, row)
}

fn empty_hint(message: &str, sub: &str) -> Paragraph<'static> {
    Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {message}"),
            Style::default()
                .fg(Palette::accent())
                .bg(Palette::card_bg())
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("  {sub}"),
            Style::default()
                .fg(Palette::muted_fg())
                .bg(Palette::card_bg()),
        )),
    ])
    .style(Palette::card())
}

// ---------------------------------------------------------------------- root

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 62 || area.height < 10 {
        frame.render_widget(
            Paragraph::new("terminal too small — resize to at least 62×10")
                .style(Palette::base())
                .alignment(Alignment::Center),
            area,
        );
        return;
    }
    let chunks = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(5),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);

    draw_header(frame, chunks[0], app);
    let body = if area.width >= 104 {
        let side =
            Layout::horizontal([Constraint::Length(30), Constraint::Min(24)]).split(chunks[1]);
        draw_sidebar(frame, side[0], app);
        side[1]
    } else {
        chunks[1]
    };
    draw_panel(frame, body, app);
    draw_progress(frame, chunks[2], app);
    draw_hints(frame, chunks[3], app);

    // The toast goes under the popups, not over them: a four-second chip must
    // never be the reason a dialog loses an edge.
    if let Some(toast) = &app.toast {
        draw_toast(frame, body, toast);
    }
    if app.help {
        draw_help(frame, area, app);
    }
    if let Some(confirm) = &app.confirm {
        draw_confirm(frame, area, confirm, app);
    }
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let card = block(" squid ");
    let inner = card.inner(area);
    frame.render_widget(card, area);

    let volume = app
        .storage
        .volumes
        .iter()
        .find(|volume| volume.is_root)
        .or_else(|| app.storage.volumes.first());
    let mut right_top = Vec::new();
    if let Some(volume) = volume {
        right_top.push(Span::styled(
            format!(
                "{}  {} / {}",
                volume.name,
                volume.free.human(),
                volume.total.human()
            ),
            Palette::muted(),
        ));
    }
    let top = joined(
        vec![
            Span::styled(
                format!(
                    " squid {} ",
                    option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0")
                ),
                Style::default()
                    .fg(Palette::background())
                    .bg(Palette::accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(squid_core::TAGLINE, Palette::muted()),
        ],
        right_top,
        inner.width as usize,
    );

    let mode = match app.delete_mode {
        squid_core::DeleteMode::Trash => app.strings.bin().to_string(),
        squid_core::DeleteMode::Permanent => app.strings.t("Permanent", "永久删除"),
    };
    let mut bottom_right = vec![Span::styled(
        format!("{} · {}", app.strings.t("delete to", "删除方式"), mode),
        Palette::muted(),
    )];
    // "进行中" over a disk map nobody asked for reads as "it is still deleting
    // what I already deleted". Say which of the two the pill means.
    let work = if app.jobs.foreground() {
        Some(app.strings.t("working", "进行中"))
    } else if app.busy() {
        Some(app.strings.t("background scan", "后台扫描"))
    } else {
        None
    };
    if let Some(work) = work {
        bottom_right.insert(
            0,
            Span::styled(
                format!("{}  ", work),
                Style::default()
                    .fg(Palette::warn())
                    .bg(Palette::background()),
            ),
        );
    }
    // The hero number is the one figure on this screen that must not round
    // itself up. Bytes moved to the Trash are still on the disk — `df` will
    // not move until the Trash is emptied — so calling 27 GB of Trash
    // content "already reclaimed" tells the user the free space they can see
    // is wrong. Either the label or the number has to change.
    let pending = app.to_trash;
    let all_trash = !pending.is_zero() && pending == app.reclaimed;
    let label = if all_trash {
        app.strings.bin_t("moved to {bin}", "移入{bin}")
    } else {
        app.strings.t("reclaimed", "已释放")
    };
    let mut bottom_left = vec![
        Span::styled(
            format!(" {}", app.reclaimed.human_long()),
            Style::default()
                .fg(Palette::good())
                .bg(Palette::background())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {label} · {}", app.sessions), Palette::muted()),
    ];
    let mut note = None;
    if !pending.is_zero() {
        let text = if all_trash {
            app.strings
                .bin_t("· empties only with the {bin}", "· 清空{bin}后才真正释放")
        } else {
            app.strings.t(
                &format!("· {} still in {{bin}}", pending.human_fixed()),
                &format!("· {} 仍在{{bin}}", pending.human_fixed()),
            )
        };
        note = Some(Span::styled(
            format!(" {text}"),
            Style::default().fg(Palette::warn()),
        ));
    }
    // The note explains the number; the right half states what the user is
    // about to do. Given a row too narrow for both, the explanation yields and
    // says nothing rather than pushing the delete mode off the card.
    if let Some(note) = note {
        if spans_width(&bottom_left) + spans_width(&bottom_right) + note.content.width()
            <= inner.width as usize
        {
            bottom_left.push(note);
        }
    }
    let bottom = joined(bottom_left, bottom_right, inner.width as usize);
    frame.render_widget(
        Paragraph::new(vec![top, bottom]).style(Palette::base()),
        inner,
    );
}

fn panel_total(app: &App, panel: Panel) -> Option<(Bytes, usize)> {
    match panel {
        Panel::Cleanup => {
            if app.clean.items.is_empty() {
                None
            } else {
                Some((app.clean.selected_size(), app.clean.selected_count()))
            }
        }
        Panel::Purge => {
            if app.purge.items.is_empty() {
                None
            } else {
                Some((app.purge.selected_size(), app.purge.selected_count()))
            }
        }
        Panel::Uninstall => app.uninstall.plan.as_ref().map(|_| {
            (
                app.uninstall.plan_selected_size(),
                app.uninstall
                    .plan
                    .as_ref()
                    .map(|plan| {
                        plan.items
                            .iter()
                            .filter(|item| item.selected && item.lock.is_none())
                            .count()
                    })
                    .unwrap_or(0),
            )
        }),
        _ => None,
    }
}

fn draw_sidebar(frame: &mut Frame, area: Rect, app: &App) {
    let host = std::env::var("USER").unwrap_or_else(|_| "you".to_string());
    let card = block(&format!(" {host} "));
    let inner = card.inner(area);
    frame.render_widget(card, area);

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (index, panel) in Panel::ALL.iter().enumerate() {
        let active = *panel == app.panel;
        let base = if active {
            Palette::selected()
        } else {
            Style::default().fg(Palette::text()).bg(Palette::card_bg())
        };
        let (size, count) = panel_total(app, *panel).unwrap_or((Bytes::ZERO, 0));
        // Reversed tells you which panel you are *in*; the brackets tell you
        // the arrow keys are pointing at it. Both read in a terminal without
        // colour, which is the whole point of the sidebar being reachable.
        let badge = if active && app.menu {
            format!("[{}]", index + 1)
        } else {
            format!(" {} ", index + 1)
        };
        let left = vec![
            Span::styled(badge, base),
            Span::styled(pad(&panel.title(&app.strings), 12), base),
        ];
        let right = if count > 0 {
            // The card is 27 columns wide including the panel name, so a badge
            // has to decide what it is for. `1634·1.8 GB` was the worst of
            // both: too wide to be true and a dot sitting where a decimal
            // point belongs. Spell the count out when there is room for it,
            // and when there is not, keep the size — the count is in the
            // panel header, where it has always had room to breathe.
            let room = (inner.width as usize).saturating_sub(spans_width(&left));
            let spelled = format!("{} · {}", app.strings.count(count), size.human_fixed());
            let badge = if spelled.width() <= room {
                spelled
            } else {
                size.human_fixed()
            };
            vec![Span::styled(
                truncate(&badge, room, false),
                if active {
                    Palette::selected()
                } else {
                    Style::default()
                        .fg(Palette::accent())
                        .bg(Palette::card_bg())
                },
            )]
        } else {
            Vec::new()
        };
        lines.push(joined(left, right, inner.width as usize));
    }
    lines.push(Line::from(""));

    let scan = match app.panel {
        Panel::Purge => Some(&app.purge),
        Panel::Cleanup => Some(&app.clean),
        _ => None,
    };
    if let Some(scan) = scan {
        if !scan.buckets.is_empty() {
            lines.push(Line::from(Span::styled(
                format!(" {}", app.strings.t("groups", "分组")),
                Palette::title(),
            )));
            let mut buckets: Vec<(Category, Bytes, usize)> = scan
                .buckets
                .iter()
                .map(|(category, (total, count))| (*category, *total, *count))
                .collect();
            buckets.sort_by_key(|entry| std::cmp::Reverse(entry.1));
            for (category, total, _count) in buckets.into_iter().take(9) {
                // Twenty-six columns hold a name and a size, not a name, a
                // size and a count: 「Developer Files 5 项 · 13.0」 was the
                // 「13.0 GB」 with its unit bitten off. The count lives on the
                // group row in the panel, where there is room for it.
                let size = total.human();
                let room = (inner.width as usize).saturating_sub(size.width() + 1);
                lines.push(joined(
                    vec![Span::styled(
                        truncate(&app.strings.category(category), room, true),
                        Style::default()
                            .fg(Palette::muted_fg())
                            .bg(Palette::card_bg()),
                    )],
                    vec![Span::styled(size, Palette::size_color(total))],
                    inner.width as usize,
                ));
            }
            lines.push(Line::from(""));
        }
    }

    if !app.storage.volumes.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(" {}", app.strings.t("disks", "磁盘")),
            Palette::title(),
        )));
        for volume in app.storage.volumes.iter().take(4) {
            let used = volume.used();
            let ratio = if volume.total.as_u64() == 0 {
                0.0
            } else {
                used.as_u64() as f64 / volume.total.as_u64() as f64
            };
            // A second disk in the list is not a second look at the same disk.
            // 「DATA 可用 503 GB」 next to a 256 GB Mac reads as a broken number
            // until the row says which of the two is the one you are cleaning.
            let tag = if volume.is_root {
                app.strings.t("built-in", "内置")
            } else if volume.is_removable {
                app.strings.t("external", "外置")
            } else {
                String::new()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    pad(&volume.name, 18),
                    Style::default().fg(Palette::text()).bg(Palette::card_bg()),
                ),
                Span::styled(tag, Palette::muted()),
            ]));
            lines.push(Line::from(Span::styled(
                format!(
                    "{} {}",
                    bar_text(ratio, 14),
                    app.strings.t(
                        &format!("{} free", volume.free.human()),
                        &format!("可用 {}", volume.free.human())
                    )
                ),
                Palette::load(ratio),
            )));
        }
    }
    frame.render_widget(Paragraph::new(lines).style(Palette::card()), inner);
}

fn draw_progress(frame: &mut Frame, area: Rect, app: &App) {
    if app.busy() && !app.phase.is_empty() {
        let progress = app.phase.clone();
        let ratio = match app.total {
            Some(total) if total > 0 => app.done as f64 / total as f64,
            _ => 0.0,
        };
        let label = match app.total {
            Some(total) if total > 0 => format!(
                "{}  {}/{}  {}",
                progress,
                squid_core::bytes::format_thousands(app.done),
                squid_core::bytes::format_thousands(total),
                app.subject
                    .as_ref()
                    .map(|path| truncate(&squid_core::paths::pretty(path), 48, true))
                    .unwrap_or_default()
            ),
            _ => format!(
                "{progress}  {}",
                app.subject
                    .as_ref()
                    .map(|path| truncate(&squid_core::paths::pretty(path), 60, true))
                    .unwrap_or_default()
            ),
        };
        let gauge = Gauge::default()
            .ratio(if app.total.is_some() { ratio } else { 0.0 })
            .gauge_style(
                Style::default()
                    .fg(Palette::accent())
                    .bg(Palette::card_bg()),
            )
            .style(Style::default().fg(Palette::text()).bg(Palette::card_bg()))
            .label(format!(" {label} "))
            .use_unicode(true);
        frame.render_widget(gauge, area);
        return;
    }
    let tail = app
        .log
        .back()
        .cloned()
        .unwrap_or_else(|| format!("squid {} · {}", squid_core::VERSION, squid_core::TAGLINE));
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(
                " {}",
                truncate(&tail, area.width.saturating_sub(2) as usize, true)
            ),
            Palette::muted(),
        )))
        .style(Palette::base()),
        area,
    );
}

/// The key hints for the panel on screen, as one line.
///
/// Two lies lived in this bar. `u 还原` was printed twice — once in the fixed
/// list and again as the history panel's own hint — and the twelve hints were a
/// fixed list whatever the panel, so `s 扫描` was advertised on the history
/// screen and `q 退出` was the piece that fell off the end of a 112-column
/// terminal. The bar now lists the keys *this* panel answers, in the order the
/// user meets them, and a test keeps it inside the window.
fn hint_text(app: &App) -> String {
    let s = &app.strings;
    let mut parts: Vec<String> = Vec::new();
    if app.menu {
        parts.push(s.t("↑↓ choose a function", "↑↓ 选择功能"));
        parts.push(s.t("enter back to the list", "回车回到列表"));
        parts.push(s.t("1-6 jump", "1-6 直达"));
        parts.push(s.t("Tab panel", "Tab 切换"));
        parts.push(s.t("m mode", "m 方式"));
    } else {
        parts.push(s.t("Tab", "Tab"));
        parts.push(s.t("esc back/menu", "esc 返回/菜单"));
        match app.panel {
            Panel::Cleanup | Panel::Purge => {
                parts.push(s.t("s scan", "s 扫描"));
                parts.push(s.t("a all", "a 全选"));
                parts.push(s.t("space select · d clean", "空格选择 · d 清理"));
            }
            Panel::Uninstall => {
                // No `a all` here even though the key works: this is the one
                // panel whose hints already fill a 112-column bar, and
                // selecting 200 applications at once is not something to
                // advertise one keystroke away from an uninstall.
                parts.push(s.t("s scan", "s 扫描"));
                parts.push(s.t("enter view · d uninstall", "回车查看 · d 卸载"));
                parts.push(s.t("f filter", "f 筛选"));
            }
            Panel::Storage => {
                parts.push(s.t("enter open · f files", "回车进入 · f 大文件"));
                parts.push(s.t("r refresh", "r 刷新"));
            }
            Panel::Stats => parts.push(s.t("r refresh", "r 刷新")),
            Panel::History => {
                parts.push(s.t("u restore", "u 还原"));
                parts.push(s.t("r refresh", "r 刷新"));
            }
        }
        // The two keys every panel except these answers.
        if !matches!(app.panel, Panel::Stats | Panel::History) {
            parts.push(s.t("m mode", "m 方式"));
            parts.push(s.t("u undo", "u 还原"));
        }
    }
    if app.busy() {
        parts.push(s.t("c cancel", "c 取消"));
    }
    parts.push(s.t("? help", "? 帮助"));
    format!(" {} · q {}", parts.join(" · "), s.quit())
}

fn draw_hints(frame: &mut Frame, area: Rect, app: &App) {
    let style = if app.menu {
        Style::default()
            .fg(Palette::accent())
            .bg(Palette::background())
    } else {
        Palette::muted()
    };
    let text = hint_text(app);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncate(&text, area.width.saturating_sub(1) as usize, false),
            style,
        )))
        .style(Palette::base()),
        area,
    );
}

// --------------------------------------------------------------- panel list

fn draw_panel(frame: &mut Frame, area: Rect, app: &App) {
    match app.panel {
        Panel::Cleanup => {
            let title = app.strings.cleanup();
            let filling = app.filling(Panel::Cleanup);
            draw_scan(
                frame,
                area,
                &app.clean,
                title,
                app.clean.cursor,
                filling,
                app,
            )
        }
        Panel::Purge => {
            let title = app.strings.purge();
            let filling = app.filling(Panel::Purge);
            draw_scan(
                frame,
                area,
                &app.purge,
                title,
                app.purge.cursor,
                filling,
                app,
            )
        }
        Panel::Uninstall => draw_uninstall(frame, area, app),
        Panel::Storage => draw_storage(frame, area, app),
        Panel::Stats => draw_stats(frame, area, app),
        Panel::History => draw_history(frame, area, app),
    }
}

fn draw_scan(
    frame: &mut Frame,
    area: Rect,
    scan: &ScanPanel,
    title: String,
    cursor: usize,
    filling: bool,
    app: &App,
) {
    let selected = scan.selected_size();
    let total = scan.total_size();
    let heading = if scan.items.is_empty() {
        format!(
            " {} · {} ",
            title,
            if filling {
                app.strings.t("scanning…", "正在扫描…")
            } else {
                app.strings.t("not scanned — press s", "尚未扫描 — 按 s")
            }
        )
    } else {
        format!(
            " {} · {} · {} {} / {} ",
            title,
            app.strings.count(scan.items.len()),
            app.strings.deleted(),
            selected.human_fixed(),
            total.human_fixed()
        )
    };
    let card = block(&heading);
    let inner = card.inner(area);
    frame.render_widget(card, area);
    if scan.items.is_empty() {
        // Nothing on the list is either "still working" or "nothing to find",
        // and the two need to be told apart: one asks for patience, the other
        // for a key press.
        // 「this Mac」 on a Windows box is a copy leak rather than a
        // translation problem, so the noun comes from the platform and each
        // platform owns the spacing of its own phrase.
        let (scanning, scanned) = match app.engine.platform().device() {
            Device::Mac => ("Scanning this Mac…", "press s to scan this Mac"),
            Device::Pc => ("Scanning this PC…", "press s to scan this PC"),
            Device::Machine => ("Scanning this machine…", "press s to scan this machine"),
        };
        let (scanning_zh, scanned_zh) = match app.engine.platform().device() {
            Device::Mac => ("正在扫描这台 Mac…", "按 s 扫描这台 Mac"),
            Device::Pc => ("正在扫描这台电脑…", "按 s 扫描这台电脑"),
            Device::Machine => ("正在扫描这台机器…", "按 s 扫描这台机器"),
        };
        let (title, hint) = if filling {
            (
                app.strings.t(scanning, scanning_zh),
                app.strings
                    .t("hold on, or press c to cancel", "请稍候，或按 c 取消"),
            )
        } else {
            (
                app.strings.t("Nothing scanned yet", "尚未扫描"),
                app.strings.t(scanned, scanned_zh),
            )
        };
        frame.render_widget(empty_hint(&title, &hint), inner);
        return;
    }
    let width = inner.width as usize;
    let lines: Vec<Line<'static>> = scan
        .lines
        .iter()
        .enumerate()
        .map(|(row, line)| match line {
            AppLine::Group(category) => {
                let (size, count) = scan.buckets.get(category).cloned().unwrap_or_else(|| {
                    let items: Vec<&CleanupItem> = scan
                        .items
                        .iter()
                        .filter(|item| item.category == *category)
                        .collect();
                    let size = items.iter().map(|item| item.on_disk()).sum::<Bytes>();
                    (size, items.len())
                });
                let open = !scan.collapsed.contains(category);
                let highlight = row == cursor;
                let left = vec![
                    Span::styled(
                        if open { TRIANGLE_OPEN } else { TRIANGLE_CLOSED },
                        dim(highlight, Palette::accent()),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        pad(&app.strings.category(*category), 24),
                        if highlight {
                            Palette::selected()
                        } else {
                            Style::default()
                                .fg(Palette::text())
                                .add_modifier(Modifier::BOLD)
                        },
                    ),
                ];
                joined(
                    left,
                    vec![Span::styled(
                        format!("{} · {}", app.strings.count(count), size.human()),
                        dim(highlight, Palette::accent()),
                    )],
                    width,
                )
                .style(band(row, highlight))
            }
            AppLine::Item(index) => scan
                .items
                .get(*index)
                .map(|item| item_row(item, width, row, row == cursor))
                .unwrap_or_else(|| Line::from("")),
        })
        .collect();
    let list = List::new(lines)
        .style(Palette::list())
        .highlight_symbol("")
        .highlight_style(Style::default());
    let mut state = ListState::default().with_selected(Some(cursor));
    frame.render_stateful_widget(list, inner, &mut state);
}

/// Give the application bundle its own look inside the plan: the row is the
/// app, not a leftover, and it can never be unticked.
fn labelled_row(row: Line<'static>) -> Line<'static> {
    let mut spans = row
        .spans
        .into_iter()
        .map(|span| {
            let mut style = span.style;
            if style.add_modifier.contains(ratatui::style::Modifier::BOLD) {
                return Span {
                    content: span.content,
                    style,
                };
            }
            style.add_modifier |= ratatui::style::Modifier::BOLD;
            Span {
                content: span.content,
                style,
            }
        })
        .collect::<Vec<_>>();
    close_gaps(&mut spans);
    Line::from(spans)
}

fn item_row(item: &CleanupItem, width: usize, row: usize, highlight: bool) -> Line<'static> {
    let size = item.on_disk();
    let (glyph, _) = checkbox(item.selected, item.lock.is_some());
    let label_w = 24usize;
    let head = 2 + 9 + 1 + label_w + 1;
    let locked = item.lock.is_some();
    let note = item
        .lock
        .map(|reason| squid_engine::delete::lock_reason(reason).to_string())
        .or_else(|| item.note.clone());
    let note_w = if width > 118 { 22usize } else { 0 };
    let path_w = width.saturating_sub(head).saturating_sub(note_w + 1);
    let body = if highlight {
        Palette::selected()
    } else if locked {
        Style::default().fg(Palette::muted_fg())
    } else {
        Style::default().fg(Palette::text())
    };
    let mut spans = vec![
        Span::styled(
            format!("{glyph} "),
            if highlight {
                Palette::selected()
            } else if locked {
                Palette::locked()
            } else {
                Style::default().fg(Palette::accent())
            },
        ),
        Span::styled(
            right(&size.human(), 9),
            if highlight {
                body
            } else {
                Palette::size_color(size)
            },
        ),
        Span::raw(" "),
        Span::styled(pad(&truncate(&item.label, label_w, false), label_w), body),
        Span::raw(" "),
        Span::styled(
            truncate(&item.display_path(), path_w, true),
            if highlight {
                body
            } else {
                Style::default().fg(Palette::muted_fg())
            },
        ),
    ];
    if note_w > 0 {
        if let Some(note) = note {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                truncate(&note, note_w, false),
                dim(highlight, Palette::warn()),
            ));
        }
    }
    // Fill the row to the full width. The path column truncates rather than
    // pads, so without this the highlight bar stops short of the right border
    // — a gap that is invisible on a coloured bar and obvious on an inverted
    // one.
    let used: usize = spans
        .iter()
        .map(|span| unicode_width::UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    if used < width {
        spans.push(Span::styled(" ".repeat(width - used), body));
    }
    close_gaps(&mut spans);
    Line::from(spans).style(band(row, highlight))
}

// ----------------------------------------------------------------- uninstall

fn draw_uninstall(frame: &mut Frame, area: Rect, app: &App) {
    if app.uninstall.view == UninstallView::Plan {
        draw_uninstall_plan(frame, area, app);
        return;
    }
    let heading = if app.uninstall.apps.is_empty() {
        format!(
            " {} · {} ",
            app.strings.uninstall(),
            if app.filling(Panel::Uninstall) {
                app.strings.t("reading apps…", "正在读取应用…")
            } else {
                app.strings.t("press s to list apps", "按 s 列出应用")
            }
        )
    } else {
        format!(
            " {} · {} {} ",
            app.strings.uninstall(),
            app.uninstall.visible().len(),
            app.strings.t("apps", "个应用")
        )
    };
    let card = block(&heading);
    let inner = card.inner(area);
    frame.render_widget(card, area);
    let visible = app.uninstall.visible();
    if visible.is_empty() {
        let filtering = !app.uninstall.filter.trim().is_empty();
        let (title, hint) = if filtering {
            (
                format!(
                    "{} “{}”",
                    app.strings.t("No applications match", "没有匹配的应用"),
                    app.uninstall.filter.trim()
                ),
                app.strings
                    .t("press esc to clear the filter", "按 esc 清除筛选"),
            )
        } else if app.filling(Panel::Uninstall) {
            (
                app.strings
                    .t("Reading your applications…", "正在读取应用列表…"),
                app.strings
                    .t("hold on, or press c to cancel", "请稍候，或按 c 取消"),
            )
        } else if app.uninstall.apps.is_empty() {
            (
                app.strings.t("No applications found", "未发现应用"),
                app.strings.t("press s to rescan", "按 s 重新扫描"),
            )
        } else {
            (
                app.strings.t("No applications matched", "没有匹配的应用"),
                app.strings.t("press s to rescan", "按 s 重新扫描"),
            )
        };
        frame.render_widget(empty_hint(&title, &hint), inner);
        return;
    }
    let width = inner.width as usize;
    let size_w = 9usize;
    let version_w = 10usize;
    let name_w = 26usize;
    let source_w = width
        .saturating_sub(2 + size_w + 1 + name_w + 1 + version_w + 1)
        .max(8);
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "{} {} {} {}",
            right(&app.strings.t("size", "大小"), size_w),
            pad(&app.strings.t("application", "应用"), name_w),
            pad(&app.strings.t("version", "版本"), version_w),
            truncate(&app.strings.t("source", "来源"), source_w, false)
        ),
        Palette::muted(),
    ))];
    if !app.uninstall.filter.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(
                "  {}{}",
                app.strings.t("filter", "筛选"),
                app.uninstall.filter
            ),
            Style::default().fg(Palette::warn()).bg(Palette::card_bg()),
        )));
    }
    lines.extend(visible.iter().enumerate().map(|(row, index)| {
        let app_info = &app.uninstall.apps[*index];
        let highlight = row == app.uninstall.cursor;
        let running = app_info.running;
        let left = vec![
            Span::styled(
                if running { "▶ " } else { "  " },
                dim(highlight, Palette::warn()),
            ),
            Span::styled(
                right(&app_info.size.human(), size_w),
                if highlight {
                    Palette::selected()
                } else {
                    Palette::size_color(app_info.size)
                },
            ),
            Span::raw(" "),
            Span::styled(
                pad(&truncate(&app_info.name, name_w, false), name_w),
                if highlight {
                    Palette::selected()
                } else {
                    Style::default().fg(Palette::text())
                },
            ),
            Span::raw(" "),
            Span::styled(
                pad(app_info.version.as_deref().unwrap_or("—"), version_w),
                dim(highlight, Palette::muted_fg()),
            ),
        ];
        let note = if running {
            app.strings.t("running", "运行中")
        } else {
            app_info.source.clone()
        };
        joined(
            left,
            vec![Span::styled(
                truncate(&note, source_w, false),
                dim(
                    highlight,
                    if running {
                        Palette::warn()
                    } else {
                        Palette::muted_fg()
                    },
                ),
            )],
            width,
        )
        .style(band(row, highlight))
    }));
    let list = List::new(lines)
        .style(Palette::list())
        .highlight_symbol("")
        .highlight_style(Style::default());
    let mut state = ListState::default().with_selected(Some(app.uninstall.cursor));
    frame.render_stateful_widget(list, inner, &mut state);
}

fn draw_uninstall_plan(frame: &mut Frame, area: Rect, app: &App) {
    let Some(plan) = &app.uninstall.plan else {
        return;
    };
    let size = app.uninstall.plan_selected_size();
    let heading = format!(
        " {} · {} · {} ",
        app.strings.uninstall(),
        plan.app.name,
        size.human()
    );
    let card = block(&heading);
    let inner = card.inner(area);
    frame.render_widget(card, area);
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(3)]).split(inner);
    let mut info = vec![Line::from(Span::styled(
        format!(
            " {} · {} · {}",
            plan.app.path.display(),
            plan.app.bundle_id.clone().unwrap_or_else(|| "—".into()),
            plan.app.version.clone().unwrap_or_else(|| "—".into())
        ),
        Palette::muted(),
    ))];
    for warning in plan.warnings.iter().take(1) {
        info.push(Line::from(Span::styled(
            format!(" ! {warning}"),
            Style::default().fg(Palette::warn()).bg(Palette::card_bg()),
        )));
    }
    frame.render_widget(Paragraph::new(info).style(Palette::card()), rows[0]);
    let width = rows[1].width as usize;
    let lines: Vec<Line<'static>> = plan
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            // The bundle is a row like any other, so the highlight stays in
            // step with the cursor. It used to be swapped for a heading at
            // position 0, which quietly assumed the bundle is the largest item
            // in the plan: once a container outgrew the app, one leftover hid
            // behind the heading and every row after it was off by one.
            let is_bundle = item.rule_id == "uninstall-bundle";
            let highlight = index == app.uninstall.plan_cursor;
            let row = item_row(item, width, index, highlight);
            let row = if is_bundle { labelled_row(row) } else { row };
            row.style(band(index, highlight))
        })
        .collect();
    let list = List::new(lines)
        .style(Palette::list())
        .highlight_symbol("")
        .highlight_style(Style::default());
    let mut state = ListState::default().with_selected(Some(app.uninstall.plan_cursor));
    frame.render_stateful_widget(list, rows[1], &mut state);
}

// -------------------------------------------------------------------- storage

fn draw_storage(frame: &mut Frame, area: Rect, app: &App) {
    if app.storage.show_files {
        let heading = format!(
            " {} · {} ",
            app.strings.t("Largest files", "最大的文件"),
            app.storage.files.len()
        );
        let card = block(&heading);
        let inner = card.inner(area);
        frame.render_widget(card, area);
        let width = inner.width as usize;
        let lines: Vec<Line<'static>> = app
            .storage
            .files
            .iter()
            .enumerate()
            .map(|(row, file)| {
                let size = file.size();
                joined(
                    vec![
                        Span::styled(
                            right(&size.human(), 9),
                            if row == app.storage.cursor {
                                Palette::selected()
                            } else {
                                Palette::size_color(size)
                            },
                        ),
                        Span::raw(" "),
                    ],
                    vec![Span::styled(
                        truncate(&squid_core::paths::pretty(&file.path), width - 11, true),
                        if row == app.storage.cursor {
                            Palette::selected()
                        } else {
                            Style::default().fg(Palette::muted_fg())
                        },
                    )],
                    width,
                )
                .style(band(row, row == app.storage.cursor))
            })
            .collect();
        let list = List::new(lines)
            .style(Palette::list())
            .highlight_symbol("")
            .highlight_style(Style::default());
        let mut state = ListState::default().with_selected(Some(app.storage.cursor));
        frame.render_stateful_widget(list, inner, &mut state);
        return;
    }

    let Some(node) = app.storage.current() else {
        let card = block(&format!(" {} ", app.strings.storage()));
        let inner = card.inner(area);
        frame.render_widget(card, area);
        frame.render_widget(
            empty_hint(
                &app.strings.t("Mapping your disk…", "正在计算磁盘占用…"),
                &app.strings.t(
                    "this walks the tree once, then you can drill in",
                    "首次遍历后即可查看",
                ),
            ),
            inner,
        );
        return;
    };
    let heading = format!(
        " {} · {} · {} ",
        app.strings.storage(),
        squid_core::paths::pretty(&node.path),
        node.size().human()
    );
    let card = block(&heading);
    let inner = card.inner(area);
    frame.render_widget(card, area);
    let children = node.display_children();
    let width = inner.width as usize;
    let bar_w = 18usize.min(width.saturating_sub(40)).max(6);
    let name_w = 24usize;
    let lines: Vec<Line<'static>> = children
        .iter()
        .enumerate()
        .map(|(row, child)| {
            let folded = row == node.children.len();
            let size = child.size();
            let ratio = child.share_of(node);
            let name = if folded {
                format!(
                    "+ {}",
                    app.strings.t(
                        &format!("{} others", node.folded),
                        &format!("其余 {} 项", node.folded)
                    )
                )
            } else {
                child.name()
            };
            let highlight = row == app.storage.cursor;
            joined(
                vec![
                    Span::styled(
                        if child.is_dir && !folded {
                            "▸ "
                        } else {
                            "  "
                        },
                        dim(highlight, Palette::muted_fg()),
                    ),
                    Span::styled(pad(&truncate(&name, name_w, false), name_w), {
                        if highlight {
                            Palette::selected()
                        } else {
                            Style::default().fg(Palette::text())
                        }
                    }),
                    Span::raw(" "),
                    Span::styled(
                        bar_text(ratio, bar_w),
                        // The bar survives the loss of colour: `bar_text` draws
                        // filled/unfilled as █/░, so inverting the row keeps it
                        // readable where a colour-only fill would disappear.
                        if highlight {
                            Palette::selected()
                        } else {
                            Palette::size_color(size)
                        },
                    ),
                    Span::raw(" "),
                    Span::styled(right(&size.human(), 9), {
                        if highlight {
                            Palette::selected()
                        } else {
                            Palette::size_color(size)
                        }
                    }),
                    Span::raw(" "),
                    Span::styled(
                        format!("{:>3.0}%", ratio * 100.0),
                        dim(highlight, Palette::muted_fg()),
                    ),
                ],
                Vec::new(),
                width,
            )
            .style(band(row, highlight))
        })
        .collect();
    let list = List::new(lines)
        .style(Palette::list())
        .highlight_symbol("")
        .highlight_style(Style::default());
    let mut state = ListState::default().with_selected(Some(app.storage.cursor));
    frame.render_stateful_widget(list, inner, &mut state);
}

// ---------------------------------------------------------------------- stats

fn meter(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    ratio: f64,
    value: &str,
    data: &[f64],
    max: f64,
) {
    let card = plain_block().title_top(format!(" {title} "));
    let inner = card.inner(area);
    frame.render_widget(card, area);
    let parts = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);
    let gauge = Gauge::default()
        .ratio(ratio.clamp(0.0, 1.0))
        .gauge_style(Palette::load(ratio))
        .style(Style::default().fg(Palette::text()).bg(Palette::card_bg()))
        .label(format!(" {value} "))
        .use_unicode(true);
    frame.render_widget(gauge, parts[0]);
    if data.len() > 1 {
        let bars: Vec<u64> = data.iter().map(|value| (*value).max(0.0) as u64).collect();
        let spark = Sparkline::default()
            .data(bars)
            .max((max as u64).max(1))
            .style(
                Style::default()
                    .fg(Palette::accent())
                    .bg(Palette::card_bg()),
            );
        frame.render_widget(spark, parts[1]);
    }
}

fn stat_card(frame: &mut Frame, area: Rect, title: &str, lines: Vec<Line<'static>>) {
    let card = plain_block().title_top(format!(" {title} "));
    let inner = card.inner(area);
    frame.render_widget(card, area);
    frame.render_widget(Paragraph::new(lines).style(Palette::card()), inner);
}

fn draw_stats(frame: &mut Frame, area: Rect, app: &App) {
    let Some(snapshot) = &app.stats.snapshot else {
        let card = block(&format!(" {} ", app.strings.system()));
        let inner = card.inner(area);
        frame.render_widget(card, area);
        frame.render_widget(
            empty_hint(
                &app.strings.t("Sampling…", "采样中…"),
                &app.strings
                    .t("first reading takes a second", "首次采样需要一秒左右"),
            ),
            inner,
        );
        return;
    };
    let trends = app.stats.trends.trimmed(48);
    let cpu: Vec<f64> = trends.cpu.iter().map(|value| *value as f64).collect();
    let memory: Vec<f64> = trends.memory.iter().map(|value| *value as f64).collect();
    let read: Vec<f64> = trends.read;

    let rows = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length(5),
        Constraint::Min(3),
    ])
    .split(area);
    let top = Layout::horizontal([
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
    ])
    .split(rows[0]);
    let cpu_ratio = snapshot.cpu.total as f64 / 100.0;
    meter(
        frame,
        top[0],
        "CPU",
        cpu_ratio,
        &format!(
            "{:.0}% · {}",
            snapshot.cpu.total, snapshot.cpu.logical_cores
        ),
        &cpu,
        100.0,
    );
    let mem_ratio = snapshot.memory.percent_used() as f64 / 100.0;
    meter(
        frame,
        top[1],
        "MEM",
        mem_ratio,
        &format!(
            "{}/{}",
            snapshot.memory.used.human(),
            snapshot.memory.total.human()
        ),
        &memory,
        100.0,
    );
    let swap_total = snapshot.memory.swap_total.as_u64().max(1);
    meter(
        frame,
        top[2],
        "SWAP",
        snapshot.memory.swap_used.as_u64() as f64 / swap_total as f64,
        &format!(
            "{}/{}",
            snapshot.memory.swap_used.human(),
            snapshot.memory.swap_total.human()
        ),
        &[],
        100.0,
    );
    let disk_ratio = snapshot.disk.activity as f64 / 100.0;
    meter(
        frame,
        top[3],
        "DISK",
        disk_ratio,
        &format!(
            "↓{} ↑{}",
            snapshot.disk.read_per_sec.human(),
            snapshot.disk.write_per_sec.human()
        ),
        &read,
        read.iter().cloned().fold(1.0f64, f64::max),
    );

    let middle = Layout::horizontal([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .split(rows[1]);
    let net_title = app.strings.t("Network", "网络");
    stat_card(
        frame,
        middle[0],
        &net_title,
        vec![
            Line::from(Span::styled(
                format!(
                    " ↓ {}/s    ↑ {}/s",
                    snapshot.network.rx_per_sec.human(),
                    snapshot.network.tx_per_sec.human()
                ),
                Style::default().fg(Palette::teal()).bg(Palette::card_bg()),
            )),
            Line::from(Span::styled(
                format!(
                    " {} {} · {}",
                    app.strings.t("since boot", "本次开机"),
                    (snapshot.network.rx_total + snapshot.network.tx_total).human(),
                    snapshot
                        .network
                        .primary
                        .clone()
                        .unwrap_or_else(|| "—".to_string())
                ),
                Palette::muted(),
            )),
        ],
    );
    let disk_title = app.strings.t("Activity", "活动");
    stat_card(
        frame,
        middle[1],
        &disk_title,
        vec![
            Line::from(Span::styled(
                format!(
                    " {} · {} procs",
                    squid_stats::human_uptime(snapshot.uptime_secs),
                    snapshot.processes
                ),
                Style::default().fg(Palette::teal()).bg(Palette::card_bg()),
            )),
            Line::from(Span::styled(
                format!(
                    " load {:.2} {:.2} {:.2}",
                    snapshot.cpu.load[0], snapshot.cpu.load[1], snapshot.cpu.load[2]
                ),
                Palette::muted(),
            )),
        ],
    );
    let battery_title = app.strings.t("Power", "电源");
    let mut power = vec![match &snapshot.battery {
        Some(battery) => Line::from(Span::styled(
            format!(
                " {} · {}",
                battery.label(),
                battery
                    .health_percent
                    .map(|health| format!("{health}% health"))
                    .unwrap_or_else(|| "—".into())
            ),
            Style::default()
                .fg(if battery.percent < 20 && !battery.on_ac {
                    Palette::bad()
                } else {
                    Palette::good()
                })
                .bg(Palette::card_bg()),
        )),
        None => Line::from(Span::styled(" desktop · ac".to_string(), Palette::muted())),
    }];
    // The platform already names the system in its own words: `os_version` is
    // `Windows 10 Pro (build 18363)` there and `macOS 15.1 (24B83)` here, so
    // naming it again here printed the word macOS on a Windows machine. When
    // the row ran short of columns it was also the build number — the half that
    // decides whether Windows Terminal exists — that fell off the end, so the
    // machine model is the part asked to give room.
    let room = plain_block().inner(middle[2]).width as usize;
    let os = &snapshot.host.os_version;
    let spare = room.saturating_sub(os.width() + 4);
    let machine = truncate(&snapshot.host.hardware, spare, false);
    let row = if machine.chars().count() >= 6 {
        format!(" {machine} · {os}")
    } else {
        // No room for both, so the machine model leaves: `LENOVO 20SDA09HCD`
        // never decides anything, the Windows build does.
        format!(" {os}")
    };
    power.push(Line::from(Span::styled(
        truncate(&row, room, false),
        Palette::muted(),
    )));
    stat_card(frame, middle[2], &battery_title, power);

    let proc_title = format!(
        " {} · {} ",
        app.strings.t("Top processes", "进程"),
        snapshot.top_processes.len()
    );
    let card = block(&proc_title);
    let inner = card.inner(rows[2]);
    frame.render_widget(card, rows[2]);
    let width = inner.width as usize;
    let name_w = 24usize;
    let lines: Vec<Line<'static>> = snapshot
        .top_processes
        .iter()
        .enumerate()
        .map(|(row, process)| {
            let highlight = row == app.stats.cursor;
            joined(
                vec![
                    Span::styled(
                        right(&format!("{:.1}%", process.cpu), 6),
                        dim(
                            highlight,
                            if process.cpu > 80.0 {
                                Palette::bad()
                            } else {
                                Palette::accent()
                            },
                        ),
                    ),
                    Span::raw(" "),
                    Span::styled(pad(&truncate(&process.name, name_w, false), name_w), {
                        if highlight {
                            Palette::selected()
                        } else {
                            Style::default().fg(Palette::text())
                        }
                    }),
                    Span::raw(" "),
                    Span::styled(
                        right(&process.memory.human(), 9),
                        dim(highlight, Palette::muted_fg()),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("pid {}", process.pid),
                        dim(highlight, Palette::accent_dim()),
                    ),
                ],
                Vec::new(),
                width,
            )
            .style(band(row, highlight))
        })
        .collect();
    let list = List::new(lines)
        .style(Palette::list())
        .highlight_symbol("")
        .highlight_style(Style::default());
    let mut state = ListState::default().with_selected(Some(app.stats.cursor));
    frame.render_stateful_widget(list, inner, &mut state);
}

// -------------------------------------------------------------------- history

fn draw_history(frame: &mut Frame, area: Rect, app: &App) {
    // "5 条记录 · 共移除 27.88 GB" needs the same care as the rows: the total
    // is a sum of records, and a run that only moved files to the Trash has
    // not taken a single byte off the volume yet.
    let total = app
        .history
        .sessions
        .iter()
        .map(squid_core::ledger::Session::kept)
        .sum::<squid_core::Bytes>();
    let trashed = app
        .history
        .sessions
        .iter()
        .any(|entry| entry.mode == squid_core::DeleteMode::Trash && !entry.restored);
    let mut heading = format!(
        " {} · {} {} · {} {} ",
        app.strings.history(),
        app.history.sessions.len(),
        app.strings.t("sessions", "条记录"),
        app.strings.t("removed", "移除"),
        total.human_fixed()
    );
    if trashed {
        heading.pop();
        heading.push_str(&app.strings.bin_t(
            " · frees once {bin} is emptied ",
            " · 清空{bin}后才真正释放 ",
        ));
    }
    let card = block(&heading);
    let inner = card.inner(area);
    frame.render_widget(card, area);
    if app.history.sessions.is_empty() {
        frame.render_widget(
            empty_hint(
                &app.strings.t("No cleanups yet", "还没有清理记录"),
                &app.strings.t(
                    "every run is written to the ledger, undo included",
                    "每次操作都会记录，可还原",
                ),
            ),
            inner,
        );
        return;
    }
    let width = inner.width as usize;
    let lines: Vec<Line<'static>> = app
        .history
        .sessions
        .iter()
        .enumerate()
        .map(|(row, session)| {
            let highlight = row == app.history.cursor;
            let when = session
                .started_at()
                .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| session.started.clone());
            let freed = session.freed();
            // `1634·24.3 GB` is unreadable: the middle dot sits where a decimal
            // point belongs, so it parses as one broken number. Spell out the
            // count, keep two decimals on the size, and say where the bytes
            // went — Trash means "not off the disk yet".
            //
            // The row then has to *fit*. The card is 84 columns wide on a
            // 112-column terminal, and a cut-off 「93 项失败」 is worse than a
            // bare 「93·」 was: the heading above already says 移除, the command
            // column only ever holds four commands, and the warning goes before
            // the mode badge so that it is the badge, not the failure count,
            // that gives up its room on a narrow terminal.
            let tally = format!(
                "{} · {}",
                app.strings.count(session.removed),
                freed.human_fixed()
            );
            let mut spans = vec![
                Span::styled(when, dim(highlight, Palette::muted_fg())),
                Span::raw("  "),
                Span::styled(
                    pad(&session.command, 16),
                    if highlight {
                        Palette::selected()
                    } else {
                        Style::default().fg(Palette::text())
                    },
                ),
                Span::raw("  "),
                Span::styled(tally, dim(highlight, Palette::good())),
            ];
            // A run that failed 93 of 107 items must not print as a success.
            if session.failed > 0 {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    app.strings.t(
                        &format!("{} failed", session.failed),
                        &format!("{} 失败", session.failed),
                    ),
                    dim(highlight, Palette::warn()),
                ));
            }
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                if session.restored {
                    app.strings.t("restored", "已还原")
                } else {
                    match session.mode {
                        squid_core::DeleteMode::Trash => app.strings.bin().to_string(),
                        squid_core::DeleteMode::Permanent => {
                            app.strings.t("deleted for good", "已清除")
                        }
                    }
                },
                dim(highlight, Palette::accent_dim()),
            ));
            joined(spans, Vec::new(), width).style(band(row, highlight))
        })
        .collect();
    let list = List::new(lines)
        .style(Palette::list())
        .highlight_symbol("")
        .highlight_style(Style::default());
    let mut state = ListState::default().with_selected(Some(app.history.cursor));
    frame.render_stateful_widget(list, inner, &mut state);
}

// -------------------------------------------------------------------- overlays

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

fn draw_confirm(frame: &mut Frame, area: Rect, confirm: &Confirm, app: &App) {
    let height = (confirm.lines.len() as u16 + 5).max(7);
    let popup = centered(area, 74, height);
    frame.render_widget(Clear, popup);
    let card = block(&confirm.title);
    let inner = card.inner(popup);
    frame.render_widget(card, popup);
    let mut lines: Vec<Line<'static>> = confirm
        .lines
        .iter()
        .map(|line| {
            Line::from(Span::styled(
                format!("  {}", truncate(line, 68, true)),
                if line.starts_with("  ") {
                    Palette::muted()
                } else {
                    Style::default().fg(Palette::text()).bg(Palette::card_bg())
                },
            ))
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "  {}",
            app.strings.t(
                "enter / y confirm · n / esc cancel",
                "回车 / y 确认 · n / esc 取消",
            )
        ),
        Style::default()
            .fg(Palette::warn())
            .bg(Palette::card_bg())
            .add_modifier(Modifier::BOLD),
    )));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_help(frame: &mut Frame, area: Rect, app: &App) {
    let s = &app.strings;
    let popup = centered(area, 78, 24);
    frame.render_widget(Clear, popup);
    let title = s.t("squid — keys", "squid — 快捷键");
    let card = block(&title);
    let inner = card.inner(popup);
    frame.render_widget(card, popup);
    let rows: Vec<(String, String)> = vec![
        ("Tab / 1-6".to_string(), s.t("switch panel", "切换面板")),
        (
            "esc".to_string(),
            s.t("back one step · function menu", "返回上一级 · 功能菜单"),
        ),
        (
            "alt/ctrl ↑ ↓".to_string(),
            s.t("switch function from anywhere", "任意位置切换功能"),
        ),
        ("j k ↑ ↓".to_string(), s.t("move", "移动")),
        (
            "space".to_string(),
            s.t("toggle item or group", "勾选项目或分组"),
        ),
        (
            "enter".to_string(),
            s.t("scan / inspect / open", "扫描 / 查看 / 进入"),
        ),
        (
            "s".to_string(),
            s.t("scan the current panel", "扫描当前面板"),
        ),
        ("a".to_string(), s.t("toggle everything", "全选 / 取消全选")),
        (
            "d".to_string(),
            s.t("clean what is checked", "清理已勾选项"),
        ),
        (
            "m".to_string(),
            s.bin_t("{bin} ⇄ permanent", "{bin} ⇄ 永久"),
        ),
        (
            "u".to_string(),
            s.bin_t("restore last {bin} session", "还原上次{bin}会话"),
        ),
        ("f".to_string(), s.t("filter applications", "筛选应用")),
        (
            "h l ← →".to_string(),
            s.t("collapse / drill", "折叠 / 进入"),
        ),
        (
            "c".to_string(),
            s.t("cancel the running job", "取消当前任务"),
        ),
        ("?".to_string(), s.t("this help", "本帮助")),
        ("q".to_string(), s.t("quit", "退出")),
    ];
    let mut lines = rows
        .into_iter()
        .map(|(key, what)| {
            Line::from(vec![
                Span::styled(
                    format!(" {:<14}", key),
                    Style::default()
                        .fg(Palette::accent())
                        .bg(Palette::card_bg()),
                ),
                Span::styled(what, Palette::muted()),
            ])
        })
        .collect::<Vec<_>>();
    // And what this terminal was told it could do. A dashboard that is grey in
    // one window and blue in the next is a report nobody can file without these
    // two answers, so they live where the keys are explained.
    let (locale, source) = squid_core::Locale::explain(app.engine.settings());
    lines.push(Line::from(""));
    for (key, value) in [
        (
            s.t("colour", "色彩"),
            format!(
                "{} · {}",
                crate::theme::color_mode().label(),
                s.t("probed when squid started", "启动时探测")
            ),
        ),
        (
            s.t("language", "语言"),
            format!(
                "{} · {}",
                locale.code(),
                if s.lang == crate::theme::Lang::Zh {
                    source.describe_zh()
                } else {
                    source.describe()
                }
            ),
        ),
    ] {
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {:<14}", key),
                Style::default()
                    .fg(Palette::muted_fg())
                    .bg(Palette::card_bg()),
            ),
            Span::styled(value, Palette::muted()),
        ]));
    }
    frame.render_widget(Paragraph::new(lines).style(Palette::card()), inner);
}

/// A toast is a chip that floats *inside* the panel card, on the card's last
/// inner row.
///
/// It used to be pinned to `height - 3` of the whole frame, and that row is the
/// card's **bottom border**. `Clear` then punched a hole in the border for the
/// four seconds the toast lived, and the untouched tail of the border stayed
/// behind the text — so the line read `功能菜单 — 上下键选择，回车返回
/// --------------╯`: a border that turns into prose halfway across the screen.
/// On the Windows console, where a repaint is what the user reads as "the
/// screen", that hole is indistinguishable from 花屏: the frame looks torn, and
/// every key that answers with a toast (opening the menu, choosing a function)
/// tears it again. Inside the card a toast may still cover one row of the list,
/// which is what a toast is for, but it can no longer eat a border.
fn draw_toast(frame: &mut Frame, card: Rect, toast: &Toast) {
    let color = match toast.kind {
        ToastKind::Good => Palette::good(),
        ToastKind::Warn => Palette::warn(),
        ToastKind::Info => Palette::accent(),
    };
    // One column inside each border: the chip has to be surrounded by the card,
    // not by whatever the card is drawn on top of.
    if card.width < 6 || card.height < 4 {
        return;
    }
    let width = ((toast.text.width() + 4) as u16).min(card.width - 2);
    let popup = Rect {
        x: card.x + 1,
        y: card.bottom() - 2,
        width,
        height: 1,
    };
    if popup.width == 0 || popup.height == 0 {
        return;
    }
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(
                " {} ",
                truncate(&toast.text, width.saturating_sub(2) as usize, false)
            ),
            Style::default()
                .fg(Palette::background())
                .bg(color)
                .add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(color)),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ColorMode;

    /// A highlighted row is one unbroken bar: the column separators and the
    /// filler before the right-hand column must not stay transparent, or an
    /// inverted (colourless) cursor row comes out striped.
    #[test]
    fn a_highlighted_row_has_no_gaps_in_it() {
        crate::theme::force_color_mode(ColorMode::TrueColor);
        let sel = Palette::selected_for(crate::theme::ColorMode::Color256);
        let mut spans = vec![
            Span::styled("▶", sel),
            Span::raw(" "),
            Span::styled("333 MB", sel),
            Span::raw(" "),
            Span::styled("Chatbox", sel),
            Span::raw(" ".repeat(20)),
            Span::styled("Applications", sel),
        ];
        close_gaps(&mut spans);
        for span in &spans {
            assert_eq!(
                span.style, sel,
                "gap span {:?} lost the highlight",
                span.content
            );
        }
    }

    /// A normal row mixes colours on purpose (size in accent, path in muted…).
    /// There is no single row style to extend, so nothing may be flattened.
    #[test]
    fn an_ordinary_row_keeps_its_mix_of_colours() {
        crate::theme::force_color_mode(ColorMode::TrueColor);
        let mut spans = vec![
            Span::styled("333 MB", Palette::card()),
            Span::raw(" "),
            Span::styled("Chatbox", Palette::muted()),
        ];
        close_gaps(&mut spans);
        assert_eq!(spans[1].style, Style::new(), "the separator stays unstyled");
    }
    /// The plan view used to swap item 0 for a heading, which only works while
    /// the `.app` is the biggest thing in the plan. As soon as a container
    /// outgrew the bundle, one leftover disappeared from the list and every
    /// row under it sat one line away from its cursor.
    #[test]
    fn the_plan_view_shows_the_bundle_and_every_leftover() {
        crate::theme::force_color_mode(ColorMode::TrueColor);
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        use squid_core::{AppInfo, ItemKind, UninstallPlan};

        let bundle = || {
            let mut item = CleanupItem::new(
                "/Applications/Demo.app".into(),
                ItemKind::Path,
                Category::Leftovers,
                "DemoBundle",
            )
            .with_rule("uninstall-bundle");
            item.size = Bytes(1 << 20);
            item.selected = true;
            item
        };
        let leftover = |path: &str, label: &str, size: u64| {
            let mut item = CleanupItem::new(
                path.into(),
                ItemKind::Path,
                Category::Leftovers,
                label.to_string(),
            )
            .with_rule("uninstall-leftovers");
            item.size = Bytes(size);
            item.selected = true;
            item
        };

        let mut app = App::new().expect("app");
        app.panel = Panel::Uninstall;
        app.uninstall.view = UninstallView::Plan;
        app.uninstall.plan = Some(UninstallPlan {
            app: AppInfo {
                name: "Demo".into(),
                bundle_id: Some("com.example.Demo".into()),
                path: "/Applications/Demo.app".into(),
                version: Some("1.0".into()),
                size: Bytes(1 << 20),
                source: "Applications".into(),
                running: false,
                has_sibling_install: false,
                publisher: None,
                installed_ms: None,
            },
            // Sorted the way `build_plan` sorts: biggest first, so the bundle
            // is *not* the first item here.
            items: vec![
                leftover(
                    "/Users/x/Library/Containers/com.example.Demo",
                    "GiantContainer",
                    9 << 30,
                ),
                bundle(),
                leftover(
                    "/Users/x/Library/Preferences/com.example.Demo.plist",
                    "PrefsFile",
                    4096,
                ),
            ],
            warnings: Vec::new(),
        });
        app.uninstall.rebuild_lines();
        assert_eq!(app.uninstall.lines.len(), 3, "one line per item");

        // Reads the screen back the way a user does: by finding a row and
        // asking what it looks like.
        fn render(app: &App) -> Terminal<TestBackend> {
            let mut term = Terminal::new(TestBackend::new(140, 34)).expect("terminal");
            term.draw(|frame| draw(frame, app)).expect("draw");
            term
        }
        /// The row as text, plus the column every one of its characters was
        /// painted in.
        ///
        /// Reading the screen back by hand is a two-step job: `find` answers
        /// with a *byte* offset, and a byte offset is only a column while the
        /// screen is ASCII. The moment the sidebar answers in Chinese each
        /// glyph costs three bytes for two columns, every match drifts, and
        /// the test reads its styles off the wrong cell — which is how this
        /// test started to fail once the interface spoke Chinese. The column
        /// table is what turns the offset back into a cell.
        fn row_text(term: &Terminal<TestBackend>, row: u16) -> (String, Vec<u16>) {
            let buf = term.backend().buffer();
            let mut text = String::new();
            let mut columns = Vec::new();
            for col in 0..buf.area().width {
                for ch in buf[(col, row)].symbol().chars() {
                    text.push(ch);
                    columns.push(col);
                }
            }
            (text, columns)
        }
        /// The column a needle starts in, given the row it was found on.
        fn column_of(text: &str, columns: &[u16], offset: usize) -> u16 {
            columns[text[..offset].chars().count()]
        }
        fn locate(term: &Terminal<TestBackend>, needle: &str) -> (u16, u16) {
            for row in 0..term.backend().buffer().area().height {
                let (text, columns) = row_text(term, row);
                if let Some(offset) = text.find(needle) {
                    return (row, column_of(&text, &columns, offset));
                }
            }
            panic!("{needle} is not on the screen");
        }
        /// A row the cursor is on is painted with one style from the checkbox
        /// to the label; a row it is not on keeps its mix of colours.
        fn under_cursor(term: &Terminal<TestBackend>, needle: &str) -> bool {
            let buf = term.backend().buffer();
            let (row, col) = locate(term, needle);
            let (text, columns) = row_text(term, row);
            let offset = text
                .find(['\u{25cf}', '\u{25cb}'])
                .expect("the row has a checkbox");
            let checked = column_of(&text, &columns, offset);
            buf[(checked, row)].style() == buf[(col, row)].style()
        }
        fn bold(term: &Terminal<TestBackend>, needle: &str) -> bool {
            let buf = term.backend().buffer();
            let (row, col) = locate(term, needle);
            buf[(col, row)]
                .style()
                .add_modifier
                .contains(Modifier::BOLD)
        }

        // Nothing under the cursor, so the only emphasis on screen is the one
        // the bundle earns from its rule.
        app.uninstall.plan_cursor = app.uninstall.lines.len();
        let term = render(&app);
        let screen: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        for label in ["GiantContainer", "DemoBundle", "PrefsFile"] {
            assert!(
                screen.contains(label),
                "{label} missing from the plan view:\n{screen}"
            );
        }
        // Every item owns a row. The heading that used to sit at position 0
        // spent one of them, so a plan of three drew two checkboxes.
        let marked = screen
            .chars()
            .filter(|c| *c == '\u{25cf}' || *c == '\u{25cb}')
            .count();
        assert_eq!(marked, 3, "one checkbox row per item:\n{screen}");
        for heading in ["the application", "\u{5e94}\u{7528}\u{672c}\u{4f53}"] {
            assert!(
                !screen.contains(heading),
                "a heading swallowed a plan row ({heading})"
            );
        }

        // The bundle is recognised by its rule, never by its position, so the
        // emphasised row is the `.app` even though a bigger container sorts
        // above it.
        assert!(
            bold(&term, "DemoBundle"),
            "the `.app` row is the emphasised one"
        );
        for other in ["GiantContainer", "PrefsFile"] {
            assert!(
                !bold(&term, other),
                "{other} is not the application, it should stay plain"
            );
        }
        let order = ["GiantContainer", "DemoBundle", "PrefsFile"]
            .iter()
            .map(|needle| locate(&term, needle).0)
            .collect::<Vec<_>>();
        assert!(
            order.windows(2).all(|pair| pair[0] < pair[1]),
            "the rows keep the order of plan.items: {order:?}"
        );

        // And the highlight follows `plan_cursor` item for item: row 0 is the
        // first item, not a heading.
        app.uninstall.plan_cursor = 0;
        let term = render(&app);
        assert!(
            under_cursor(&term, "GiantContainer"),
            "the cursor paints the row of the item it points at"
        );
        for other in ["DemoBundle", "PrefsFile"] {
            assert!(
                !under_cursor(&term, other),
                "{other} sits on another row than the cursor"
            );
        }
    }

    /// A row of the data pane is a strip, not a scattering of coloured words:
    /// one band of grey from the left border to the right, alternating with a
    /// second band, and a third under the cursor. That is what makes a list
    /// readable when the terminal has 256 colours and the accent has been
    /// rounded to the nearest entry of a 6x6x6 cube — and it is what stops a
    /// row from breaking apart into the gaps between its columns.
    #[test]
    fn the_rows_of_the_data_pane_are_banded() {
        crate::theme::force_color_mode(ColorMode::TrueColor);
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        use squid_core::ItemKind;

        let junk = |name: &str| {
            let mut item = CleanupItem::new(
                format!("/Users/x/Library/Caches/{name}").into(),
                ItemKind::File,
                Category::SystemJunk,
                name,
            );
            item.size = Bytes(1 << 20);
            item.selected = true;
            item
        };
        let mut app = App::new().expect("app");
        app.menu = false;
        app.clean.items = vec![junk("alpha"), junk("bravo"), junk("charlie"), junk("delta")];
        app.clean.rebuild_lines();
        // Lines are grouped, so the rows are 1..=4 and the cursor sits on the
        // third item.
        app.clean.cursor = 3;

        let mut term = Terminal::new(TestBackend::new(120, 30)).expect("terminal");
        term.draw(|frame| draw(frame, &app)).expect("draw");
        let buffer = term.backend().buffer();
        let row_of = |needle: &str| -> u16 {
            for row in 0..buffer.area().height {
                let line: String = (0..buffer.area().width)
                    .map(|col| buffer[(col, row)].symbol())
                    .collect();
                if line.contains(needle) {
                    return row;
                }
            }
            panic!("{needle} is not on the screen");
        };
        let bg_at = |row: u16, col: u16| buffer[(col, row)].style().bg;
        // One column inside the right border: the part of a row that carries no
        // text of its own, which is precisely where an unstyled gap shows up.
        let edge = buffer.area().width - 3;
        let (alpha, bravo, charlie, delta) = (
            bg_at(row_of("alpha"), edge),
            bg_at(row_of("bravo"), edge),
            bg_at(row_of("charlie"), edge),
            bg_at(row_of("delta"), edge),
        );
        assert_ne!(alpha, bravo, "next door is the other band");
        assert_eq!(bravo, delta, "two rows down is the same band again");
        assert_ne!(charlie, alpha, "the cursor is a third colour");
        assert_ne!(charlie, bravo, "not one of the two bands");
        assert!(
            buffer[(edge, row_of("charlie"))]
                .style()
                .add_modifier
                .contains(Modifier::BOLD),
            "and bold, so it survives a terminal with no palette at all"
        );
        // The card the list lives in: the last two vertical borders on that
        // row are its left and right edge, the ones before them belong to the
        // sidebar.
        let row = row_of("bravo");
        let bars: Vec<u16> = (0..buffer.area().width)
            .filter(|col| buffer[(*col, row)].symbol() == "\u{2502}")
            .collect();
        assert!(bars.len() >= 2, "the card has no borders: {bars:?}");
        let plain: Vec<Option<ratatui::style::Color>> = (bars[bars.len() - 2] + 1
            ..bars[bars.len() - 1])
            .map(|col| bg_at(row, col))
            .collect();
        assert!(
            plain.windows(2).all(|pair| pair[0] == pair[1]),
            "the band runs the whole width of the card, but came out as {:?}",
            plain
                .iter()
                .enumerate()
                .filter(|(_, bg)| **bg != plain[0])
                .map(|(col, _)| col)
                .collect::<Vec<_>>()
        );
    }

    /// Keyboard focus on the sidebar must read in a terminal without colour,
    /// so the focused row carries brackets as well as a reversed background.
    #[test]
    fn the_menu_marks_the_sidebar_row() {
        crate::theme::force_color_mode(ColorMode::TrueColor);
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        fn screen(app: &App) -> String {
            let mut term = Terminal::new(TestBackend::new(120, 34)).expect("terminal");
            term.draw(|frame| draw(frame, app)).expect("draw");
            term.backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect()
        }
        let mut app = App::new().expect("app");
        app.menu = false;
        let plain = screen(&app);
        assert!(
            !plain.contains("[1]"),
            "no brackets while the list has the arrows:\n{plain}"
        );

        app.menu = true;
        assert!(
            screen(&app).contains("[1]"),
            "the focused row shows brackets"
        );
        app.panel = Panel::Purge;
        assert!(
            screen(&app).contains("[3]"),
            "and the brackets follow the selection"
        );
    }

    /// A thin-provisioned Docker image once made the header promise 1.1 TB on
    /// a 256 GB SSD, because every total took `max(logical, allocated)`. The
    /// list and its header both owe the user the number the disk can confirm.
    #[test]
    fn a_sparse_item_shows_the_space_it_really_occupies() {
        crate::theme::force_color_mode(ColorMode::TrueColor);
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        use squid_core::ItemKind;

        let mut image = CleanupItem::new(
            "/Users/x/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw".into(),
            ItemKind::File,
            Category::SystemJunk,
            "Docker image",
        );
        image.size = Bytes(1_024_000_000_000); // 1 TiB of address space
        image.allocated = Bytes(12_000_000_000); // 12 GB of blocks
        image.selected = true;

        let mut app = App::new().expect("app");
        app.menu = false;
        app.clean.items = vec![image];
        app.clean.rebuild_lines();

        assert_eq!(app.clean.total_size(), Bytes(12_000_000_000));
        assert_eq!(app.clean.selected_size(), Bytes(12_000_000_000));

        let mut term = Terminal::new(TestBackend::new(140, 34)).expect("terminal");
        term.draw(|frame| draw(frame, &app)).expect("draw");
        let screen: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            screen.contains("12.0 GB"),
            "the row and the header should both read 12.0 GB:\n{screen}"
        );
        assert!(
            !screen.contains("TB"),
            "a sparse file must not inflate the total past the physical disk:\n{screen}"
        );
    }
    /// The hint bar has to fit the window it is drawn in, and it has to say
    /// each key once. Both were broken: the twelve fixed hints ran past the
    /// right edge — `q 退出` and the panel's own key were what fell off — and
    /// the history panel printed `u 还原` twice.
    #[test]
    fn every_hint_bar_fits_and_says_each_key_once() {
        crate::theme::force_color_mode(ColorMode::TrueColor);
        for lang in [crate::theme::Lang::Zh, crate::theme::Lang::En] {
            for panel in Panel::ALL {
                let mut app = App::new().expect("app");
                app.strings = crate::theme::Strings::for_lang(lang);
                app.panel = panel;
                for menu in [true, false] {
                    app.menu = menu;
                    let text = hint_text(&app);
                    assert!(
                        text.width() <= 104,
                        "{lang:?} {panel:?} menu={menu} needs {} columns: {text}",
                        text.width()
                    );
                    assert!(text.contains('?'), "help is always reachable: {text}");
                    assert!(
                        text.ends_with("q 退出") || text.ends_with("q quit"),
                        "{text}"
                    );
                    let undo = text.matches("还原").count() + text.matches("undo").count();
                    assert!(undo <= 1, "{panel:?} advertises undo {undo} times: {text}");
                }
            }
        }
    }

    #[test]
    fn the_power_card_names_the_system_the_platform_reported() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        use squid_stats::{HostInfo, SystemSnapshot};

        crate::theme::force_color_mode(ColorMode::TrueColor);
        fn screen(app: &App, columns: u16) -> String {
            let mut term = Terminal::new(TestBackend::new(columns, 34)).expect("terminal");
            term.draw(|frame| draw(frame, app)).expect("draw");
            term.backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect()
        }
        let windows = SystemSnapshot {
            host: HostInfo {
                os_name: "Windows".into(),
                os_version: "Windows 10 Pro (build 18363)".into(),
                hardware: "LENOVO 20SDA09HCD".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut app = App::new().expect("app");
        app.panel = Panel::Stats;
        app.stats.snapshot = Some(windows.clone());

        let wide = screen(&app, 140);
        assert!(
            wide.contains("Windows 10 Pro (build 18363)"),
            "the card says which Windows this is:\n{wide}"
        );
        assert!(
            !wide.contains("macOS"),
            "a Windows machine must not be called macOS:\n{wide}"
        );

        // Narrow enough that neither half fits in full. The system is still
        // the half on screen, and the machine model is the half that left.
        let narrow = screen(&app, 72);
        assert!(
            narrow.contains("Windows 10"),
            "a squeezed Power card still says what this machine runs:\n{narrow}"
        );
        assert!(
            !narrow.contains("LENOVO"),
            "the machine model is what gives up its columns:\n{narrow}"
        );
    }
    /// A toast is four seconds of prose; a card's border is permanent. The chip
    /// used to be pinned to `height - 3` of the *frame*, and that row is the
    /// panel card's bottom border — so every toast cut a sentence into the
    /// border and left the rest of the border standing behind the text. That is
    /// the exact picture the Windows user reported as 花屏: a frame that reads
    /// as torn, torn again by every key that answers with a toast (opening the
    /// menu, choosing a function). A border has to survive a toast unchanged.
    #[test]
    fn a_toast_cannot_cut_a_hole_in_the_cards_border() {
        crate::theme::force_color_mode(ColorMode::TrueColor);
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        use unicode_width::UnicodeWidthChar;

        fn rows(app: &App) -> Vec<String> {
            let mut term = Terminal::new(TestBackend::new(100, 30)).expect("terminal");
            term.draw(|frame| draw(frame, app)).expect("draw");
            let buffer = term.backend().buffer();
            (0..buffer.area.height)
                .map(|y| {
                    let mut line = String::new();
                    let mut x = 0;
                    while x < buffer.area.width {
                        let symbol = buffer[(x, y)].symbol();
                        line.push_str(symbol);
                        // 宽字符占两格,第二格被 ratatui 重置成一个空格:跳过去,
                        // 否则读出来的行里每个汉字中间都夹一个空格,搜也搜不到。
                        x += if symbol.chars().next().is_some_and(|c| c.width() == Some(2)) {
                            2
                        } else {
                            1
                        };
                    }
                    line
                })
                .collect()
        }

        let mut app = App::new().expect("app");
        app.toast = None;
        let quiet = rows(&app);
        let bottoms: Vec<usize> = quiet
            .iter()
            .enumerate()
            .filter(|(_, line)| line.starts_with('╰'))
            .map(|(y, _)| y)
            .collect();
        assert!(
            !bottoms.is_empty(),
            "the cards should have bottom borders:\n{}",
            quiet.join("\n")
        );

        app.toast(
            "功能菜单 — 上下键选择，回车返回",
            crate::app::ToastKind::Info,
        );
        let loud = rows(&app);
        for y in &bottoms {
            assert_eq!(
                &quiet[*y],
                &loud[*y],
                "a toast rewrote the border on row {}:\n{}",
                y + 1,
                loud.join("\n")
            );
        }
        let screen: String = loud.concat();
        assert!(
            screen.contains("上下键选择"),
            "the toast still has to be readable:\n{}",
            loud.join("\n")
        );
        // And it has to be readable *inside* the card: the left border of the row
        // carrying the chip must still be a border, not the start of the toast.
        let carrier = loud
            .iter()
            .find(|line| line.contains("上下键选择"))
            .expect("the toast is on screen");
        assert!(
            carrier.starts_with('│'),
            "the toast should float inside the card, not over its edge: {carrier:?}"
        );
    }
}
