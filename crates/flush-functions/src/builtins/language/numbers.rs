//! `$round`/`$floor`/`$ceil`/`$abs`/`$clamp`/`$fixed`/`$formatNumber`/
//! `$formatBytes`/`$percent`/`$toBase`/`$fromBase`/`$date`/`$relativeTime`/
//! `$parseDate` (yuzuha `builtins/language/numbers.ts`).

use num_bigint::BigInt;
use serde_json::Value;

use super::common::{finite_number, integer, number};
use crate::errors::{FnError, ScriptFailure};
use crate::types::Definitions;
use crate::values::{js_number, text};

const PT_BR: &str = "pt-BR";

fn normalize_locale(raw: &str) -> Result<String, ScriptFailure> {
    let trimmed = raw.trim();
    let locale = if trimmed.is_empty() {
        PT_BR.to_string()
    } else {
        trimmed.to_string()
    };
    match locale.as_str() {
        "pt-BR" | "pt" | "en-US" | "en" => Ok(locale),
        _ => Err(FnError::new("NUMBER", "Invalid locale.").into()),
    }
}

fn round_to(value: f64, places: i64) -> f64 {
    let factor = 10_f64.powi(places as i32);
    (value * factor).round() / factor
}

fn parse_moment(raw: &str, ctx: &crate::context::ScriptContext) -> Result<i64, ScriptFailure> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok((ctx.now)() as i64);
    }
    let timestamp = js_number(trimmed);
    match timestamp.filter(|v| v.is_finite()) {
        Some(v) => Ok(v as i64),
        None => Err(FnError::new("DATE", "Timestamp must be a number in milliseconds.").into()),
    }
}

fn format_grouped(value: f64, locale: &str) -> String {
    // Intl.NumberFormat approximation for pt-BR and en-US.
    let (thousands, decimal_point) = if locale.starts_with("pt") {
        ('.', ',')
    } else {
        (',', '.')
    };
    if !value.is_finite() {
        return text(&number(value));
    }
    let negative = value < 0.0;
    let abs = value.abs();
    let int_part = abs.trunc() as u64;
    let mut grouped = String::new();
    for (i, ch) in int_part.to_string().chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            grouped.push(thousands);
        }
        grouped.push(ch);
    }
    let mut grouped: String = grouped.chars().rev().collect();
    let frac = abs.fract();
    if frac > 0.0 {
        let mut frac_text = format!("{frac:.15}");
        frac_text = frac_text.trim_start_matches("0.").to_string();
        let frac_text = frac_text.trim_end_matches('0');
        if !frac_text.is_empty() {
            grouped.push(decimal_point);
            grouped.push_str(frac_text);
        }
    }
    format!("{}{grouped}", if negative { "-" } else { "" })
}

// Civil date decomposition (proleptic Gregorian), UTC.
fn civil_from_ms(ms: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (mp + if mp < 10 { 3 } else { -9 }) as u32;
    let year = y + if m <= 2 { 1 } else { 0 };
    (
        year,
        m,
        d,
        (rem / 3_600_000) as u32,
        (rem / 60_000 % 60) as u32,
        (rem / 1000 % 60) as u32,
    )
}

fn date_parts(ms: i64, locale: &str, part: &str) -> String {
    let (y, m, d, hh, mm, ss) = civil_from_ms(ms);
    let brazil = locale.starts_with("pt");
    match part {
        "date" => {
            if brazil {
                format!("{d:02}/{m:02}/{y:04}")
            } else {
                format!("{m}/{d}/{y:02}", y = y % 100)
            }
        }
        "time" => {
            if brazil {
                format!("{hh:02}:{mm:02}:{ss:02}")
            } else {
                let (h12, suffix) = if hh % 12 == 0 {
                    (12, if hh < 12 { "AM" } else { "PM" })
                } else {
                    (hh % 12, if hh < 12 { "AM" } else { "PM" })
                };
                format!("{h12}:{mm:02}:{ss:02} {suffix}")
            }
        }
        _ => {
            if brazil {
                format!("{d:02}/{m:02}/{y:04}, {hh:02}:{mm:02}:{ss:02}")
            } else {
                let (h12, suffix) = if hh % 12 == 0 {
                    (12, if hh < 12 { "AM" } else { "PM" })
                } else {
                    (hh % 12, if hh < 12 { "AM" } else { "PM" })
                };
                format!(
                    "{}/{}/{}, {}:{:02}:{:02} {}",
                    m,
                    d,
                    y % 100,
                    h12,
                    mm,
                    ss,
                    suffix
                )
            }
        }
    }
}

fn relative_amount(timestamp: i64, now: i64) -> (i64, &'static str) {
    let diff = timestamp - now;
    let abs = diff.abs();
    for (unit, ms) in [
        ("year", 365 * 24 * 3_600_000_i64),
        ("month", 30 * 24 * 3_600_000),
        ("day", 24 * 3_600_000),
        ("hour", 3_600_000),
        ("minute", 60_000),
        ("second", 1_000),
    ] {
        if abs >= ms || unit == "second" {
            return ((diff as f64 / ms as f64).round() as i64, unit);
        }
    }
    (0, "second")
}

fn format_relative(amount: i64, unit: &str, locale: &str) -> String {
    // Intl.RelativeTimeFormat(numeric: 'auto') approximation.
    let brazil = locale.starts_with("pt");
    let (one, other) = if brazil {
        match unit {
            "year" => ("ano", "anos"),
            "month" => ("mês", "meses"),
            "day" => ("dia", "dias"),
            "hour" => ("hora", "horas"),
            "minute" => ("minuto", "minutos"),
            _ => ("segundo", "segundos"),
        }
    } else {
        match unit {
            "year" => ("year", "years"),
            "month" => ("month", "months"),
            "day" => ("day", "days"),
            "hour" => ("hour", "hours"),
            "minute" => ("minute", "minutes"),
            _ => ("second", "seconds"),
        }
    };
    if amount == 0 {
        return if brazil {
            match unit {
                "day" => "hoje".to_string(),
                "hour" => "nesta hora".to_string(),
                "minute" => "neste minuto".to_string(),
                "second" => "agora".to_string(),
                "month" => "neste mês".to_string(),
                _ => "neste ano".to_string(),
            }
        } else {
            match unit {
                "day" => "today".to_string(),
                "hour" => "this hour".to_string(),
                "minute" => "this minute".to_string(),
                "second" => "now".to_string(),
                "month" => "this month".to_string(),
                _ => "this year".to_string(),
            }
        };
    }
    let abs = amount.abs();
    let word = if abs == 1 { one } else { other };
    if brazil {
        if amount < 0 {
            format!("há {abs} {word}")
        } else {
            format!("em {abs} {word}")
        }
    } else if amount < 0 {
        format!("{abs} {word} ago")
    } else {
        format!("in {abs} {word}")
    }
}

fn parse_iso(raw: &str) -> Option<i64> {
    // ISO-8601 subset: YYYY-MM-DD[THH:MM[:SS[.fff]]][Z|±HH:MM].
    let raw = raw.trim();
    let bytes = raw.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    let year: i64 = raw.get(0..4)?.parse().ok()?;
    if bytes[4] != b'-' {
        return None;
    }
    let month: i64 = raw.get(5..7)?.parse().ok()?;
    if bytes[7] != b'-' {
        return None;
    }
    let day: i64 = raw.get(8..10)?.parse().ok()?;
    let rest = &raw[10..];
    let (mut hh, mut mi, mut ss, mut frac_ms) = (0i64, 0i64, 0i64, 0i64);
    let mut offset_part = "";
    let mut has_time = false;
    if rest.starts_with('T') || rest.starts_with(' ') {
        has_time = true;
        let time = &rest[1..];
        let tz_pos = time
            .find(['Z', 'z'])
            .or_else(|| time[1..].find('+').map(|i| i + 1))
            .or_else(|| time[1..].find('-').map(|i| i + 1));
        let (clock, tz) = match tz_pos {
            Some(pos) => (&time[..pos], &time[pos..]),
            None => (time, ""),
        };
        offset_part = tz;
        let pieces: Vec<&str> = clock.split(':').collect();
        if pieces.len() < 2 || pieces.len() > 3 {
            return None;
        }
        hh = pieces[0].parse().ok()?;
        mi = pieces[1].parse().ok()?;
        if pieces.len() == 3 {
            let sec = pieces[2];
            let (whole, frac) = match sec.split_once('.') {
                Some((w, f)) => (w, f),
                None => (sec, ""),
            };
            ss = whole.parse().ok()?;
            if !frac.is_empty() {
                if !frac.chars().all(|ch| ch.is_ascii_digit()) || frac.len() > 9 {
                    return None;
                }
                frac_ms = format!("{:0<3}", &frac[..frac.len().min(3)]).parse().ok()?;
            }
        }
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // days_from_civil.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let mut ms = days * 86_400_000 + hh * 3_600_000 + mi * 60_000 + ss * 1000 + frac_ms;
    if has_time && !offset_part.is_empty() {
        if offset_part == "Z" || offset_part == "z" {
            // UTC.
        } else if offset_part.len() == 6
            && matches!(offset_part.as_bytes()[0], b'+' | b'-')
            && offset_part.as_bytes()[3] == b':'
        {
            let oh: i64 = offset_part[1..3].parse().ok()?;
            let om: i64 = offset_part[4..6].parse().ok()?;
            let offset = oh * 3_600_000 + om * 60_000;
            ms += if offset_part.as_bytes()[0] == b'-' {
                offset
            } else {
                -offset
            };
        } else {
            return None;
        }
    } else if has_time && offset_part.is_empty() {
        return None; // JS requires a zone when a time is present (treated as local/UTC-dependent).
    }
    Some(ms)
}

fn decimal_places(raw: &str) -> Result<i64, ScriptFailure> {
    let places = integer(raw, "Places")?;
    if !(0..=6).contains(&places) {
        return Err(FnError::new("NUMBER", "Places must be between 0 and 6.").into());
    }
    Ok(places)
}

fn radix(raw: &str) -> Result<u32, ScriptFailure> {
    let base = integer(raw, "Base")?;
    if !(2..=36).contains(&base) {
        return Err(FnError::new("BASE", "Base must be between 2 and 36.").into());
    }
    Ok(base as u32)
}

pub fn register(d: &mut Definitions) {
    for (name, description) in [
        ("round", "Rounds with decimal places."),
        ("floor", "Rounds down."),
        ("ceil", "Rounds up."),
        ("abs", "Absolute value."),
    ] {
        d.add(
            name,
            "language",
            1,
            if name == "round" { 2 } else { 1 },
            if name == "round" {
                "[value;places?]"
            } else {
                "[value]"
            },
            description,
            move |c| {
                Box::pin(async move {
                    let value = finite_number(&c.arg(0).await?, "Value")?;
                    Ok(match name {
                        "round" => {
                            let places = if c.arg_count() > 1 {
                                integer(&c.arg(1).await?, "Places")?
                            } else {
                                0
                            };
                            if !(0..=100).contains(&places) {
                                return Err(FnError::new(
                                    "NUMBER",
                                    "Places must be between 0 and 100.",
                                )
                                .into());
                            }
                            number(round_to(value, places))
                        }
                        "floor" => number(value.floor()),
                        "ceil" => number(value.ceil()),
                        _ => number(value.abs()),
                    })
                })
            },
        );
    }

    d.add(
        "clamp",
        "language",
        3,
        3,
        "[value;min;max]",
        "Clamps the value to the interval.",
        |c| {
            Box::pin(async move {
                let value = finite_number(&c.arg(0).await?, "Value")?;
                let min = finite_number(&c.arg(1).await?, "Min")?;
                let max = finite_number(&c.arg(2).await?, "Max")?;
                if min > max {
                    return Err(FnError::new("NUMBER", "Min cannot exceed max.").into());
                }
                Ok(number(value.clamp(min, max)))
            })
        },
    );

    d.add(
        "fixed",
        "language",
        1,
        2,
        "[value;places?]",
        "Text with fixed decimal places.",
        |c| {
            Box::pin(async move {
                let value = finite_number(&c.arg(0).await?, "Value")?;
                let places = if c.arg_count() > 1 {
                    integer(&c.arg(1).await?, "Places")?
                } else {
                    0
                };
                if !(0..=100).contains(&places) {
                    return Err(FnError::new("NUMBER", "Places must be between 0 and 100.").into());
                }
                Ok(Value::String(format!(
                    "{value:.places$}",
                    places = places as usize
                )))
            })
        },
    );

    d.add(
        "formatNumber",
        "language",
        1,
        2,
        "[value;locale?]",
        "Formats the number for the given locale.",
        |c| {
            Box::pin(async move {
                let value = finite_number(&c.arg(0).await?, "Value")?;
                let locale = if c.arg_count() > 1 {
                    normalize_locale(&c.arg(1).await?)?
                } else {
                    PT_BR.to_string()
                };
                Ok(Value::String(format_grouped(value, &locale)))
            })
        },
    );

    d.add(
        "formatBytes",
        "language",
        1,
        3,
        "[bytes;places?;base?]",
        "Formats non-negative bytes; places 0-6, base 1024 (KiB) or 1000 (kB).",
        |c| {
            Box::pin(async move {
                let bytes = finite_number(&c.arg(0).await?, "Bytes")?;
                if bytes.fract() != 0.0 || !(0.0..=9_007_199_254_740_991.0).contains(&bytes) {
                    return Err(FnError::new(
                        "NUMBER",
                        "Bytes must be a safe non-negative integer.",
                    )
                    .into());
                }
                let places = if c.arg_count() > 1 {
                    decimal_places(&c.arg(1).await?)?
                } else {
                    2
                };
                let base = if c.arg_count() > 2 {
                    integer(&c.arg(2).await?, "Base")?
                } else {
                    1024
                };
                if base != 1000 && base != 1024 {
                    return Err(FnError::new("NUMBER", "Byte base must be 1000 or 1024.").into());
                }
                let units: [&str; 6] = if base == 1024 {
                    ["B", "KiB", "MiB", "GiB", "TiB", "PiB"]
                } else {
                    ["B", "kB", "MB", "GB", "TB", "PB"]
                };
                let mut scaled = bytes;
                let mut unit = 0;
                while scaled >= base as f64 && unit < units.len() - 1 {
                    scaled /= base as f64;
                    unit += 1;
                }
                Ok(Value::String(format!(
                    "{} {}",
                    round_to(scaled, places),
                    units[unit]
                )))
            })
        },
    );

    d.add(
        "percent",
        "language",
        2,
        3,
        "[part;total;places?]",
        "Formats part/total x 100 with %; positive total, places 0-6 (default 2).",
        |c| {
            Box::pin(async move {
                let part = finite_number(&c.arg(0).await?, "Part")?;
                let total = finite_number(&c.arg(1).await?, "Total")?;
                if total <= 0.0 {
                    return Err(FnError::new("NUMBER", "Total must be positive.").into());
                }
                let value = part / total * 100.0;
                if !value.is_finite() {
                    return Err(
                        FnError::new("NUMBER", "Percentage outside the finite range.").into(),
                    );
                }
                let places = if c.arg_count() > 2 {
                    decimal_places(&c.arg(2).await?)?
                } else {
                    2
                };
                Ok(Value::String(format!(
                    "{value:.places$}%",
                    places = places as usize
                )))
            })
        },
    );

    d.add(
        "toBase",
        "language",
        2,
        2,
        "[decimalInteger;base]",
        "Converts a safe integer or exact decimal text to base 2-36; up to 1024 input digits.",
        |c| {
            Box::pin(async move {
                let raw = c.arg(0).await?;
                let digits = raw.trim();
                let valid = digits.len() <= 1024
                    && !digits.is_empty()
                    && digits
                        .strip_prefix('-')
                        .unwrap_or(digits)
                        .chars()
                        .all(|ch| ch.is_ascii_digit());
                if !valid {
                    return Err(FnError::new(
                        "BASE",
                        "Provide a decimal integer with up to 1024 digits, without prefixes or spaces.",
                    )
                    .into());
                }
                let base = radix(&c.arg(1).await?)?;
                let value: BigInt = digits.parse().map_err(|_| {
                    FnError::new("BASE", "Invalid decimal integer.")
                })?;
                Ok(Value::String(value.to_str_radix(base)))
            })
        },
    );

    d.add(
        "fromBase",
        "language",
        2,
        2,
        "[text;base]",
        "Converts base-2-36 text to exact decimal text; up to 1024 digits, no prefixes.",
        |c| {
            Box::pin(async move {
                let value = c.arg(0).await?;
                let valid = value.len() <= 1024
                    && !value.is_empty()
                    && value
                        .strip_prefix('-')
                        .unwrap_or(&value)
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric());
                if !valid {
                    return Err(FnError::new(
                        "BASE",
                        "Provide text with up to 1024 digits, without prefixes or spaces.",
                    )
                    .into());
                }
                let base = radix(&c.arg(1).await?)?;
                let (negative, digits) = match value.strip_prefix('-') {
                    Some(rest) => (true, rest),
                    None => (false, value.as_str()),
                };
                let mut result = BigInt::from(0);
                for digit in digits.to_lowercase().chars() {
                    let index = "0123456789abcdefghijklmnopqrstuvwxyz".find(digit).unwrap() as u32;
                    if index >= base {
                        return Err(FnError::new(
                            "BASE",
                            format!("Invalid digit for base {base}."),
                        )
                        .into());
                    }
                    result = result * BigInt::from(base) + BigInt::from(index);
                }
                if negative {
                    result = -result;
                }
                Ok(Value::String(result.to_str_radix(10)))
            })
        },
    );

    d.add(
        "date",
        "language",
        0,
        3,
        "[timestamp?;format?;locale?]",
        "Formats a date; timestamp in ms, format date/time/datetime/iso/relative.",
        |c| {
            Box::pin(async move {
                let timestamp = if c.arg_count() > 0 {
                    parse_moment(&c.arg(0).await?, c.ctx)?
                } else {
                    (c.ctx.now)() as i64
                };
                let format = if c.arg_count() > 1 {
                    c.arg(1).await?.trim().to_lowercase()
                } else {
                    "iso".to_string()
                };
                let format = if format.is_empty() {
                    "iso".to_string()
                } else {
                    format
                };
                let locale = if c.arg_count() > 2 {
                    normalize_locale(&c.arg(2).await?)?
                } else {
                    PT_BR.to_string()
                };
                let (y, m, day, hh, mm, ss) = civil_from_ms(timestamp);
                match format.as_str() {
                    "iso" => Ok(Value::String(format!(
                        "{y:04}-{m:02}-{day:02}T{hh:02}:{mm:02}:{ss:02}.{:03}Z",
                        timestamp.rem_euclid(1000)
                    ))),
                    "date" | "time" | "datetime" => {
                        Ok(Value::String(date_parts(timestamp, &locale, &format)))
                    }
                    "relative" => Ok(Value::String({
                        let (amount, unit) = relative_amount(timestamp, (c.ctx.now)() as i64);
                        format_relative(amount, unit, &locale)
                    })),
                    _ => Err(FnError::new(
                        "DATE",
                        "Format must be iso, date, time, datetime or relative.",
                    )
                    .into()),
                }
            })
        },
    );

    d.add(
        "relativeTime",
        "language",
        1,
        2,
        "[timestamp;locale?]",
        "Describes the distance until now, e.g. \"2 days ago\".",
        |c| {
            Box::pin(async move {
                let timestamp = parse_moment(&c.arg(0).await?, c.ctx)?;
                let locale = if c.arg_count() > 1 {
                    normalize_locale(&c.arg(1).await?)?
                } else {
                    PT_BR.to_string()
                };
                let (amount, unit) = relative_amount(timestamp, (c.ctx.now)() as i64);
                Ok(Value::String(format_relative(amount, unit, &locale)))
            })
        },
    );

    d.add(
        "parseDate",
        "language",
        1,
        1,
        "[text]",
        "Converts date text to milliseconds; error when invalid.",
        |c| {
            Box::pin(async move {
                let raw = c.arg(0).await?;
                match parse_iso(&raw) {
                    Some(ms) if !raw.trim().is_empty() => Ok(Value::from(ms)),
                    _ => Err(FnError::new("DATE", "Invalid date.").into()),
                }
            })
        },
    );
}
