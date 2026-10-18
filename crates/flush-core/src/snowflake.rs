use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

use crate::error::CoreError;

/// Epoch do Fluxer em ms (2015-01-01T00:00:00Z).
pub const FLUXER_EPOCH: u64 = 1_420_070_400_000;

/// Snowflake do Fluxer: u64 serializado como string decimal no wire.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Snowflake(pub u64);

impl Snowflake {
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    pub fn get(self) -> u64 {
        self.0
    }

    /// Timestamp de criação em ms Unix: `(id >> 22) + FLUXER_EPOCH`.
    pub fn timestamp_ms(self) -> u64 {
        (self.0 >> 22) + FLUXER_EPOCH
    }

    pub fn worker_id(self) -> u16 {
        ((self.0 >> 12) & 0x3FF) as u16
    }

    pub fn sequence(self) -> u16 {
        (self.0 & 0xFFF) as u16
    }

    /// Menor snowflake possível para um timestamp (fronteira de paginação).
    pub fn from_timestamp_ms(ts: u64) -> Self {
        Self(ts.saturating_sub(FLUXER_EPOCH) << 22)
    }

    pub fn parse_opt(raw: Option<&str>) -> Option<Self> {
        raw?.parse().ok()
    }
}

impl fmt::Display for Snowflake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Snowflake {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        if t.is_empty() {
            return Err(CoreError::InvalidSnowflake(s.to_owned()));
        }
        // Wire aceita string decimal; HTTP também aceita inteiro JSON (ver Deserialize).
        if t.len() > 1 && t.starts_with('0') {
            return Err(CoreError::InvalidSnowflake(s.to_owned()));
        }
        if t.starts_with('+') || t.starts_with('-') || t.contains('.') {
            return Err(CoreError::InvalidSnowflake(s.to_owned()));
        }
        if !t.bytes().all(|b| b.is_ascii_digit()) {
            return Err(CoreError::InvalidSnowflake(s.to_owned()));
        }
        let v: u64 = t
            .parse()
            .map_err(|_| CoreError::InvalidSnowflake(s.to_owned()))?;
        if v > i64::MAX as u64 {
            return Err(CoreError::SnowflakeOutOfRange(s.to_owned()));
        }
        Ok(Self(v))
    }
}

impl From<u64> for Snowflake {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

impl From<Snowflake> for u64 {
    fn from(s: Snowflake) -> u64 {
        s.0
    }
}

impl From<Snowflake> for String {
    fn from(s: Snowflake) -> String {
        s.to_string()
    }
}

impl Serialize for Snowflake {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Snowflake {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{self, Visitor};
        struct V;
        impl Visitor<'_> for V {
            type Value = Snowflake;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("snowflake como string decimal ou inteiro")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Snowflake, E> {
                v.parse().map_err(E::custom)
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Snowflake, E> {
                v.parse().map_err(E::custom)
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Snowflake, E> {
                if v > i64::MAX as u64 {
                    return Err(E::custom("SNOWFLAKE_OUT_OF_RANGE"));
                }
                Ok(Snowflake(v))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Snowflake, E> {
                if v < 0 {
                    return Err(E::custom("INVALID_SNOWFLAKE_FORMAT"));
                }
                self.visit_u64(v as u64)
            }
        }
        d.deserialize_any(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_roundtrip() {
        let ts = 1_786_000_000_000u64;
        let sf = Snowflake::from_timestamp_ms(ts);
        assert_eq!(sf.timestamp_ms(), ts);
    }

    #[test]
    fn rejects_leading_zero_and_sign() {
        assert!("0123".parse::<Snowflake>().is_err());
        assert!("-5".parse::<Snowflake>().is_err());
        assert!("".parse::<Snowflake>().is_err());
    }

    #[test]
    fn wire_string_roundtrip() {
        let sf = Snowflake(1501320000000000000);
        let json = serde_json::to_string(&sf).unwrap();
        assert_eq!(json, "\"1501320000000000000\"");
        let back: Snowflake = serde_json::from_str(&json).unwrap();
        assert_eq!(back, sf);
        let from_int: Snowflake = serde_json::from_str("1501320000000000000").unwrap();
        assert_eq!(from_int, sf);
    }
}
