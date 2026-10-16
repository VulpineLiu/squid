//! Carrying a finished frame to the console it is about to be drawn on.
//!
//! What a console charges for the characters an interface is drawn out of, and
//! what to draw when it charges more than the layout paid, is
//! [`squid_platform::currency`]: the command line buys the same screen at the
//! same counter, so the price list and the exchange live beside the console
//! rather than beside a renderer. What cannot travel is the walk over a finished
//! ratatui [`Buffer`] — a question about somebody else's data structure — so
//! that, and only that, is what this crate adds.
//!
//! Two things live here because of it. [`witnesses`] is the list of glyphs this
//! interface is drawn out of, which only this crate can know: the source of the
//! files every screen is built from, plus the symbols ratatui's widgets draw on
//! its behalf. [`Carry`] is the walk that spends the answers on a frame.
//!
//! The invariant worth defending is that those two are one list. A glyph drawn
//! but never priced is invisible on every terminal that charges one cell for
//! everything, and overflows a row on the one that does not, so the mistake is
//! undetectable from a screenshot and lands in front of exactly one person. The
//! last test here is what holds the two halves together.
//!

use ratatui::buffer::Buffer;
use ratatui::symbols;
use squid_platform::currency::{self, Exchange};

/// Ask this console what it charges for everything this interface draws, and
/// work out what to draw instead.
///
/// [`squid_platform::console::prices`] measures the answers by writing each
/// character and reading back what it cost, so the Chinese console that cannot
/// afford a block of shading draws its gauges in `#` while keeping the rounded
/// cards it can afford, and Windows Terminal, a console on UTF-8 and a Mac keep
/// everything they were drawn with.
///
/// Measuring spends characters on whatever the console is showing, which is why
/// this is called once the alternate screen is up and not at startup.
pub fn measure() -> Exchange {
    Exchange::new(squid_platform::console::prices(&witnesses()))
}

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
    currency::witnesses(
        FACES
            .iter()
            .map(|face| face.to_string())
            .chain(drawn_by_widgets()),
    )
}

/// Spend a console's answers on a frame that was drawn without asking for them.
///
/// A trait rather than a method on [`Exchange`], because the exchange is shared
/// with the command line and a command line has no frame to walk. What travels
/// is the arithmetic; this is the part that knows what a cell is.
pub trait Carry {
    /// Rewrite a rendered frame so that every cell costs what the layout paid.
    ///
    /// Called once the widgets are done and before the frame reaches the
    /// terminal, which is the last moment at which an over-long row can still be
    /// made to fit. On a console that charges one cell for everything there is
    /// nothing to do, and this returns without touching the frame.
    fn carry(&self, buffer: &mut Buffer);
}

impl Carry for Exchange {
    fn carry(&self, buffer: &mut Buffer) {
        if !self.overcharged() {
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
                if let Some(instead) = self.change(first) {
                    cell.set_char(instead);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{witnesses, Carry, FACES};
    use ratatui::{
        buffer::Buffer,
        layout::Rect,
        style::Style,
        widgets::{Block, BorderType, Borders, Widget},
    };
    use squid_platform::console::{Prices, SymbolCells};
    use squid_platform::currency::{self, Exchange};
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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

    /// A console that charges two cells for everything the layout measured as
    /// one, which is the worst answer the probe can bring back.
    fn cramped() -> Exchange {
        Exchange::new(Prices::flat(SymbolCells::Double))
    }

    /// A console that charges one cell for everything, so nothing here has a
    /// reason to touch the frame.
    fn fairly_priced() -> Exchange {
        Exchange::new(Prices::flat(SymbolCells::Single))
    }

    /// The console this exchange was written for and still got wrong once: the
    /// rules cost one cell, the shading costs two. Whether a console really
    /// charges this way is measured, never assumed — this is the *shape* of the
    /// unlucky answer, so the walk is tested against the two sets having
    /// different prices, which is the one thing a single price cannot express.
    fn rules_affordable_gauges_not() -> Exchange {
        let mut asked = Vec::new();
        for ch in "░▒▓█".chars() {
            asked.push((ch, SymbolCells::Double));
        }
        for ch in "│─╭·…↑✓".chars() {
            asked.push((ch, SymbolCells::Single));
        }
        Exchange::new(Prices::measured(asked, SymbolCells::Single))
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
        let exchange = rules_affordable_gauges_not();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 14, 1));
        buffer.set_string(0, 0, "│ C: ░░░░░ ███", Style::default());
        exchange.carry(&mut buffer);
        // The rule is exactly as it was drawn, because this console was measured
        // charging one cell for it; only the blocks changed hands.
        assert_eq!(read(&buffer, 0), "│ C: ..... ###");
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
                let Some(instead) = currency::cheaper(ch) else {
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
    fn the_probe_asks_the_price_of_everything_that_ends_up_on_the_screen() {
        // The invariant that would have caught it: the list the console is asked
        // about and the list the interface draws are one list. A glyph added to
        // a card, or drawn by a widget nobody thought about, has to be priced —
        // and no test that renders a frame can tell, because on a terminal that
        // charges one cell for everything an unpriced glyph is invisible.
        let asked = witnesses();
        for face in FACES {
            for ch in face.chars() {
                if currency::disputed(ch) {
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
