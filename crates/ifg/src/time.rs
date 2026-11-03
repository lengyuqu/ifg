//! Timestamp helpers: parse RFC 3339 (the server format) without a date
//! library and render relative ages for interactive output.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Parse `YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)` into a `SystemTime`.
pub fn parse_rfc3339(text: &str) -> Option<SystemTime> {
    let bytes = text.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    let year: i64 = text.get(0..4)?.parse().ok()?;
    let month: u32 = text.get(5..7)?.parse().ok()?;
    let day: u32 = text.get(8..10)?.parse().ok()?;
    let hour: u32 = text.get(11..13)?.parse().ok()?;
    let minute: u32 = text.get(14..16)?.parse().ok()?;
    let second: u32 = text.get(17..19)?.parse().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let mut seconds =
        days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second);

    // Timezone suffix: optional fractional seconds, then Z or ±HH:MM.
    let mut idx = 19;
    let bytes = text.as_bytes();
    if bytes.get(idx) == Some(&b'.') {
        idx += 1;
        while bytes.get(idx).is_some_and(u8::is_ascii_digit) {
            idx += 1;
        }
    }
    let tz = text.get(idx..).unwrap_or("");
    let offset = match tz {
        "" | "Z" | "z" => 0,
        _ if tz.starts_with('+') || tz.starts_with('-') => {
            let sign = if tz.starts_with('-') { -1 } else { 1 };
            let (hours, minutes) = tz.get(1..)?.split_once(':')?;
            sign * (hours.parse::<i64>().ok()? * 3_600 + minutes.parse::<i64>().ok()? * 60)
        }
        _ => return None,
    };
    seconds -= offset;

    if seconds < 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(seconds as u64))
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day = i64::from(day);
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// "3m ago"-style age; `None` when the timestamp cannot be parsed.
pub fn relative(iso: &str) -> Option<String> {
    let then = parse_rfc3339(iso)?;
    let now = SystemTime::now();
    let age = now.duration_since(then).unwrap_or(Duration::ZERO);
    let seconds = age.as_secs();
    let text = match seconds {
        0..=59 => "just now".to_string(),
        60..=3_599 => format!("{}m ago", seconds / 60),
        3_600..=86_399 => format!("{}h ago", seconds / 3_600),
        86_400..=604_799 => format!("{}d ago", seconds / 86_400),
        604_800..=2_592_000 => format!("{}w ago", seconds / 604_800),
        2_592_001..=31_535_999 => format!("{}mo ago", seconds / 2_592_000),
        _ => format!("{}y ago", seconds / 31_536_000),
    };
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_zulu_and_offsets() {
        let zulu = parse_rfc3339("2026-11-03T10:00:00Z").unwrap();
        let offset = parse_rfc3339("2026-11-03T18:00:00+08:00").unwrap();
        assert_eq!(zulu, offset);

        let fractional = parse_rfc3339("2026-11-03T10:00:00.123456Z").unwrap();
        let whole = parse_rfc3339("2026-11-03T10:00:00Z").unwrap();
        assert!(fractional.duration_since(whole).unwrap().as_secs() < 1);

        assert!(parse_rfc3339("not a date").is_none());
        assert!(parse_rfc3339("2026-13-40T99:00:00Z").is_none());
    }

    #[test]
    fn renders_relative_ages() {
        // A timestamp in the past by a known delta, built from now().
        let now = SystemTime::now();
        let two_hours_ago = now - Duration::from_secs(2 * 3_600 + 30);
        let seconds = two_hours_ago.duration_since(UNIX_EPOCH).unwrap().as_secs();
        let iso = format_iso(seconds);
        assert_eq!(relative(&iso).as_deref(), Some("2h ago"));
    }

    /// Minimal reverse formatting for tests (UTC, no fractional part).
    fn format_iso(seconds: u64) -> String {
        let days = (seconds / 86_400) as i64;
        let rem = seconds % 86_400;
        let (year, month, day) = civil_from_days(days);
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
            rem / 3_600,
            (rem % 3_600) / 60,
            rem % 60
        )
    }

    fn civil_from_days(days: i64) -> (i64, u32, u32) {
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let day_of_era = z - era * 146_097;
        let year_of_era =
            (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_prime = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
        let month = if month_prime < 10 {
            month_prime + 3
        } else {
            month_prime - 9
        };
        let year = if month <= 2 { year + 1 } else { year };
        (year, month as u32, day as u32)
    }
}
