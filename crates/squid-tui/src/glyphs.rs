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
//! The exchange is decided character by character, which is the part that took
//! the longest to get right. Rules, shading and punctuation are priced one at a
//! time by the code page and font a console was left on, nothing binds the three
//! to the same answer, and a layout that asks about one and pays for all three is
//! one character deep — the first Chinese window this was chased through had room
//! for the rounded cards and none for the `░` inside them. So the price of every
//! glyph this interface draws is asked of the console
//! — [`squid_platform::console::prices`] takes the whole list — and the
//! rounded card a Chinese console can afford stays rounded while only the
//! blocks that it cannot are paid for in ASCII. A character that was never
//! asked about is charged the pessimistic price, because a glyph that escaped
//! the list is a glyph nobody checked.
//!
//! Chinese text is left alone, and so are the letters of most alphabets —
//! neither is ambiguous, so both cost the console exactly what they cost the
//! layout. That is deliberate. A person's paths and process names are not what
//! an alignment bug was caused by, and this module rewrites only the characters
//! it can prove the console is overcharging.

use ratatui::buffer::Buffer;
use ratatui::symbols;
use squid_platform::console::{Prices, SymbolCells};
use unicode_width::UnicodeWidthChar;

/// What the console charges for the characters this interface is drawn from.
pub struct Fit {
    /// The price list, kept so a beat can say what the glass charged and so a
    /// test can ask what one glyph cost.
    prices: Prices,
    /// What to draw in place of each character, indexed by code point, with
    /// `None` for the ones this console can afford. A frame is tens of
    /// thousands of cells and a beat is a quarter of a second, so the answer is
    /// looked up rather than worked out — and the table holds the replacement
    /// rather than a bit saying *this one is wrong*, because the character is
    /// what a cell needs and asking the price of a glyph is the only way to
    /// know that a rounded corner may stay rounded while a block of shading
    /// may not.
    exchange: Vec<Option<char>>,
}

impl Fit {
    /// Ask the console what it charges for every character this interface draws.
    ///
    /// [`squid_platform::console::prices`] measures the answers by writing each
    /// character and reading back what it cost, so the Chinese Windows console
    /// that cannot afford a block of shading draws its gauges in `#` while
    /// keeping the rounded cards it can afford, and Windows Terminal, a console
    /// on UTF-8 and a Mac keep everything they were drawn with.
    ///
    /// Measuring spends characters on whatever the console is showing, which is
    /// why this is called once the alternate screen is up and not at startup.
    pub fn measure() -> Fit {
        Fit::priced(squid_platform::console::prices(&witnesses()))
    }

    /// A console whose answers are already known: the ones a person named in the
    /// environment, and the ones a test wants to pretend.
    fn priced(prices: Prices) -> Fit {
        let exchange = (0..=PRICED_UPTO)
            .map(|code| {
                char::from_u32(code)
                    // Two questions have to be answered before a cell is
                    // rewritten: is this a character this module owns at all
                    // (not Chinese, not somebody's path), and does *this*
                    // console actually charge two cells for it. The first is a
                    // rule, the second is a measurement, and a glyph that fails
                    // either one keeps what it was drawn with.
                    .filter(|&ch| prices.price(ch) == SymbolCells::Double)
                    .and_then(cheaper)
            })
            .collect();
        squid_platform::console::note(&format!("symbols {}", prices.text()));
        Fit { prices, exchange }
    }

    /// Rewrite a rendered frame so that every cell costs what the layout paid.
    ///
    /// Called once the widgets are done and before the frame reaches the
    /// terminal, which is the last moment at which an over-long row can still
    /// be made to fit. On a console that charges one cell for everything there
    /// is nothing to do, and this returns without touching the frame.
    pub fn carry(&self, buffer: &mut Buffer) {
        if !self.prices.overcharged() {
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
                // A character above the table is one no console reprices: that
                // is where Chinese lives, and both sides already agree that it
                // costs two cells, so it has nothing here to buy back.
                if let Some(instead) = self.exchange.get(first as usize).copied().flatten() {
                    cell.set_char(instead);
                }
            }
        }
    }
}

/// The highest code point the price of is worth a slot in the table, which is
/// one past the last range a Far-Eastern console was ever seen repricing.
const PRICED_UPTO: u32 = 0x2BFF;

/// The three files every screen of this interface is drawn out of.
const FACES: &[&str] = &[
    include_str!("app.rs"),
    include_str!("theme.rs"),
    include_str!("views.rs"),
];

/// The characters the widgets draw on our behalf, taken from the library that
/// draws them rather than from what this crate happens to mention: the rules of
/// a card, the fill and the empty of a gauge, the bars of a sparkline. A widget
/// is not a literal anybody can grep for, so it is asked for by name.
fn drawn_by_widgets() -> Vec<String> {
    use symbols::border::Set;
    let mut drawn: Vec<String> = Vec::new();

    let shading = [
        symbols::shade::EMPTY,
        symbols::shade::LIGHT,
        symbols::shade::MEDIUM,
        symbols::shade::DARK,
        symbols::shade::FULL,
        symbols::DOT,
    ];
    drawn.extend(shading.map(str::to_string));

    for level in [
        symbols::half_block::UPPER,
        symbols::half_block::LOWER,
        symbols::half_block::FULL,
    ] {
        drawn.push(level.to_string());
    }

    // A gauge is filled with the left blocks and a sparkline drawn with the
    // bottom ones, both in eight levels, and either could be the character this
    // console charges two cells for.
    let bars = [
        symbols::bar::FULL,
        symbols::bar::SEVEN_EIGHTHS,
        symbols::bar::THREE_QUARTERS,
        symbols::bar::FIVE_EIGHTHS,
        symbols::bar::HALF,
        symbols::bar::THREE_EIGHTHS,
        symbols::bar::ONE_QUARTER,
        symbols::bar::ONE_EIGHTH,
        symbols::block::FULL,
        symbols::block::SEVEN_EIGHTHS,
        symbols::block::THREE_QUARTERS,
        symbols::block::FIVE_EIGHTHS,
        symbols::block::HALF,
        symbols::block::THREE_EIGHTHS,
        symbols::block::ONE_QUARTER,
        symbols::block::ONE_EIGHTH,
    ];
    drawn.extend(bars.map(str::to_string));

    for Set {
        top_left,
        top_right,
        bottom_left,
        bottom_right,
        vertical_left,
        vertical_right,
        horizontal_top,
        horizontal_bottom,
    } in [
        symbols::border::PLAIN,
        symbols::border::ROUNDED,
        symbols::border::DOUBLE,
        symbols::border::THICK,
    ] {
        drawn.extend(
            [
                top_left,
                top_right,
                bottom_left,
                bottom_right,
                vertical_left,
                vertical_right,
                horizontal_top,
                horizontal_bottom,
            ]
            .map(str::to_string),
        );
    }

    drawn
}

/// Every character whose price has to be asked rather than looked up: the ones
/// this interface draws itself, and the ones its widgets draw for it.
///
/// This is the list that used to be one character long. It is derived from the
/// program — the source of the screens, plus the widgets — so that a glyph put
/// into a card is a glyph whose price gets asked without anybody remembering to
/// update a table, which is exactly the thing that went wrong and looked
/// perfectly fine on every terminal but one.
pub fn witnesses() -> Vec<char> {
    let mut asked: Vec<char> = Vec::new();
    let widgets = drawn_by_widgets();
    let drawn = FACES.iter().map(|face| face.to_string());
    for face in drawn.chain(widgets) {
        for ch in face.chars() {
            if in_dispute(ch) && !asked.contains(&ch) {
                asked.push(ch);
            }
        }
    }
    asked
}

/// The character to draw instead, or `None` when this one is fairly priced.
fn cheaper(ch: char) -> Option<char> {
    in_dispute(ch).then(|| one_cell(ch))
}

/// True when this is a character a console is free to charge two cells for.
///
/// In dispute is not the same as overcharged: whether *this* console does it is
/// measured, per character, by [`Fit::measure`]. What this answers is which
/// characters are worth asking about, and which ones the frame is allowed to
/// rewrite once the console has said.
///
/// The first of the two tests is the honest one: `width` is what the layout paid
/// and `width_cjk` is what the Ambiguous class says a Far-Eastern terminal
/// charges, so where they agree there is nothing for this module to repair,
/// whatever else the console may be getting wrong.
///
/// The second test exists because that class is a description of what terminals
/// have been seen to do rather than a rule, and a Chinese console charges it to
/// characters the class leaves out. `░` is not Ambiguous at all, and it is what
/// this interface fills its gauges with — and it is the character the single probe
/// never asked about, which is how a frame whose rules were measured fair still
/// overflowed. Anything this list declines to ask about is a character the console
/// prices without being asked, so it errs towards the questions.
/// Being too generous here buys a plainer character on the console that cannot
/// afford the drawn one; being too strict buys the shifted frame this module
/// exists to prevent. So the blocks a Far-Eastern writing system was laid out
/// over are asked as well as the class, and the letters of other alphabets keep
/// their one cell.
fn in_dispute(ch: char) -> bool {
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
    use super::{cheaper, in_dispute, one_cell, witnesses, Fit, FACES};
    use ratatui::{
        buffer::Buffer,
        layout::Rect,
        style::Style,
        widgets::{Block, BorderType, Borders, Widget},
    };
    use squid_platform::console::{Prices, SymbolCells};
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

    /// A console that charges two cells for everything the layout measured as
    /// one, which is the worst answer the probe can bring back.
    fn cramped() -> Fit {
        Fit::priced(Prices::flat(SymbolCells::Double))
    }

    /// A console that charges one cell for everything, so nothing here has a
    /// reason to touch the frame.
    fn fairly_priced() -> Fit {
        Fit::priced(Prices::flat(SymbolCells::Single))
    }

    /// The console this module was written for and still got wrong: the rules
    /// cost one cell, the shading costs two. Whether a console really charges this
    /// way is measured, never assumed — this is the *shape* of the unlucky answer,
    /// so the exchange is tested against the two sets having different prices,
    /// which is the one thing a single price can never express.
    fn rules_affordable_gauges_not() -> Fit {
        let mut asked = Vec::new();
        for ch in "░▒▓█".chars() {
            asked.push((ch, SymbolCells::Double));
        }
        for ch in "│─╭·…↑✓".chars() {
            asked.push((ch, SymbolCells::Single));
        }
        Fit::priced(Prices::measured(asked, SymbolCells::Single))
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
            assert!(in_dispute(ch), "{ch} should be in dispute");
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
        fairly_priced().carry(&mut buffer);
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
                    "`{ch}` is drawn here, so the console must not guess what it meant"
                );
            }
        }
        assert!(
            drawn.chars().count() > 15,
            "the scan found {drawn:?}, which is not this interface"
        );
    }

    #[test]
    fn a_console_that_reprices_the_gauges_keeps_the_rules_it_priced_fairly() {
        // The frame this module existed for two rounds and still got wrong. One
        // probe asked the price of a box-drawing rule, was answered one cell, and
        // that became the price of every glyph on the screen — so the rounded card
        // stayed rounded, the gauge stayed made of `█`, and every row of the
        // Storage and System panels ran further than the console had cells for.
        // The overflow arrived at the beginning of the row below, which is where
        // the menu is: a gauge overwriting the second entry of a list is what the
        // screen looked like, and it is why a price is now asked of every
        // character instead of of whichever one was easiest to write.
        let fit = rules_affordable_gauges_not();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 14, 1));
        buffer.set_string(0, 0, "│ C: ░░░░░ ███", Style::default());
        fit.carry(&mut buffer);
        // The rule is exactly as it was drawn, because this console was measured
        // charging one cell for it; only the blocks changed hands.
        assert_eq!(read(&buffer, 0), "│ C: ..... ###");
    }

    #[test]
    fn the_probe_asks_the_price_of_everything_that_ends_up_on_the_screen() {
        // The invariant that would have caught it: the list the console is asked
        // about and the list the interface draws are one list. A glyph added to
        // a card, or drawn by a widget nobody thought about, has to be priced —
        // and no test that renders a frame can tell, because on a terminal that
        // charges one cell for everything an unpriced glyph is invisible.
        let asked = witnesses();
        for face in FACES {
            for ch in face.chars() {
                if in_dispute(ch) {
                    assert!(asked.contains(&ch), "`{ch}` is drawn but never priced");
                }
            }
        }
        // The shading and the bars come out of ratatui, not out of this crate.
        for ch in ['░', '▒', '▓', '█', '▄', '▉', '▏', '│', '─', '╭', '╰'] {
            assert!(asked.contains(&ch), "the widgets draw `{ch}`");
        }
        // Chinese is priced the same by both sides and is not worth a question.
        assert!(!asked.contains(&'回'), "a person's text is not in dispute");
        assert!(asked.len() > 20, "{asked:?} is not this interface");
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
