//! Wall-clock formatting and local-time windows, without a date library.

use std::fmt;

/// Formats Unix milliseconds as RFC 3339 UTC, e.g. `2026-09-10T18:22:01.123Z`.
pub fn rfc3339(unix_ms: u64) -> String {
    let (y, m, d) = civil_from_days((unix_ms / 86_400_000) as i64);
    let ms_of_day = unix_ms % 86_400_000;
    let (h, min, s, ms) =
        (ms_of_day / 3_600_000, (ms_of_day / 60_000) % 60, (ms_of_day / 1_000) % 60, ms_of_day % 1_000);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}.{ms:03}Z")
}

/// Parses the subset of RFC 3339 that [`rfc3339`] produces (UTC, optional fraction).
pub fn parse_rfc3339(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<u64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let rest = &s[19..];
    let (frac_ms, tail) = match rest.strip_prefix('.') {
        Some(f) => {
            let digits = f.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 {
                return None;
            }
            let padded: String = f[..digits].chars().chain("000".chars()).take(3).collect();
            (padded.parse::<u64>().ok()?, &f[digits..])
        }
        None => (0, rest),
    };
    if tail != "Z" {
        return None;
    }
    let days = days_from_civil(y as i64, mo as u32, d as u32);
    u64::try_from(days).ok().map(|days| days * 86_400_000 + h * 3_600_000 + mi * 60_000 + se * 1_000 + frac_ms)
}

/// Calendar date `(year, month, day)` for Unix milliseconds shifted by a UTC offset.
pub fn local_date(unix_ms: u64, utc_offset_min: i32) -> (i64, u32, u32) {
    let shifted = unix_ms as i64 + i64::from(utc_offset_min) * 60_000;
    civil_from_days(shifted.div_euclid(86_400_000))
}

/// Hour of the local day, `0..24`.
pub fn local_hour(unix_ms: u64, utc_offset_min: i32) -> u32 {
    let shifted = unix_ms as i64 + i64::from(utc_offset_min) * 60_000;
    (shifted.rem_euclid(86_400_000) / 3_600_000) as u32
}

// Howard Hinnant's days <-> civil algorithms (proleptic Gregorian, exact for all i64 days).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// A time of day in whole minutes after local midnight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Clock(pub u16);

impl Clock {
    pub fn parse(s: &str) -> Option<Clock> {
        let (h, m) = s.trim().split_once(':')?;
        if h.is_empty() || h.len() > 2 || m.len() != 2 {
            return None;
        }
        let (h, m): (u16, u16) = (h.parse().ok()?, m.parse().ok()?);
        (h < 24 && m < 60).then_some(Clock(h * 60 + m))
    }
}

impl fmt::Display for Clock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.0 / 60, self.0 % 60)
    }
}

/// Whether `now` falls within `[start, end)`, wrapping past midnight when
/// `end <= start` (so `22:00`–`08:00` covers the night). Equal bounds mean never.
pub fn in_window(now: Clock, start: Clock, end: Clock) -> bool {
    match start.cmp(&end) {
        std::cmp::Ordering::Equal => false,
        std::cmp::Ordering::Less => start <= now && now < end,
        std::cmp::Ordering::Greater => now >= start || now < end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instants() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(rfc3339(1_789_064_521_123), "2026-09-10T18:22:01.123Z");
    }

    #[test]
    fn parse_inverts_format_across_centuries() {
        let mut t: u64 = 0;
        while t < 8_000_000_000_000 {
            assert_eq!(parse_rfc3339(&rfc3339(t)), Some(t), "at {t}");
            t += 97_654_321_987;
        }
    }

    #[test]
    fn parses_without_fraction_and_rejects_offsets() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:01Z"), Some(1_000));
        assert_eq!(parse_rfc3339("1970-01-01T00:00:01.5Z"), Some(1_500));
        assert_eq!(parse_rfc3339("1970-01-01T00:00:01+01:00"), None);
        assert_eq!(parse_rfc3339("1970-13-01T00:00:01Z"), None);
        assert_eq!(parse_rfc3339("garbage"), None);
    }

    #[test]
    fn local_date_and_hour_apply_offset() {
        // 2026-09-10T02:30Z is still the 9th in Seattle (UTC-7).
        let t = parse_rfc3339("2026-09-10T02:30:00Z").unwrap();
        assert_eq!(local_date(t, -420), (2026, 9, 9));
        assert_eq!(local_hour(t, -420), 19);
        assert_eq!(local_date(t, 330), (2026, 9, 10));
    }

    #[test]
    fn clock_parsing() {
        assert_eq!(Clock::parse("22:00"), Some(Clock(1_320)));
        assert_eq!(Clock::parse("7:05"), Some(Clock(425)));
        for bad in ["24:00", "12:60", "1200", "12:5", "", "ab:cd"] {
            assert_eq!(Clock::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Clock(425).to_string(), "07:05");
    }

    #[test]
    fn windows_wrap_past_midnight() {
        let c = |s| Clock::parse(s).unwrap();
        assert!(in_window(c("23:30"), c("22:00"), c("08:00")));
        assert!(in_window(c("03:00"), c("22:00"), c("08:00")));
        assert!(!in_window(c("08:00"), c("22:00"), c("08:00")));
        assert!(!in_window(c("12:00"), c("22:00"), c("08:00")));
        assert!(in_window(c("12:00"), c("09:00"), c("17:00")));
        assert!(!in_window(c("12:00"), c("09:00"), c("09:00")));
    }
}
