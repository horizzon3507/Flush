//! Serde helpers tolerant of the Fluxer wire format.
//!
//! Gateway and REST send `null` for empty lists and integers for enums.
//! These helpers turn `null` into the default instead of failing.

use serde::{Deserialize, Deserializer};

/// `Vec<T>` that accepts `null` as empty.
/// Usage: `#[serde(default, deserialize_with = "crate::wire::null_vec")]`.
pub fn null_vec<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let opt: Option<Vec<T>> = Option::deserialize(d)?;
    Ok(opt.unwrap_or_default())
}

/// `Option<T>` that accepts `null` as `None` (explicit use avoids
/// ambiguity with the plain `Option::deserialize` on some types).
pub fn null_to_none<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(d)
}

/// `bool` that accepts `null` as `false`.
pub fn null_bool<'de, D>(d: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<bool> = Option::deserialize(d)?;
    Ok(opt.unwrap_or(false))
}

/// `u64` that accepts `null` as `0`.
pub fn null_u64<'de, D>(d: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<u64> = Option::deserialize(d)?;
    Ok(opt.unwrap_or(0))
}

/// `u32` that accepts `null` as `0`.
pub fn null_u32<'de, D>(d: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<u32> = Option::deserialize(d)?;
    Ok(opt.unwrap_or(0))
}

/// `i32` that accepts `null` as `0`.
pub fn null_i32<'de, D>(d: D) -> Result<i32, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<i32> = Option::deserialize(d)?;
    Ok(opt.unwrap_or(0))
}

/// `i64` that accepts `null` as `0`.
pub fn null_i64<'de, D>(d: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<i64> = Option::deserialize(d)?;
    Ok(opt.unwrap_or(0))
}
