//! Buying the screen at the price the console actually charges for it.
//!
//! The layout measures a row in columns, and every character is charged the
//! number of columns it measures: one for a letter, one for a box-drawing bar,
//! two for a Chinese one. Unicode keeps a class of characters whose price was
//! never settled — *East Asian Ambiguous* — and it leaves a terminal free to
//! charge either one cell or two for them. ratatui charges one. A console left
//! on a Far-Eastern code page charges two, and that class is exactly where the
//! chrome of an interface lives: the rules `─│╭`, the shading `█▒░`, the arrows
//! `↑↓`, the ticks `✓`, the middot `·`, the ellipsis `…`.
//!
//! A screen of this interface is a pile of cards, so it is built out of nothing
//! but ambiguous characters, and on such a console every row of it is several
//! cells longer than the screen. What goes past the right edge arrives at the
//! start of the row below, which shifts that one, and the frame ends up as
//! several copies of itself at several offsets: a border that stops short, half
//! a rounded corner standing alone where the next frame did not reach, a footer
//! printed twice. It looks like a renderer that cannot count. The renderer is
//! counting correctly, in a currency the console will not accept.
//!
//! So the finished frame passes through here on its way to the terminal, and
//! every character the console overcharges is drawn as an ASCII character that
//! costs the one cell the layout paid for it. The rules become `|-+`, the
//! gauges `#`, the arrows `<^>v`. Nothing about the layout moves, because the
//! exchange is one column for one column: what changes is that the rows fit.
//!
//! Chinese text is left alone, and so are the letters of most alphabets —
//! neither is ambiguous, so both cost the console exactly what they cost the
//! layout. That is deliberate. A person's paths and process names are not what
//! an alignment bug was caused by, and this module rewrites only the characters
//! it can prove the console is overcharging.

use ratatui::buffer::Buffer;
use squid_platform::console::SymbolCells;
use unicode_width::UnicodeWidthChar;

/// The console's price list, taken once, from the console itself.
pub struct Fit {
    /// True when a character the layout bought for one cell costs two here.
    overcharging: bool,
}

impl Fit {
    /// Ask what the screen being drawn on charges for a one-column character.
    ///
    /// [`squid_platform::console::symbol_cells`] measures the answer by writing
    /// one character and reading back what it cost, so the Chinese Windows
    /// console that cannot afford a box-drawing bar gets the ASCII chrome it
    /// can, while Windows Terminal, a console on UTF-8 and a Mac keep the
    /// rounded cards they have always paid for.
    pub fn measure() -> Fit {
        let cells = squid_platform::console::symbol_cells();
        squid_platform::console::note(&format!("symbols={cells:?}"));
        Fit {
            overcharging: cells == SymbolCells::Double,
        }
    }

    /// Rewrite a rendered frame so that every cell costs what the layout paid.
    ///
    /// Called once the widgets are done and before the frame reaches the
    /// terminal, which is the last moment at which an over-long row can still
    /// be made to fit. On a console that charges one cell there is nothing to
    /// do, and this returns without touching the frame.
    pub fn carry(&self, buffer: &mut Buffer) {
        if !self.overcharging {
            return;
        }
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                let Some(cell) = buffer.cell_mut((x, y)) else {
                    continue;
                };
                // A cell holding no character is the half of a wide one that a
                // previous row already paid for, and has nothing to price.
                let Some(first) = cell.symbol().chars().next() else {
                    continue;
                };
                if let Some(instead) = cheaper(first) {
                    cell.set_char(instead);
                }
            }
        }
    }
}

/// The character to draw instead, or `None` when this one is fairly priced.
fn cheaper(ch: char) -> Option<char> {
    overcharged(ch).then(|| one_cell(ch))
}

/// True when the console charges two cells for what the layout measured as one.
///
/// The first of the two tests is the honest one: `width` is what the layout paid
/// and `width_cjk` is what the Ambiguous class says a Far-Eastern terminal
/// charges, so where they agree there is nothing for this module to repair,
/// whatever else the console may be getting wrong.
///
/// The second test exists because that class is a description of what terminals
/// have been seen to do rather than a rule, and a console on code page 936 has
/// been seen doing it to characters the class leaves out. `░` is not Ambiguous at
/// all, and it is what this interface fills its gauges with: a row of ten of them
/// is twenty cells on that glass. Being too generous here buys a plainer
/// character on the console that cannot afford the drawn one; being too strict
/// buys the shifted frame this module exists to prevent. So the blocks a
/// Far-Eastern writing system was laid out over are asked as well as the class,
/// and the letters of other alphabets keep their one cell.
fn overcharged(ch: char) -> bool {
    ch.width() == Some(1) && (ch.width_cjk() == Some(2) || laid_out_by_far_east(ch))
}

/// The blocks a Far-Eastern code page spells in two bytes, and a console reading
/// those bytes therefore reserves two cells for.
///
/// Kept to the blocks an interface is drawn out of — punctuation, rules, shading,
/// arrows, shapes, enclosed digits — rather than applied to everything above
/// ASCII, because what is left out is what a person brought with them: a path
/// with an `é`, a process named in Greek. Rewriting those would be editing
/// somebody's data to cover for the console.
///
/// Only consulted once the console has been measured charging two, so on every
/// terminal that charges one this answers nothing at all.
fn laid_out_by_far_east(ch: char) -> bool {
    let code = ch as u32;
    matches!(
        code,
        // Punctuation and signs: `· × ° ± ¼ «`.
        0x00A1..=0x00BF
        // Dashes, quotes, bullets, the ellipsis.
        | 0x2010..=0x205E
        // The currency sign a price or a path may carry.
        | 0x20A0..=0x20BF
        // Signs like `№`, arrows of every shape, and the maths an interface
        // leans on for `≈ ≥ ⋅`, and the keys a Mac legends with a shape.
        | 0x2100..=0x218F | 0x2190..=0x21FF | 0x2200..=0x23FF
        // Digits and letters with a ring drawn round them.
        | 0x2460..=0x24FF
        // Box drawing, block shading, geometric shapes, and the symbols and
        // dingbats a status line is made of.
        | 0x2500..=0x2BFF
    )
}

/// What an overcharged character is worth in characters the console accepts.
///
/// Every answer is a single ASCII column, because ASCII is the one thing a
/// Far-Eastern console never reprices, and every one is chosen to say the same
/// thing at the same width: a rule stays a rule, a gauge stays a gauge, a tick
/// still points the way it did. A shape with no honest equivalent becomes `?`,
/// which is what the console would have shown in its place anyway.
fn one_cell(ch: char) -> char {
    match ch {
        // Rules: the chrome of every card on the screen.
        '─' | '━' | '═' | '┄' | '┅' | '┈' | '┉' => '-',
        '│' | '┃' | '║' | '┆' | '┇' | '┊' | '┋' => '|',
        // Corners and junctions both mean "these lines meet here", which ASCII
        // spells with one character.
        '┌' | '┍' | '┎' | '┏' | '╭' | '╔' | '┐' | '┑' | '┒' | '┓' | '╮' | '╗' | '└' | '┕' | '┖'
        | '┗' | '╰' | '╚' | '┘' | '┙' | '┚' | '┛' | '╯' | '╝' | '├' | '┝' | '┞' | '┟' | '┠'
        | '┡' | '┢' | '┣' | '╠' | '┤' | '┥' | '┦' | '┧' | '┨' | '┩' | '┪' | '┫' | '╣' | '┬'
        | '┭' | '┮' | '┯' | '┰' | '┱' | '┲' | '┳' | '╦' | '┴' | '┵' | '┶' | '┷' | '┸' | '┹'
        | '┺' | '┻' | '╩' | '┼' | '┽' | '┾' | '┿' | '╀' | '╁' | '╂' | '╃' | '╄' | '╅' | '╆'
        | '╇' | '╈' | '╉' | '╊' | '╬' | '╪' | '╫' => '+',
        // Gauges and shading, densest to faintest.
        '█' | '▓' | '▄' | '▅' | '▆' | '▇' | '▉' | '■' | '▪' | '◆' | '◼' => {
            '#'
        }
        '▒' | '▂' | '▃' => '%',
        '░' | '▁' | '□' | '▫' | '◻' => '.',
        // Status. A tick still has to read as a tick, so it keeps the shape
        // ASCII art has always used for one; a cross keeps the one it had.
        '✓' | '✔' | '☑' => 'v',
        '✗' | '✘' | '☒' | '☓' | '×' => 'x',
        // A box nobody is allowed to toggle says "not this one" with the same
        // letter a cross uses.
        '⊘' => 'x',
        '●' | '◉' => '@',
        '○' | '◎' | '◦' => 'o',
        // Direction.
        '▶' | '▸' | '►' | '▹' | '→' | '⇒' | '➤' | '»' => '>',
        '◀' | '◂' | '◄' | '◃' | '←' | '⇐' | '«' => '<',
        // Both directions at once has no spelling in one letter. An equals sign
        // is the closest one that still says the two sides belong together.
        '⇄' | '⇅' | '↔' | '↕' => '=',
        '▲' | '△' | '▴' | '⌃' | '↑' | '⇧' => '^',
        '▼' | '▽' | '▾' | '↓' | '⇩' => 'v',
        // An arrow that turns the corner into something: a right angle is what
        // a backslash looks like.
        '↳' => '\\',
        // The separator this interface puts between two thoughts, and the bullets.
        '·' | '‧' | '⋅' | '÷' => '/',
        '•' | '‣' | '★' | '☆' => '*',
        // A word the row ran out of room for, and a line the row ran out of.
        '…' | '⋯' | '⋮' => '.',
        '—' | '–' | '―' | '−' => '-',
        '“' | '”' | '„' | '‟' => '"',
        '‘' | '’' | '‚' | '‛' => '\'',
        '±' => '+',
        '≈' => '~',
        '≥' => '>',
        '≤' => '<',
        // The keys a Mac legends with a shape no other keyboard carries.
        '⌘' => '#',
        '⌥' => 'A',
        '⌫' => '<',
        '⌦' => '>',
        '⇥' => '>',
        // A digit with a ring round it: the ring is the part being charged for.
        '①'..='⑨' => char::from_u32(ch as u32 - '①' as u32 + '0' as u32).unwrap_or('?'),
        _ => '?',
    }
}

#[cfg(test)]
mod tests {
    use super::{cheaper, one_cell, overcharged, Fit};
    use ratatui::{
        buffer::Buffer,
        layout::Rect,
        style::Style,
        widgets::{Block, BorderType, Borders, Widget},
    };
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

    /// A narrow frame, spelled out so the tests read as rows of a screen.
    fn cramped() -> Fit {
        Fit { overcharging: true }
    }

    /// What a row of the buffer reads as. The cell behind a wide character is
    /// reset to a blank and no eye ever sees it, so it is stepped over here the
    /// same way the terminal steps over it.
    fn read(buffer: &Buffer, y: u16) -> String {
        let mut row = String::new();
        let mut hidden = 0usize;
        for x in 0..buffer.area.width {
            let Some(cell) = buffer.cell((x, y)) else {
                continue;
            };
            if hidden > 0 {
                hidden -= 1;
                continue;
            }
            let symbol = cell.symbol();
            hidden = symbol.width().saturating_sub(1);
            row.push_str(symbol);
        }
        row
    }

    #[test]
    fn the_chrome_of_this_interface_is_exactly_what_the_console_overcharges() {
        // The class is not a curiosity. A card's border, the separator between
        // two thoughts, the ellipsis that ends a truncated name and the gauge
        // that says how full a disk is are all in it.
        for ch in [
            '─', '│', '╭', '╮', '╯', '╰', '·', '…', '—', '↑', '↓', '✓', '×', '█', '▒', '▾', '①',
            '●', '“', '░', '►', '⊘', '⇄',
        ] {
            assert!(overcharged(ch), "{ch} should be in dispute");
            assert!(
                one_cell(ch).is_ascii(),
                "{ch} is in dispute and must be exchanged, not argued about"
            );
        }
    }

    #[test]
    fn chinese_and_the_latin_alphabet_keep_their_own_price() {
        // Two cells to both sides, or one to both: no dispute, nothing to
        // rewrite. This is what keeps a person's paths out of the repair.
        for ch in ['回', '收', '站', '，', '：', 'é', 'ü', 'Ω', 'ф', '十'] {
            assert!(
                cheaper(ch).is_none(),
                "{ch} costs the same to both sides and must survive"
            );
        }
    }

    #[test]
    fn nothing_is_exchanged_for_a_character_with_the_same_problem() {
        // Sweeps the whole plane: whatever this module answers with must be one
        // cell by both tables and spellable by a console that reprices
        // everything else. A gap here is a row that still overflows.
        for code in 0x20u32..=0xFFFF {
            let Some(ch) = char::from_u32(code) else {
                continue;
            };
            let Some(instead) = cheaper(ch) else {
                continue;
            };
            assert!(instead.is_ascii(), "{ch} was paid for with {instead}");
            assert_eq!(
                instead.width(),
                Some(1),
                "{ch} bought {instead}, which is not one column"
            );
            assert_eq!(
                instead.width_cjk(),
                Some(1),
                "{ch} bought {instead}, which is overcharged too"
            );
        }
    }

    #[test]
    fn an_overcharged_frame_comes_back_as_a_row_that_fits() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 9, 1));
        buffer.set_string(0, 0, "╭─╮ · ░▒█", Style::default());
        cramped().carry(&mut buffer);
        assert_eq!(read(&buffer, 0), "+-+ / .%#");
    }

    #[test]
    fn a_frame_is_left_alone_on_a_console_that_charges_one() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 9, 1));
        buffer.set_string(0, 0, "╭─╮ · ░▒█", Style::default());
        Fit {
            overcharging: false,
        }
        .carry(&mut buffer);
        assert_eq!(read(&buffer, 0), "╭─╮ · ░▒█");
    }

    #[test]
    fn chinese_keeps_both_of_its_cells_and_its_neighbour_keeps_one() {
        // 回收站 spends six cells and the middot after it one, so the row is
        // seven wide before and after. The repair may not move text it is not
        // touching, which is the whole difference between this and a redraw.
        let mut buffer = Buffer::empty(Rect::new(0, 0, 7, 1));
        buffer.set_string(0, 0, "回收站·", Style::default());
        cramped().carry(&mut buffer);
        assert_eq!(read(&buffer, 0), "回收站/");
    }

    /// The source of every screen this crate draws, so the set of glyphs is the
    /// program's and not a list someone keeps up to date by hand.
    const FACES: &[&str] = &[
        include_str!("app.rs"),
        include_str!("theme.rs"),
        include_str!("views.rs"),
    ];

    #[test]
    fn every_glyph_this_interface_draws_has_a_price() {
        // What this test stands in for is the mistake that spent a week: a glyph
        // put into a card in one file and never priced in the table in another.
        // Nothing else in the suite notices, because on every terminal that
        // charges one cell the row is exactly as wide as the layout says, and
        // the frame only falls apart in front of a person on a Chinese console.
        let mut drawn = String::new();
        for face in FACES {
            for ch in face.chars() {
                if ch as u32 <= 0x7F || ch.width() != Some(1) || drawn.contains(ch) {
                    continue;
                }
                drawn.push(ch);
                let Some(instead) = cheaper(ch) else {
                    continue;
                };
                assert!(
                    instead.is_ascii() && instead != '?',
                    "`{ch}` is drawn by this interface, so the console must not be                      left guessing what it was meant to be"
                );
            }
        }
        assert!(
            drawn.chars().count() > 15,
            "the scan found {drawn:?}, which is not this interface"
        );
    }

    #[test]
    fn a_card_is_still_a_card_after_the_exchange() {
        // ratatui's own border widget rather than a hand-written row, so this
        // fails if the library ever draws a card with a character that has been
        // left unpriced.
        let mut buffer = Buffer::empty(Rect::new(0, 0, 5, 3));
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .render(buffer.area, &mut buffer);
        cramped().carry(&mut buffer);
        assert_eq!(read(&buffer, 0), "+---+");
        assert_eq!(read(&buffer, 1), "|   |");
        assert_eq!(read(&buffer, 2), "+---+");
    }
}
