//! Helpers serde tolerantes ao wire do Fluxer.
//!
//! O Gateway/REST mandam `null` em listas vazias e usam inteiros para enums.
//! Estes helpers convertem `null` em default em vez de falhar.

use serde::{Deserialize, Deserializer};

/// `Vec<T>` que aceita `null` como vazio.
/// Uso: `#[serde(default, deserialize_with = "crate::wire::null_vec")]`.
pub fn null_vec<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let opt: Option<Vec<T>> = Option::deserialize(d)?;
    Ok(opt.unwrap_or_default())
}

/// `Option<T>` que aceita `null` como `None` (uso explícito evita
/// ambiguidade com o `Option::deserialize` normal em alguns tipos).
pub fn null_to_none<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(d)
}

/// `bool` que aceita `null` como `false`.
pub fn null_bool<'de, D>(d: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<bool> = Option::deserialize(d)?;
    Ok(opt.unwrap_or(false))
}

/// `u64` que aceita `null` como `0`.
pub fn null_u64<'de, D>(d: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<u64> = Option::deserialize(d)?;
    Ok(opt.unwrap_or(0))
}

/// `u32` que aceita `null` como `0`.
pub fn null_u32<'de, D>(d: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<u32> = Option::deserialize(d)?;
    Ok(opt.unwrap_or(0))
}

/// `i32` que aceita `null` como `0`.
pub fn null_i32<'de, D>(d: D) -> Result<i32, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<i32> = Option::deserialize(d)?;
    Ok(opt.unwrap_or(0))
}

/// `i64` que aceita `null` como `0`.
pub fn null_i64<'de, D>(d: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<i64> = Option::deserialize(d)?;
    Ok(opt.unwrap_or(0))
}
