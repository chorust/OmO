//! Shared civil-date / RFC 3339 UTC helpers (Howard Hinnant's algorithm, days relative to 1970-01-01).

use std::time::{SystemTime, UNIX_EPOCH};

/// Convert a proleptic Gregorian civil date `(year, month, day)` to days since `1970-01-01`.
pub fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if month > 2 { month - 3 } else { month + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

/// Convert days since `1970-01-01` to proleptic Gregorian `(year, month, day)`.
pub fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as i64;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as i64;
    let year = if month <= 2 { y + 1 } else { y };
    (year, month, day)
}

/// Convert Unix epoch seconds to `(year, month, day, hour, minute, second)` in UTC.
pub fn unix_to_civil(unix: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    (
        year as i32,
        month as u32,
        day as u32,
        secs / 3_600,
        (secs % 3_600) / 60,
        secs % 60,
    )
}

/// Format Unix epoch seconds as an RFC 3339 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn format_utc_rfc3339(secs_since_epoch: i64) -> String {
    let (year, month, day, hour, minute, second) = unix_to_civil(secs_since_epoch);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Current wall-clock UTC time formatted as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now_utc_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    format_utc_rfc3339(secs)
}

#[cfg(test)]
mod tests {
    use super::{civil_from_days, days_from_civil, format_utc_rfc3339};

    #[test]
    fn roundtrips_epoch_leap_and_year_boundaries() {
        assert_eq!(format_utc_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc_rfc3339(1_709_251_199), "2024-02-29T23:59:59Z");
        assert_eq!(format_utc_rfc3339(1_709_251_200), "2024-03-01T00:00:00Z");
        assert_eq!(format_utc_rfc3339(1_798_761_599), "2026-12-31T23:59:59Z");
        assert_eq!(format_utc_rfc3339(1_798_761_600), "2027-01-01T00:00:00Z");
        assert_eq!(format_utc_rfc3339(4_107_542_399), "2100-02-28T23:59:59Z");
        assert_eq!(format_utc_rfc3339(4_107_542_400), "2100-03-01T00:00:00Z");
        for (y, m, d) in [(1970, 1, 1), (2024, 2, 29), (2026, 11, 23), (2100, 3, 1)] {
            assert_eq!(civil_from_days(days_from_civil(y, m, d)), (y, m, d));
        }
    }
}
