//! sRGB colours as written in configuration files.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An opaque sRGB colour. Opacity is configured separately, because softening a
/// tint by lightening its colour washes it out to white rather than making it gentler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Names accepted wherever a colour is expected, alongside `#RRGGBB` and `#RGB`.
pub const NAMED: &[(&str, Rgb)] = &[
    ("green", Rgb::new(0x00, 0xFF, 0x5A)),
    ("blue", Rgb::new(0x08, 0xA9, 0xFF)),
    ("purple", Rgb::new(0x8B, 0x2F, 0xCE)),
    ("red", Rgb::new(0xFF, 0x3B, 0x30)),
    ("amber", Rgb::new(0xFF, 0xAA, 0x00)),
    ("violet", Rgb::new(0xA8, 0x55, 0xF7)),
    ("lavender", Rgb::new(0xB9, 0xA7, 0xFF)),
    ("indigo", Rgb::new(0x7C, 0x6B, 0xFF)),
    ("teal", Rgb::new(0x00, 0xC9, 0xA7)),
    ("pink", Rgb::new(0xFF, 0x7A, 0xB8)),
    ("cyan", Rgb::new(0x00, 0xE5, 0xFF)),
    ("white", Rgb::new(0xFF, 0xFF, 0xFF)),
];

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// Relative luminance per WCAG 2.x, in `0.0..=1.0`.
    pub fn relative_luminance(self) -> f64 {
        fn channel(c: u8) -> f64 {
            let c = f64::from(c) / 255.0;
            if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        }
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorError(String);

impl fmt::Display for ColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a colour: {:?} (use a name such as green, or #RRGGBB)", self.0)
    }
}

impl std::error::Error for ColorError {}

impl FromStr for Rgb {
    type Err = ColorError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let s = input.trim();
        if let Some((_, rgb)) = NAMED.iter().find(|(name, _)| name.eq_ignore_ascii_case(s)) {
            return Ok(*rgb);
        }
        let hex = s.strip_prefix('#').unwrap_or(s);
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ColorError(input.to_owned()));
        }
        let nibble = |i: usize| u8::from_str_radix(&hex[i..=i], 16).map(|v| v * 17);
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16);
        let parsed = match hex.len() {
            3 => nibble(0).and_then(|r| Ok(Rgb::new(r, nibble(1)?, nibble(2)?))),
            6 => byte(0).and_then(|r| Ok(Rgb::new(r, byte(2)?, byte(4)?))),
            _ => return Err(ColorError(input.to_owned())),
        };
        parsed.map_err(|_| ColorError(input.to_owned()))
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_case_insensitively() {
        assert_eq!("Purple".parse::<Rgb>().unwrap(), Rgb::new(0x8B, 0x2F, 0xCE));
    }

    #[test]
    fn parses_long_and_short_hex_with_or_without_hash() {
        assert_eq!("#08A9FF".parse::<Rgb>().unwrap(), Rgb::new(0x08, 0xA9, 0xFF));
        assert_eq!("08a9ff".parse::<Rgb>().unwrap(), Rgb::new(0x08, 0xA9, 0xFF));
        assert_eq!("#fa0".parse::<Rgb>().unwrap(), Rgb::new(0xFF, 0xAA, 0x00));
    }

    #[test]
    fn six_letter_names_are_not_mistaken_for_hex() {
        // v1 treated any six-character string as hex, so "violet" became "#violet"
        // and silently fell back to green.
        assert_eq!("violet".parse::<Rgb>().unwrap(), Rgb::new(0xA8, 0x55, 0xF7));
        assert!("purpl3".parse::<Rgb>().is_err());
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in ["", "#", "#12345", "#GGGGGG", "notacolour", "#1234567"] {
            assert!(bad.parse::<Rgb>().is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn round_trips_through_hex() {
        let c = Rgb::new(1, 2, 254);
        assert_eq!(c.to_hex().parse::<Rgb>().unwrap(), c);
    }

    #[test]
    fn luminance_spans_black_to_white() {
        assert!(Rgb::new(0, 0, 0).relative_luminance().abs() < 1e-9);
        assert!((Rgb::new(255, 255, 255).relative_luminance() - 1.0).abs() < 1e-9);
    }
}
