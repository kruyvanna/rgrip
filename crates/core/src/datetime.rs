//! Date/time values as Postgres prints them, for the inline-edit picker.
//! Only the shapes the picker needs: no calendar library required.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemporalKind {
    Date,
    Timestamp,
    TimestampTz,
    Time,
    TimeTz,
}

impl TemporalKind {
    pub fn has_date(self) -> bool {
        matches!(self, Self::Date | Self::Timestamp | Self::TimestampTz)
    }

    pub fn has_time(self) -> bool {
        !matches!(self, Self::Date)
    }

    pub fn has_tz(self) -> bool {
        matches!(self, Self::TimestampTz | Self::TimeTz)
    }
}

/// From `format_type` output: `date`, `timestamp(3) with time zone`, ….
/// Arrays and intervals aren't pickable.
pub fn kind_of(data_type: &str) -> Option<TemporalKind> {
    let t = data_type.to_ascii_lowercase();
    if t.ends_with("[]") {
        return None;
    }
    let tz = t.contains("with time zone") && !t.contains("without");
    Some(match () {
        _ if t == "date" => TemporalKind::Date,
        _ if t.starts_with("timestamp") && tz => TemporalKind::TimestampTz,
        _ if t.starts_with("timestamp") => TemporalKind::Timestamp,
        _ if t.starts_with("time") && tz => TemporalKind::TimeTz,
        _ if t.starts_with("time") => TemporalKind::Time,
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Temporal {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// Fractional seconds as printed, with the dot (`.123`), or empty.
    pub frac: String,
    /// Zone offset as printed (`+07`, `-03:30`), or empty.
    pub tz: String,
}

fn num<T: std::str::FromStr>(s: &str, len: usize) -> Option<T> {
    (s.len() == len && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

fn parse_date(s: &str) -> Option<(i32, u32, u32)> {
    let mut it = s.split('-');
    let (y, m, d) = (num(it.next()?, 4)?, num(it.next()?, 2)?, num(it.next()?, 2)?);
    (it.next().is_none() && (1..=12).contains(&m) && (1..=days_in_month(y, m)).contains(&d)).then_some((y, m, d))
}

/// `HH:MM:SS[.fff][±zz[:zz]]`
fn parse_time(s: &str) -> Option<(u32, u32, u32, String, String)> {
    let split = s.find(['+', '-']).unwrap_or(s.len());
    let (clock, tz) = s.split_at(split);
    let (clock, frac) = match clock.find('.') {
        Some(i) => (&clock[..i], clock[i..].to_string()),
        None => (clock, String::new()),
    };
    let mut it = clock.split(':');
    let (h, m) = (num(it.next()?, 2)?, num(it.next()?, 2)?);
    let s = it.next().map_or(Some(0), |s| num(s, 2))?;
    (it.next().is_none() && h < 24 && m < 60 && s < 60).then(|| (h, m, s, frac, tz.to_string()))
}

/// Parse a value of `kind` as Postgres prints it (ISO style). `None` for
/// anything else (BC dates, `infinity`, other DateStyles).
pub fn parse(kind: TemporalKind, text: &str) -> Option<Temporal> {
    let text = text.trim();
    let (date, time) = match kind {
        TemporalKind::Date => (Some(text), None),
        TemporalKind::Time | TemporalKind::TimeTz => (None, Some(text)),
        _ => {
            let (d, t) = text.split_once([' ', 'T'])?;
            (Some(d), Some(t))
        }
    };
    let (year, month, day) = match date {
        Some(d) => parse_date(d)?,
        None => (2000, 1, 1),
    };
    let (hour, minute, second, frac, tz) = match time {
        Some(t) => parse_time(t)?,
        None => (0, 0, 0, String::new(), String::new()),
    };
    if !kind.has_tz() && !tz.is_empty() {
        return None;
    }
    Some(Temporal { year, month, day, hour, minute, second, frac, tz })
}

/// The literal to store, in the same shape Postgres prints.
pub fn format(kind: TemporalKind, t: &Temporal) -> String {
    let date = format!("{:04}-{:02}-{:02}", t.year, t.month, t.day);
    let time = format!("{:02}:{:02}:{:02}{}", t.hour, t.minute, t.second, t.frac);
    let tz = if kind.has_tz() { t.tz.as_str() } else { "" };
    match kind {
        TemporalKind::Date => date,
        TemporalKind::Timestamp | TemporalKind::TimestampTz => format!("{date} {time}{tz}"),
        TemporalKind::Time | TemporalKind::TimeTz => format!("{time}{tz}"),
    }
}

pub fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 (Howard Hinnant's `days_from_civil`).
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// 0 = Monday … 6 = Sunday.
pub fn weekday(y: i32, m: u32, d: u32) -> u32 {
    // 1970-01-01 was a Thursday.
    (days_from_civil(y, m, d) + 3).rem_euclid(7) as u32
}

/// Inverse of [`days_from_civil`].
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (yoe + era * 400 + if m <= 2 { 1 } else { 0 }) as i32;
    (y, m, d)
}

/// The wall-clock time `unix_secs + offset_secs`, with the offset as Postgres
/// prints it (`+07`, `-03:30`).
pub fn from_unix(unix_secs: i64, offset_secs: i64) -> Temporal {
    let local = unix_secs + offset_secs;
    let (year, month, day) = civil_from_days(local.div_euclid(86400));
    let secs = local.rem_euclid(86400);
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let (oh, om) = (offset_secs.abs() / 3600, offset_secs.abs() % 3600 / 60);
    let tz = if om == 0 { format!("{sign}{oh:02}") } else { format!("{sign}{oh:02}:{om:02}") };
    Temporal {
        year,
        month,
        day,
        hour: (secs / 3600) as u32,
        minute: (secs % 3600 / 60) as u32,
        second: (secs % 60) as u32,
        frac: String::new(),
        tz,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TemporalKind::*;

    #[test]
    fn kinds() {
        assert_eq!(kind_of("date"), Some(Date));
        assert_eq!(kind_of("timestamp without time zone"), Some(Timestamp));
        assert_eq!(kind_of("timestamp(3) with time zone"), Some(TimestampTz));
        assert_eq!(kind_of("time without time zone"), Some(Time));
        assert_eq!(kind_of("time(0) with time zone"), Some(TimeTz));
        assert_eq!(kind_of("date[]"), None);
        assert_eq!(kind_of("interval"), None);
        assert_eq!(kind_of("text"), None);
    }

    #[test]
    fn round_trips() {
        for (kind, s) in [
            (Date, "2024-02-29"),
            (Timestamp, "2024-03-05 14:30:00"),
            (Timestamp, "2024-03-05 14:30:00.123456"),
            (TimestampTz, "2024-03-05 14:30:00+07"),
            (TimestampTz, "2024-03-05 14:30:00.5-03:30"),
            (Time, "09:05:07"),
            (TimeTz, "09:05:07+05:45"),
        ] {
            let t = parse(kind, s).unwrap_or_else(|| panic!("{s}"));
            assert_eq!(format(kind, &t), s);
        }
        assert_eq!(parse(Timestamp, "2024-03-05T14:30").map(|t| format(Timestamp, &t)).as_deref(), Some("2024-03-05 14:30:00"));
        for (kind, s) in [(Date, "2023-02-29"), (Date, "infinity"), (Date, "0044-03-15 BC"), (Time, "24:00:00"), (Timestamp, "2024-03-05 14:30:00+07"), (Date, "")] {
            assert!(parse(kind, s).is_none(), "{s}");
        }
    }

    #[test]
    fn calendar() {
        assert_eq!(weekday(1970, 1, 1), 3);
        assert_eq!(weekday(2026, 10, 13), 1); // a Tuesday
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        for z in [-800_000, -1, 0, 1, 19_000, 2_000_000] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
        let t = from_unix(1_791_883_758, 7 * 3600);
        assert_eq!(format(TimestampTz, &t), "2026-10-13 16:29:18+07");
        assert_eq!(from_unix(0, -(3 * 3600 + 1800)).tz, "-03:30");
    }
}
