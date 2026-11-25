//! Cron expressions (port of Yuzuha `domain/cron.ts`): five fields
//! `minute hour day-of-month month weekday`, IANA timezones and the
//! missed-run policy. Nonexistent local minutes (DST gaps) are skipped
//! and ambiguous minutes (DST folds) occur twice.

use std::collections::BTreeSet;

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp};
use thiserror::Error;

/// What a cron job does when the process wakes up past its slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissedRunPolicy {
    /// Skips the missed slot and waits for the next one (60s grace).
    Skip,
    /// Runs once as soon as possible, then continues on schedule.
    RunOnce,
}

#[derive(Debug, Error)]
pub enum CronError {
    #[error("invalid cron expression")]
    Expression,
    #[error("cron needs five fields: minute hour day-of-month month weekday")]
    FieldCount,
    #[error("invalid cron field; use numbers, `*`, lists, ranges and steps")]
    Field,
    #[error("cron step outside the field limits")]
    Step,
    #[error("a cron field must be between {min} and {max}, in ascending ranges")]
    Range { min: i64, max: i64 },
    #[error("cron matches no calendar date")]
    Unreachable,
    #[error("provide an IANA timezone, such as America/Bahia or UTC")]
    Timezone,
    #[error("invalid reference instant")]
    Instant,
}

/// A parsed five-field cron expression.
#[derive(Debug, Clone)]
pub struct CronExpression {
    pub minutes: Vec<i64>,
    pub hours: Vec<i64>,
    pub days: Vec<i64>,
    pub months: Vec<i64>,
    /// Weekdays with Sunday as 0 (a 7 in the expression folds to 0).
    pub weekdays: Vec<i64>,
    pub any_day: bool,
    pub any_weekday: bool,
}

fn field(source: &str, min: i64, max: i64) -> Result<Vec<i64>, CronError> {
    let mut values = BTreeSet::new();
    for part in source.split(',') {
        let (base, step) = match part.split_once('/') {
            Some((b, s)) => (b, s),
            None => (part, ""),
        };
        let has_step = part.contains('/');
        if has_step && step.is_empty() {
            // `15/` has no step: refuse rather than expand to every 1.
            return Err(CronError::Field);
        }
        let step: i64 = if step.is_empty() {
            1
        } else {
            step.parse().map_err(|_| CronError::Field)?
        };
        if step < 1 || step > max - min + 1 {
            return Err(CronError::Step);
        }
        let (start, end): (i64, i64) = if base == "*" {
            (min, max)
        } else if let Some((a, b)) = base.split_once('-') {
            (
                a.parse().map_err(|_| CronError::Field)?,
                b.parse().map_err(|_| CronError::Field)?,
            )
        } else {
            let v: i64 = base.parse().map_err(|_| CronError::Field)?;
            // `N` alone means N; `N/s` means N through max every s.
            (v, if has_step { max } else { v })
        };
        if start < min || end > max || start > end {
            return Err(CronError::Range { min, max });
        }
        let mut value = start;
        while value <= end {
            values.insert(value);
            value += step;
        }
    }
    Ok(values.into_iter().collect())
}

/// Parses `minute hour day month weekday`. A 7 in the weekday field is Sunday.
pub fn parse_cron(expression: &str) -> Result<CronExpression, CronError> {
    if expression.is_empty() || expression.len() > 256 {
        return Err(CronError::Expression);
    }
    let parts: Vec<&str> = expression.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(CronError::FieldCount);
    }
    let weekdays: BTreeSet<i64> = field(parts[4], 0, 7)?.into_iter().map(|d| d % 7).collect();
    let parsed = CronExpression {
        minutes: field(parts[0], 0, 59)?,
        hours: field(parts[1], 0, 23)?,
        days: field(parts[2], 1, 31)?,
        months: field(parts[3], 1, 12)?,
        weekdays: weekdays.into_iter().collect(),
        any_day: parts[2].starts_with('*'),
        any_weekday: parts[4].starts_with('*'),
    };
    let month_lengths = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if (parsed.any_day || parsed.any_weekday)
        && !parsed
            .months
            .iter()
            .any(|m| parsed.days.iter().any(|d| *d <= month_lengths[(*m - 1) as usize]))
    {
        return Err(CronError::Unreachable);
    }
    Ok(parsed)
}

/// Validates an IANA timezone name.
pub fn assert_timezone(timezone: &str) -> Result<(), CronError> {
    lookup_timezone(timezone).map(|_| ())
}

fn lookup_timezone(timezone: &str) -> Result<TimeZone, CronError> {
    if timezone.is_empty() || timezone.len() > 100 || timezone.starts_with(['+', '-']) {
        return Err(CronError::Timezone);
    }
    TimeZone::get(timezone).map_err(|_| CronError::Timezone)
}

fn weekday(date: Date) -> i64 {
    match date.weekday() {
        jiff::civil::Weekday::Sunday => 0,
        jiff::civil::Weekday::Monday => 1,
        jiff::civil::Weekday::Tuesday => 2,
        jiff::civil::Weekday::Wednesday => 3,
        jiff::civil::Weekday::Thursday => 4,
        jiff::civil::Weekday::Friday => 5,
        jiff::civil::Weekday::Saturday => 6,
    }
}

fn matches_day(expression: &CronExpression, date: Date) -> bool {
    if !expression.months.contains(&i64::from(date.month())) {
        return false;
    }
    let day = expression.days.contains(&i64::from(date.day()));
    let wday = expression.weekdays.contains(&weekday(date));
    // Vixie cron semantics: both restricted → OR, otherwise AND.
    if expression.any_day || expression.any_weekday {
        day && wday
    } else {
        day || wday
    }
}

/// Returns the first instant (ms since the Unix epoch) strictly after `after`
/// that matches `expression` in `timezone`. Errors when nothing matches within
/// the next eight years.
pub fn next_cron(expression: &str, timezone: &str, after: i64) -> Result<i64, CronError> {
    let parsed = parse_cron(expression)?;
    let tz = lookup_timezone(timezone)?;
    let after_ts = Timestamp::from_millisecond(after).map_err(|_| CronError::Instant)?;
    let start = after_ts.to_zoned_datetime(tz.clone()).date();
    for offset in 0..=(8 * 366 + 2) {
        let date = start
            .checked_add(Span::new().days(offset))
            .map_err(|_| CronError::Unreachable)?;
        if !matches_day(&parsed, date) {
            continue;
        }
        let mut candidates = BTreeSet::new();
        for hour in &parsed.hours {
            for minute in &parsed.minutes {
                let local = date.at(*hour as i8, *minute as i8, 0, 0);
                let ambiguous = tz.to_ambiguous_zoned_datetime(local);
                for zoned in [ambiguous.clone().earlier(), ambiguous.later()].into_iter().flatten() {
                    // Gap interpretations never land on the requested local
                    // time (skipped); both fold interpretations do (runs twice).
                    if zoned.datetime() == local {
                        let ms = zoned.timestamp().as_millisecond();
                        if ms > after {
                            candidates.insert(ms);
                        }
                    }
                }
            }
        }
        if let Some(first) = candidates.into_iter().next() {
            return Ok(first);
        }
    }
    Err(CronError::Unreachable)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: i64 = 60_000;

    fn at(ms: i64) -> String {
        Timestamp::from_millisecond(ms)
            .unwrap()
            .to_zoned_datetime(TimeZone::UTC)
            .to_string()
    }

    #[test]
    fn parses_every_fifteen_minutes() {
        let parsed = parse_cron("*/15 * * * *").unwrap();
        assert_eq!(parsed.minutes, vec![0, 15, 30, 45]);
        assert!(parsed.any_day && parsed.any_weekday);
    }

    #[test]
    fn rejects_bad_inputs() {
        assert!(matches!(parse_cron("* * * *"), Err(CronError::FieldCount)));
        assert!(matches!(
            parse_cron("61 * * * *"),
            Err(CronError::Range { .. })
        ));
        assert!(matches!(parse_cron("a * * * *"), Err(CronError::Field)));
        // February 31st never happens.
        assert!(matches!(
            parse_cron("0 0 31 2 *"),
            Err(CronError::Unreachable)
        ));
        assert!(matches!(parse_cron("0 0 * * x"), Err(CronError::Field)));
        // A `/` with no step is invalid, not `step 1`.
        assert!(matches!(parse_cron("15/ * * * *"), Err(CronError::Field)));
        assert!(matches!(parse_cron("*/ * * * *"), Err(CronError::Field)));
        assert!(matches!(parse_cron("10-20/ * * * *"), Err(CronError::Field)));
    }

    #[test]
    fn next_quarter_hour_utc() {
        let base = Timestamp::from_millisecond(1_700_000_000_000).unwrap();
        let next = next_cron("*/15 * * * *", "UTC", base.as_millisecond()).unwrap();
        assert_eq!(next % (15 * MINUTE), 0);
        assert!(next > base.as_millisecond());
    }

    #[test]
    fn honors_timezone() {
        // Daily at 9:00 in Bahia (UTC-3) → 12:00 UTC.
        let after = Timestamp::from_second(1_700_000_000)
            .unwrap()
            .as_millisecond();
        let next = next_cron("0 9 * * *", "America/Bahia", after).unwrap();
        let zoned = Timestamp::from_millisecond(next)
            .unwrap()
            .to_zoned_datetime(TimeZone::get("America/Bahia").unwrap());
        assert_eq!(zoned.hour(), 9);
        assert_eq!(zoned.minute(), 0);
        assert!(next > after);
    }

    #[test]
    fn rejects_unknown_timezone() {
        assert!(matches!(
            next_cron("0 0 * * *", "Not/AZone", 0),
            Err(CronError::Timezone)
        ));
        assert!(matches!(
            next_cron("0 0 * * *", "+3", 0),
            Err(CronError::Timezone)
        ));
    }

    #[test]
    fn weekday_and_day_use_or_semantics() {
        // `13` OR Friday (`5`): every Friday and every 13th.
        let after = Timestamp::from_second(1_699_603_200).unwrap().as_millisecond(); // 2023-11-10 Fri
        let next = next_cron("0 12 13 * 5", "UTC", after).unwrap();
        let next2 = next_cron("0 12 13 * 5", "UTC", next).unwrap();
        // 2023-11-10 12:00 is Friday → next is Monday 13th.
        assert!(at(next).starts_with("2023-11-13T12:00") || at(next).starts_with("2023-11-10T12:00"));
        assert!(next2 > next);
    }

    #[test]
    fn skipped_local_minutes_do_not_run() {
        // America/New_York 2024-03-10: 02:00–02:59 never happened (spring forward).
        // `0 2 * * *` must land on 03:00 of that day or the next day, never 02:xx.
        let after = Timestamp::from_second(1_710_000_000).unwrap().as_millisecond(); // 2024-03-09 ~13:20 UTC
        let next = next_cron("0 2 * * *", "America/New_York", after).unwrap();
        let zoned = Timestamp::from_millisecond(next)
            .unwrap()
            .to_zoned_datetime(TimeZone::get("America/New_York").unwrap());
        assert_eq!(zoned.hour(), 2);
        // 2024-03-10 has no 02:00 locally; next valid 02:00 is Mar 11.
        assert_eq!(format!("{}", zoned.date()), "2024-03-11");
    }

    #[test]
    fn fold_minutes_run_twice() {
        // America/New_York 2024-11-03: local 01:30 happened twice, first at
        // 05:30 UTC (EDT) and again one hour later at 06:30 UTC (EST).
        let first = Timestamp::from_second(1_730_611_800)
            .unwrap()
            .as_millisecond();
        let next = next_cron("30 1 * * *", "America/New_York", first - 1).unwrap();
        assert_eq!(next, first);
        let second = next_cron("30 1 * * *", "America/New_York", next).unwrap();
        assert_eq!(second, first + 3_600_000);
        let zoned = Timestamp::from_millisecond(second)
            .unwrap()
            .to_zoned_datetime(TimeZone::get("America/New_York").unwrap());
        assert_eq!((zoned.hour(), zoned.minute()), (1, 30));
    }
}
