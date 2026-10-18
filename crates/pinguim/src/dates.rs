use flush_core::Snowflake;

fn civil_from_days(z: u64) -> (i64, u32, u32) {
    let z = z as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

fn from_millis(ms: u64) -> String {
    let secs = ms / 1000;
    let (y, mo, d) = civil_from_days(secs / 86400);
    let rem = secs % 86400;
    format!(
        "{d:02}/{mo:02}/{y}, {:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// Data de criação do recurso (UTC, estilo `dd/mm/aaaa, hh:mm`).
pub fn from_snowflake(id: Snowflake) -> String {
    from_millis(id.timestamp_ms())
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `2024-05-06T07:08:09...` → `06/05/2024, 07:08`. `None` se fora do formato.
pub fn from_iso(raw: &str) -> Option<String> {
    let (date, time) = raw.split_once('T')?;
    let (y, rest) = date.split_once('-')?;
    let (mo, d) = rest.split_once('-')?;
    let hhmm = time.get(..5)?;
    if y.len() != 4 || !digits(y) || !digits(mo) || !digits(d) {
        return None;
    }
    if hhmm.as_bytes().get(2) != Some(&b':') || !digits(&hhmm[..2]) || !digits(&hhmm[3..]) {
        return None;
    }
    Some(format!("{d}/{mo}/{y}, {hhmm}"))
}

/// ISO curto, cru como fallback, `?` quando ausente.
pub fn short_opt(raw: Option<&str>) -> String {
    raw.and_then(from_iso)
        .or_else(|| raw.map(str::to_owned))
        .unwrap_or_else(|| "?".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snowflake_zero_is_fluxer_epoch() {
        assert_eq!(from_snowflake(Snowflake::new(0)), "01/01/2015, 00:00");
    }

    #[test]
    fn iso_formats_pt_short() {
        assert_eq!(
            from_iso("2024-05-06T07:08:09.000Z"),
            Some("06/05/2024, 07:08".into())
        );
        assert_eq!(from_iso("lixo"), None);
        assert_eq!(from_iso("2024-05-06"), None);
    }

    #[test]
    fn short_opt_falls_back() {
        assert_eq!(short_opt(None), "?");
        assert_eq!(short_opt(Some("2024-05-06T07:08:09Z")), "06/05/2024, 07:08");
    }
}
