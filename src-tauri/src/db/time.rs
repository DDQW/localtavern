//! Timestamp text format. The C# app stored `DateTimeOffset.ToString("O")` (round-trip format:
//! `2026-09-29T12:34:56.1234567+00:00`), and rows are sorted as text (`ORDER BY UpdatedAt`), so new
//! writes must use the same shape to keep ordering consistent with existing data.

use chrono::{DateTime, SecondsFormat, Utc};

/// Formats like .NET's "O" for a UTC value: 7 fractional digits and a `+00:00` offset.
pub fn format(dt: &DateTime<Utc>) -> String {
    let base = dt.to_rfc3339_opts(SecondsFormat::Nanos, false); // ...:56.123456789+00:00
    // Nanos gives 9 digits; .NET ticks are 100ns, i.e. 7 digits.
    match base.split_once('.') {
        Some((head, tail)) => {
            let (frac, offset) = tail.split_at(9);
            format!("{head}.{}{offset}", &frac[..7])
        }
        None => base,
    }
}

pub fn parse(s: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    DateTime::parse_from_rfc3339(s).map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn formats_like_dotnet_round_trip() {
        let dt = Utc.with_ymd_and_hms(2026, 9, 29, 12, 34, 56).unwrap()
            + chrono::Duration::nanoseconds(123_456_700);
        assert_eq!(format(&dt), "2026-09-29T12:34:56.1234567+00:00");
    }

    #[test]
    fn parses_dotnet_output_with_offset() {
        let dt = parse("2026-09-29T14:34:56.1234567+02:00").unwrap();
        assert_eq!(format(&dt), "2026-09-29T12:34:56.1234567+00:00");
    }

    #[test]
    fn round_trips() {
        let now = Utc::now();
        let back = parse(&format(&now)).unwrap();
        // 7-digit truncation loses sub-100ns precision only.
        assert!((now - back).num_nanoseconds().unwrap().abs() < 100);
    }
}
