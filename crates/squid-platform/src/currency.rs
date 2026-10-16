//! What a console charges for the characters an interface is drawn out of, and
//! what to draw when it charges more than the layout paid.
//!
//! A layout measures a row in columns and charges every character the number of
//! columns it measured: one for a letter, one for a box-drawing bar, two for a
//! Chinese one. Unicode keeps a class of characters whose price was never
//! settled — *East Asian Ambiguous* — and leaves each terminal free to charge
//! either one cell or two for them. Where the two disagree is exactly where the
//! chrome of an interface lives: the rules `─│╭`, the shading `█▒░`, the arrows
//! `↑↓`, the ticks `✓`, the middot `·`, the ellipsis `…`.
//!
//! Two programs pay for that disagreement in two different ways, and both of
//! them are this module's business. An interface that is a pile of cards is built
//! out of nothing but ambiguous characters, so on a console that charges two
//! every row of it is several cells longer than the screen, and what goes past
//! the right edge arrives at the start of the row below: several copies of one
//! frame at several offsets, which looks like a renderer that cannot count. It is
//! counting correctly, in a currency the console will not accept. A command-line
//! program does not measure its rows at all — it repeats `█` twenty-eight times
//! for a gauge and lets the console do the arithmetic, and prints a table whose
//! rows are each a screen and a half long. Same price, same receipt.
//!
//! The price is asked, not looked up, and it is asked of every character
//! separately: [`crate::console::prices`] writes each one and reads back what it
//! cost, and [`Exchange`] turns those answers into what to draw instead. Nothing
//! promises that rules, shading and punctuation land on the same side of a code
//! page and a font, so one answer for the lot is one character deep — the console
//! this was chased through had room for the rounded cards and none for the `░`
//! inside them. On such a glass the card stays rounded and the gauge is paid for
//! in `.`, `%`, `#`. A character nobody asked about is charged the pessimistic
//! price, because a glyph that escaped the list is a glyph nobody checked.
//!
//! Chinese text is left alone, and so are the letters of most alphabets —
//! neither is ambiguous, so both cost the console exactly what they cost the
//! layout. That is deliberate. A person's paths and process names are not what an
//! alignment bug was caused by, and this module rewrites only the characters it
//! can prove the console is overcharging.
//!
//! This lives beside the console rather than inside the interface because the
//! command line and the interface buy the same screen at the same counter. The
//! half that cannot travel is the walking of a finished ratatui frame — a
//! different library's question, so it stays in [`squid_tui::glyphs`].
//!
//! [`squid_tui::glyphs`]: ../../squid_tui/glyphs/index.html

use crate::console::{note, Prices, SymbolCells};
use unicode_width::UnicodeWidthChar;

/// What to draw in place of every character this console cannot afford.
///
/// Held as a table rather than as one verdict, for the reason in the module
/// documentation: the price is set per character, so the answer is per character
/// too. A frame is tens of thousands of cells and a beat is a quarter of a
/// second, so the answer is looked up rather than worked out — and the table
/// holds the replacement rather than a bit saying *this one is wrong*, because
/// the character is what the caller needs.
pub struct Exchange {
    /// The answers this table was built from, kept so a log can say what the
    /// glass charged and a test can ask what one glyph cost.
    prices: Prices,
    /// Indexed by code point, with `None` for every character this console can
    /// afford. Empty when nothing is overcharged, which is what makes
    /// [`Exchange::afford`] cheap on the terminals where there is nothing to do.
    table: Vec<Option<char>>,
}

impl Exchange {
    /// Build the table from one console's answers, and write those answers into
    /// the trace.
    ///
    /// The `note` belongs here rather than at each call site because it is the
    /// only record of why a gauge came out as `#`: a person comparing the plain
    /// frame with the drawn one needs to be told what the glass charged for `░`,
    /// and both the interface and the command line owe them that line.
    pub fn new(prices: Prices) -> Exchange {
        let table = if prices.overcharged() {
            (0..=PRICED_UPTO)
                .map(|code| {
                    char::from_u32(code)
                        // Two questions have to be answered before a character is
                        // rewritten: is this one owned at all (not Chinese, not
                        // somebody's path), and does *this* console actually
                        // charge two cells for it. The first is a rule, the
                        // second is a measurement, and a glyph that fails either
                        // one keeps what it was drawn with.
                        .filter(|&ch| prices.price(ch) == SymbolCells::Double)
                        .and_then(cheaper)
                })
                .collect()
        } else {
            Vec::new()
        };
        note(&format!("symbols {}", prices.text()));
        Exchange { prices, table }
    }

    /// The answers this table was built from.
    pub fn prices(&self) -> &Prices {
        &self.prices
    }

    /// Whether this console charges for anything a layout measured as one cell,
    /// which is the one question worth asking before rewriting anything.
    pub fn overcharged(&self) -> bool {
        !self.table.is_empty()
    }

    /// The character to draw in place of this one, or `None` when this console
    /// can afford the character it was given.
    pub fn change(&self, ch: char) -> Option<char> {
        self.table.get(ch as usize).copied().flatten()
    }

    /// One run of text at the price this console charges.
    ///
    /// This is the door a command-line program prints through: it has no frame to
    /// walk, only strings, and a gauge built by repeating `█` twenty-eight times
    /// is twenty-eight cells wide as far as the console is concerned — the only
    /// moment a row like that can still be made to fit is before the bytes leave.
    pub fn afford(&self, text: &str) -> String {
        if !self.overcharged() {
            return text.to_string();
        }
        text.chars()
            .map(|ch| self.change(ch).unwrap_or(ch))
            .collect()
    }
}

/// Every character in these pieces of chrome whose price has to be asked rather
/// than looked up.
///
/// The list is derived from the program rather than maintained by hand, which is
/// the whole lesson of the single-probe version: a glyph put into a screen in one
/// file and never mentioned in another produced a frame that was correct on every
/// terminal but one, and no test could see it. Hand each piece of chrome this
/// program draws out of — its own source, a widget library's symbols — and the
/// questions to ask come out of the answer.
pub fn witnesses(pieces: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<char> {
    let mut asked: Vec<char> = Vec::new();
    for piece in pieces {
        for ch in piece.as_ref().chars() {
            if disputed(ch) && !asked.contains(&ch) {
                asked.push(ch);
            }
        }
    }
    asked
}

/// The highest code point the price of is worth a slot in the table, which is
/// one past the last range a Far-Eastern console was ever seen repricing.
pub const PRICED_UPTO: u32 = 0x2BFF;

/// True when this is a character a console is free to charge two cells for.
///
/// In dispute is not the same as overcharged: whether *this* console does it is
/// measured, per character, by [`Exchange`]. What this answers is which
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
pub fn disputed(ch: char) -> bool {
    ch.width() == Some(1) && (ch.width_cjk() == Some(2) || laid_out_by_far_east(ch))
}

/// The character to draw instead, or `None` when this one is fairly priced.
pub fn cheaper(ch: char) -> Option<char> {
    disputed(ch).then(|| one_cell(ch))
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
    use super::{cheaper, disputed, one_cell, witnesses, Exchange, PRICED_UPTO};
    use crate::console::{Prices, SymbolCells};
    use unicode_width::UnicodeWidthChar;

    /// The worst answer a probe can bring back: two cells for everything the
    /// layout measured as one.
    fn cramped() -> Exchange {
        Exchange::new(Prices::flat(SymbolCells::Double))
    }

    /// The answer every terminal not on a Far-Eastern code page gives.
    fn fairly_priced() -> Exchange {
        Exchange::new(Prices::flat(SymbolCells::Single))
    }

    /// The shape of the unlucky answer: the rules cost one cell, the shading
    /// costs two. Whether a console really charges this way is measured, never
    /// assumed — this is the case a single price can never express, so it is
    /// built rather than waited for.
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
    fn the_chrome_an_interface_is_drawn_from_is_what_a_console_overcharges() {
        // The class is not a curiosity. A card's border, the separator between
        // two thoughts, the ellipsis that ends a truncated name and the gauge
        // that says how full a disk is are all in it.
        for ch in [
            "─", "│", "╭", "╮", "╯", "╰", "·", "…", "—", "↑", "↓", "✓", "×", "█", "▒", "▾", "①",
            "●", "“", "░", "►", "⊘", "⇄",
        ] {
            let ch = ch.chars().next().expect("one character");
            assert!(disputed(ch), "{ch} should be in dispute");
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
        for word in ["回", "收", "站", "，", "：", "é", "ü", "Ω", "ф", "十"] {
            let ch = word.chars().next().expect("one character");
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
    fn a_line_pays_for_itself_in_characters_the_console_accepts() {
        // The command line's whole need in one assertion: what comes out is all
        // one column wide, so a gauge of twenty-eight of them is twenty-eight
        // columns and the row under it still holds its own text.
        let paid = cramped().afford("  ✓ [░▒█] · ↑↓ …");
        assert_eq!(paid, "  v [.%#] / ^v .");
        for ch in paid.chars() {
            assert_eq!(ch.width_cjk(), Some(1), "{paid} still charges two");
        }
    }

    #[test]
    fn text_untouched_is_the_answer_on_a_console_that_charges_one() {
        let line = "  ✓ [░▒█] · 回收站";
        assert!(!fairly_priced().overcharged());
        assert_eq!(fairly_priced().afford(line), line);
    }

    #[test]
    fn only_the_overcharged_half_of_a_line_changes_hands() {
        // The console this module was written for and still got wrong once: the
        // rule was asked, found fair, and that one answer became the price of
        // every glyph on the screen.
        let exchange = rules_affordable_gauges_not();
        assert_eq!(exchange.afford("│ C: ░░ █"), "│ C: .. #");
    }

    #[test]
    fn witnesses_are_the_chrome_they_are_handed() {
        let asked = witnesses(["│░ clean", "  ✓ 回"]);
        assert!(asked.contains(&'│') && asked.contains(&'░') && asked.contains(&'✓'));
        assert!(!asked.contains(&'c'), "ascii is never in dispute");
        assert!(!asked.contains(&'回'), "a person's text is not in dispute");
        assert_eq!(witnesses(["█"]), witnesses(["██"]), "asked once");
    }

    #[test]
    fn the_table_reaches_every_character_a_console_is_seen_to_reprice() {
        // A character above the table is one nothing reprices, so the last slot
        // has to be past the last block a Far-Eastern writing system was laid out
        // over — otherwise the exchange answers `?` where it should have had a plan.
        assert!(char::from_u32(PRICED_UPTO).is_some());
        for ch in ["─", "█", "■", "→", "①"] {
            let ch = ch.chars().next().expect("one character");
            assert!(
                (ch as u32) <= PRICED_UPTO,
                "{ch} is off the end of the table"
            );
        }
    }
}
