//! Typed command arguments: a cursor over the raw `args` with parsers for
//! mentions, IDs, integers, booleans and durations.

use std::time::Duration;

use flush_core::Snowflake;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArgError {
    #[error("missing argument `{0}`")]
    Missing(String),
    #[error("invalid value `{value}` for `{name}`: expected {expected}")]
    Invalid {
        name: String,
        value: String,
        expected: &'static str,
    },
}

fn strip_mention(raw: &str, prefixes: &[&str]) -> Option<Snowflake> {
    let raw = raw.trim();
    let inner = raw.strip_prefix('<').and_then(|r| r.strip_suffix('>'));
    let digits = match inner {
        Some(inner) => prefixes
            .iter()
            .find_map(|p| inner.strip_prefix(p))
            .unwrap_or(""),
        None => raw,
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<Snowflake>().ok()
}

/// `<@id>`, `<@!id>` or a bare ID.
pub fn parse_user(raw: &str) -> Option<Snowflake> {
    strip_mention(raw, &["@!", "@"])
}

/// `<@&id>` or a bare ID.
pub fn parse_role(raw: &str) -> Option<Snowflake> {
    strip_mention(raw, &["@&"])
}

/// `<#id>` or a bare ID.
pub fn parse_channel(raw: &str) -> Option<Snowflake> {
    strip_mention(raw, &["#"])
}

/// `yes/no`, `true/false`, `on/off`, `1/0` (case-insensitive).
pub fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "y" | "on" | "1" => Some(true),
        "false" | "no" | "n" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// `90` (seconds) or unit groups such as `10m`, `1h30m`, `2d`, `1w`.
pub fn parse_duration(raw: &str) -> Option<Duration> {
    let raw = raw.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return None;
    }
    if raw.bytes().all(|b| b.is_ascii_digit()) {
        return raw.parse().ok().map(Duration::from_secs);
    }
    let mut total = 0u64;
    let mut number = String::new();
    for c in raw.chars() {
        if c.is_ascii_digit() {
            number.push(c);
            continue;
        }
        let unit = match c {
            's' => 1,
            'm' => 60,
            'h' => 3_600,
            'd' => 86_400,
            'w' => 604_800,
            _ => return None,
        };
        let value: u64 = number.parse().ok()?;
        total = total.checked_add(value.checked_mul(unit)?)?;
        number.clear();
    }
    if !number.is_empty() {
        return None;
    }
    Some(Duration::from_secs(total))
}

/// Cursor over command arguments. Each `next_*` consumes one argument.
#[derive(Debug, Clone)]
pub struct Args {
    items: Vec<String>,
    pos: usize,
}

impl Args {
    pub fn new(items: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            items: items.into_iter().map(Into::into).collect(),
            pos: 0,
        }
    }

    pub fn remaining(&self) -> usize {
        self.items.len() - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub fn peek(&self) -> Option<&str> {
        self.items.get(self.pos).map(String::as_str)
    }

    pub fn next_str(&mut self, name: &str) -> Result<String, ArgError> {
        let value = self
            .items
            .get(self.pos)
            .cloned()
            .ok_or_else(|| ArgError::Missing(name.to_owned()))?;
        self.pos += 1;
        Ok(value)
    }

    fn next_with<T>(
        &mut self,
        name: &str,
        expected: &'static str,
        parse: impl Fn(&str) -> Option<T>,
    ) -> Result<T, ArgError> {
        let raw = self
            .peek()
            .ok_or_else(|| ArgError::Missing(name.to_owned()))?;
        let value = parse(raw).ok_or_else(|| ArgError::Invalid {
            name: name.to_owned(),
            value: raw.to_owned(),
            expected,
        })?;
        self.pos += 1;
        Ok(value)
    }

    /// Like the `next_*` parsers, but leaves the cursor in place when the
    /// argument is absent or does not parse.
    pub fn optional<T>(&mut self, parse: impl Fn(&str) -> Option<T>) -> Option<T> {
        let value = parse(self.peek()?)?;
        self.pos += 1;
        Some(value)
    }

    pub fn next_user(&mut self, name: &str) -> Result<Snowflake, ArgError> {
        self.next_with(name, "a user mention or ID", parse_user)
    }

    pub fn next_role(&mut self, name: &str) -> Result<Snowflake, ArgError> {
        self.next_with(name, "a role mention or ID", parse_role)
    }

    pub fn next_channel(&mut self, name: &str) -> Result<Snowflake, ArgError> {
        self.next_with(name, "a channel mention or ID", parse_channel)
    }

    pub fn next_int(&mut self, name: &str) -> Result<i64, ArgError> {
        self.next_with(name, "an integer", |raw| raw.trim().parse().ok())
    }

    pub fn next_bool(&mut self, name: &str) -> Result<bool, ArgError> {
        self.next_with(name, "yes or no", parse_bool)
    }

    pub fn next_duration(&mut self, name: &str) -> Result<Duration, ArgError> {
        self.next_with(name, "a duration such as 10m or 1h30m", parse_duration)
    }

    /// Joins everything left with single spaces; `None` when nothing is left.
    pub fn rest(&mut self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let joined = self.items[self.pos..].join(" ");
        self.pos = self.items.len();
        Some(joined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_and_ids() {
        assert_eq!(parse_user("<@!42>"), Some(Snowflake(42)));
        assert_eq!(parse_user("42"), Some(Snowflake(42)));
        assert_eq!(parse_role("<@&7>"), Some(Snowflake(7)));
        assert_eq!(parse_channel("<#9>"), Some(Snowflake(9)));
        assert_eq!(parse_role("<@7>"), None);
        assert_eq!(parse_user("abc"), None);
        assert_eq!(parse_user("007"), None);
        assert_eq!(parse_user("<@18446744073709551615>"), None);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("90"), Some(Duration::from_secs(90)));
        assert_eq!(parse_duration("1h30m"), Some(Duration::from_secs(5400)));
        assert_eq!(parse_duration("2D"), Some(Duration::from_secs(172_800)));
        assert_eq!(parse_duration("10x"), None);
        assert_eq!(parse_duration("m"), None);
        assert_eq!(parse_duration("1h5"), None);
    }

    #[test]
    fn cursor_reports_missing_and_invalid() {
        let mut args = Args::new(["<@1>", "10m", "spam", "links"]);
        assert_eq!(args.next_user("user"), Ok(Snowflake(1)));
        assert_eq!(args.optional(parse_bool), None);
        assert_eq!(args.next_duration("time"), Ok(Duration::from_secs(600)));
        assert!(matches!(args.next_int("n"), Err(ArgError::Invalid { .. })));
        assert_eq!(args.rest().as_deref(), Some("spam links"));
        assert_eq!(args.next_str("x"), Err(ArgError::Missing("x".into())));
    }
}
