//! RFC 3339 timestamps as epoch seconds; the only date arithmetic this crate needs.

/// `2026-10-21T18:55:02.553823Z`, with or without a fraction, `Z` or `±hh:mm`. Harbor job files
/// omit the zone; those read as UTC, which is the best a pure parser can do.
pub(crate) fn parse_iso(text: &str) -> Option<f64> {
    let num = |at: usize, len: usize| -> Option<i64> { text.get(at..at + len)?.parse().ok() };
    if text.as_bytes().get(4) != Some(&b'-') || text.as_bytes().get(10) != Some(&b'T') {
        return None;
    }
    let (year, month, day) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (hour, minute, second) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let mut rest = text.get(19..)?;
    let mut fraction = 0.0;
    if let Some(digits) = rest.strip_prefix('.') {
        let len = digits.bytes().take_while(u8::is_ascii_digit).count();
        fraction = format!("0.{}", digits.get(..len)?).parse().ok()?;
        rest = digits.get(len..)?;
    }
    let offset = match rest.as_bytes() {
        [] | [b'Z'] => 0,
        [sign @ (b'+' | b'-'), ..] => {
            let h: i64 = rest.get(1..3)?.parse().ok()?;
            let m: i64 = rest.get(4..6)?.parse().ok()?;
            let seconds = h * 3600 + m * 60;
            if *sign == b'+' { seconds } else { -seconds }
        }
        _ => return None,
    };
    let days = days_from_civil(year, month, day);
    Some((days * 86_400 + hour * 3600 + minute * 60 + second - offset) as f64 + fraction)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_zones_and_fractions() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(parse_iso("2000-03-01T00:00:00"), Some(951_868_800.0));
        assert_eq!(parse_iso("1970-01-01T01:00:00.5+01:00"), Some(0.5));
        assert_eq!(parse_iso("1970-01-02T00:00:00-02:00"), Some(93_600.0));
        assert_eq!(parse_iso("yesterday"), None);
        assert_eq!(parse_iso("2026-13-01T00:00:00Z"), None);
    }
}
