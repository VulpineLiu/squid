//! Byte counts with the two formatting styles the UI needs.
//!
//! macOS reports disk space in decimal units (1 KB = 1000 B) so that is the
//! default; [`Bytes::human_binary`] is there for memory readouts.

use serde::{Deserialize, Serialize};
use std::fmt;

pub const KB: u64 = 1_000;
pub const MB: u64 = 1_000 * KB;
pub const GB: u64 = 1_000 * MB;
pub const TB: u64 = 1_000 * GB;

/// `1234567` -> `1,234,567`. The `{:,}` specifier is not available here, so
/// grouping is done by hand.
pub fn format_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (idx, ch) in digits.chars().enumerate() {
        if idx > 0 && (digits.len() - idx).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Bytes(pub u64);

impl Bytes {
    pub const ZERO: Bytes = Bytes(0);

    pub const fn new(value: u64) -> Self {
        Bytes(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }

    pub fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// (value, unit) pair, decimal.
    pub fn parts(self) -> (f64, &'static str) {
        const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
        let mut value = self.0 as f64;
        let mut unit = 0;
        while value >= 1000.0 && unit < UNITS.len() - 1 {
            value /= 1000.0;
            unit += 1;
        }
        (value, UNITS[unit])
    }

    /// Short form used in dense tables: `4.2 GB`, `318 MB`, `12 KB`.
    pub fn human(self) -> String {
        let (value, unit) = self.parts();
        if unit == "B" {
            format!("{} {}", value as u64, unit)
        } else if value >= 100.0 {
            format!("{:.0} {}", value, unit)
        } else {
            format!("{:.1} {}", value, unit)
        }
    }

    /// Ledger form: always two decimals — `24.31 GB`, `1.37 GB`, `920 KB`.
    ///
    /// A list row can get away with one decimal because the number is an
    /// estimate that is about to change. A history entry is a *record*, and a
    /// record that reads `1.4 GB` for a run that freed 1,365,856,256 bytes
    /// looks rounded, not measured.
    pub fn human_fixed(self) -> String {
        let (value, unit) = self.parts();
        if unit == "B" {
            format!("{} {}", value as u64, unit)
        } else {
            format!("{value:.2} {unit}")
        }
    }

    /// Precise form for detail panes: `4.28 GB (4,593,815,296 bytes)`.
    pub fn human_long(self) -> String {
        let (value, unit) = self.parts();
        if unit == "B" {
            format!("{} bytes", self.0)
        } else {
            format!("{:.2} {} ({} bytes)", value, unit, format_thousands(self.0))
        }
    }

    /// Binary form for memory: `16.0 GiB`.
    pub fn human_binary(self) -> String {
        const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
        let mut value = self.0 as f64;
        let mut unit = 0;
        while value >= 1024.0 && unit < UNITS.len() - 1 {
            value /= 1024.0;
            unit += 1;
        }
        if unit == 0 {
            format!("{} {}", value as u64, UNITS[unit])
        } else {
            format!("{:.1} {}", value, UNITS[unit])
        }
    }

    /// Whole percentage of `total`, saturating at 100.
    pub fn percent_of(self, total: Bytes) -> u8 {
        if total.is_zero() {
            return 0;
        }
        ((self.0 as u128 * 100) / total.0 as u128).min(100) as u8
    }
}

impl std::ops::Add for Bytes {
    type Output = Bytes;
    fn add(self, rhs: Self) -> Self::Output {
        Bytes(self.0.saturating_add(rhs.0))
    }
}

impl<'a> std::iter::Sum<&'a Bytes> for Bytes {
    fn sum<I: Iterator<Item = &'a Bytes>>(iter: I) -> Self {
        iter.fold(Bytes::ZERO, |acc, item| acc + *item)
    }
}

impl std::iter::Sum<Bytes> for Bytes {
    fn sum<I: Iterator<Item = Bytes>>(iter: I) -> Self {
        iter.fold(Bytes::ZERO, |acc, item| acc + item)
    }
}

impl std::ops::AddAssign for Bytes {
    fn add_assign(&mut self, rhs: Self) {
        self.0 = self.0.saturating_add(rhs.0);
    }
}

impl fmt::Display for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.human())
    }
}

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bytes({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_decimal_units() {
        assert_eq!(Bytes(0).human(), "0 B");
        assert_eq!(Bytes(999).human(), "999 B");
        assert_eq!(Bytes(1_500).human(), "1.5 KB");
        assert_eq!(Bytes(4_593_815_296).human(), "4.6 GB");
        assert_eq!(Bytes(120_000_000_000).human(), "120 GB");
    }

    #[test]
    fn binary_units_for_memory() {
        assert_eq!(Bytes(16 * 1024 * 1024 * 1024).human_binary(), "16.0 GiB");
    }

    #[test]
    fn percentage() {
        assert_eq!(Bytes(50 * GB).percent_of(Bytes(200 * GB)), 25);
        assert_eq!(Bytes(1).percent_of(Bytes::ZERO), 0);
    }

    /// The ledger is a record, not an estimate: `1.4 GB` for a run that freed
    /// 1,365,856,256 bytes reads as rounded, and the user read the whole number
    /// as a display bug.
    #[test]
    fn the_ledger_form_keeps_two_decimals() {
        assert_eq!(Bytes(1_365_856_256).human_fixed(), "1.37 GB");
        assert_eq!(Bytes(24_311_898_112).human_fixed(), "24.31 GB");
        assert_eq!(Bytes(753_655_808).human_fixed(), "753.66 MB");
        assert_eq!(Bytes(920).human_fixed(), "920 B");
        assert_eq!(Bytes(0).human_fixed(), "0 B");
    }
}
