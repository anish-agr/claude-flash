//! Human-readable durations (`90s`, `15m`, `1h30m`) for configuration and the CLI.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A duration in milliseconds that reads and writes as text such as `"3m"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span(pub u64);

impl Span {
    pub const ZERO: Span = Span(0);

    pub const fn ms(self) -> u64 {
        self.0
    }

    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurationError(String);

impl fmt::Display for DurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a duration: {:?} (use forms like 90s, 15m, 1h30m or 0)", self.0)
    }
}

impl std::error::Error for DurationError {}

/// Parses `0`, `250ms`, `90s`, `15m`, `2h`, `1d` and concatenations like `1h30m`.
pub fn parse(input: &str) -> Result<Span, DurationError> {
    let err = || DurationError(input.to_owned());
    let s = input.trim().to_ascii_lowercase();
    if s == "0" {
        return Ok(Span::ZERO);
    }
    if s.is_empty() {
        return Err(err());
    }
    let bytes = s.as_bytes();
    let mut total: u64 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if start == i {
            return Err(err());
        }
        let value: u64 = s[start..i].parse().map_err(|_| err())?;
        let unit_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        let factor = match &s[unit_start..i] {
            "ms" => 1,
            "s" => 1_000,
            "m" => 60_000,
            "h" => 3_600_000,
            "d" => 86_400_000,
            _ => return Err(err()),
        };
        total = value.checked_mul(factor).and_then(|v| total.checked_add(v)).ok_or_else(err)?;
    }
    Ok(Span(total))
}

/// Formats compactly for display: `0s`, `850ms`, `42s`, `3m 05s`, `2h 14m`, `3d 4h`.
pub fn format(ms: u64) -> String {
    let secs = ms / 1_000;
    match ms {
        0 => "0s".to_owned(),
        1..=999 => format!("{ms}ms"),
        _ if secs < 60 => format!("{secs}s"),
        _ if secs < 3_600 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ if secs < 86_400 => format!("{}h {:02}m", secs / 3_600, (secs % 3_600) / 60),
        _ => format!("{}d {}h", secs / 86_400, (secs % 86_400) / 3_600),
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Canonical, re-parseable form (unlike `format`, which is for people).
        let ms = self.0;
        if ms == 0 {
            return f.write_str("0");
        }
        let units = [(86_400_000, "d"), (3_600_000, "h"), (60_000, "m"), (1_000, "s"), (1, "ms")];
        let mut rest = ms;
        for (size, suffix) in units {
            if rest >= size {
                write!(f, "{}{}", rest / size, suffix)?;
                rest %= size;
            }
        }
        Ok(())
    }
}

impl Serialize for Span {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Span {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_and_compound_units() {
        assert_eq!(parse("250ms").unwrap().ms(), 250);
        assert_eq!(parse("90s").unwrap().ms(), 90_000);
        assert_eq!(parse("15m").unwrap().ms(), 900_000);
        assert_eq!(parse("1h30m").unwrap().ms(), 5_400_000);
        assert_eq!(parse(" 2H ").unwrap().ms(), 7_200_000);
        assert_eq!(parse("0").unwrap(), Span::ZERO);
    }

    #[test]
    fn rejects_bare_numbers_and_unknown_units() {
        // A bare "15" is ambiguous between seconds and minutes, so it is refused.
        for bad in ["", "15", "m", "10x", "1.5h", "-5m", "5 m"] {
            assert!(parse(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn rejects_overflow() {
        assert!(parse("999999999999999999d").is_err());
    }

    #[test]
    fn display_round_trips() {
        for ms in [0, 1, 999, 1_000, 61_000, 5_400_000, 90_061_001] {
            assert_eq!(parse(&Span(ms).to_string()).unwrap().ms(), ms);
        }
    }

    #[test]
    fn formats_for_people() {
        assert_eq!(format(0), "0s");
        assert_eq!(format(850), "850ms");
        assert_eq!(format(42_000), "42s");
        assert_eq!(format(185_000), "3m 05s");
        assert_eq!(format(8_040_000), "2h 14m");
        assert_eq!(format(273_600_000), "3d 4h");
    }
}
