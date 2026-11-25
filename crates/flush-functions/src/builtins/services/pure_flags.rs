//! Port of `src/functions/builtins/pure-flags.ts`.

use serde_json::{json, Map, Value};

use crate::errors::{codes, FnError, ScriptFailure};
use crate::types::Definitions;
use crate::values::text;

use super::{sdk_data, support};

const MAX_BITS: u128 = u128::MAX;

fn flags_error(message: impl Into<String>) -> ScriptFailure {
    FnError::new(codes::SDK, message.into()).into()
}

fn table(name: &str) -> Result<std::collections::HashMap<String, u128>, ScriptFailure> {
    sdk_data::flag_tables()
        .get(&name.to_lowercase())
        .cloned()
        .ok_or_else(|| flags_error(format!("Unknown flag table '{name}'.")))
}

/// `resolveFlagInput(input, bits)`: names, decimals or arrays
/// (depth <= 4, <= 128 items).
fn resolve(
    bits: &std::collections::HashMap<String, u128>,
    value: &Value,
) -> Result<u128, ScriptFailure> {
    resolve_inner(bits, value, 0, &mut 0usize)
}

fn resolve_inner(
    bits: &std::collections::HashMap<String, u128>,
    value: &Value,
    depth: usize,
    count: &mut usize,
) -> Result<u128, ScriptFailure> {
    *count += 1;
    if *count > 128 {
        return Err(flags_error("A flag array takes at most 128 items."));
    }
    match value {
        Value::Array(items) => {
            if depth >= 4 {
                return Err(flags_error("Flag arrays nest at most 4 deep."));
            }
            let mut acc = 0u128;
            for item in items {
                acc |= resolve_inner(bits, item, depth + 1, count)?;
            }
            Ok(acc)
        }
        Value::Number(n) => {
            let n = n.as_u64().or_else(|| {
                n.as_f64()
                    .filter(|f| *f >= 0.0 && f.fract() == 0.0)
                    .map(|f| f as u64)
            });
            match n {
                Some(v) => Ok(v as u128),
                None => Err(flags_error(format!(
                    "Invalid flag number '{0}'.",
                    text(value)
                ))),
            }
        }
        Value::String(s) => {
            let name = s.trim();
            if let Some(bit) = bits.get(name) {
                return Ok(*bit);
            }
            if name.chars().all(|c| c.is_ascii_digit()) && !name.is_empty() {
                return name
                    .parse::<u128>()
                    .map_err(|_| flags_error(format!("Flag number '{name}' exceeds 128 bits.")));
            }
            Err(flags_error(format!("Unknown flag '{name}'.")))
        }
        _ => Err(flags_error(format!(
            "Invalid flag value '{0}'.",
            text(value)
        ))),
    }
}

/// `Administrator` implies every bit of the `permissions` table.
fn implies_all(name: &str, mask: u128, table: &std::collections::HashMap<String, u128>) -> u128 {
    if name.eq_ignore_ascii_case("permissions")
        && mask & table.get("Administrator").copied().unwrap_or(0) != 0
    {
        return MAX_BITS;
    }
    mask
}

fn names_of(table: &std::collections::HashMap<String, u128>, mask: u128) -> Vec<Value> {
    let mut names: Vec<_> = table
        .iter()
        .filter(|(_, bit)| **bit != 0 && mask & *bit != 0)
        .map(|(name, _)| Value::String(name.clone()))
        .collect();
    names.sort_by_key(text);
    names
}

pub fn register(d: &mut Definitions) {
    d.add(
        "sdkConstant",
        "util",
        1,
        2,
        "[família;nome?]",
        "Reads an @fluxerjs constant family (or a single entry).",
        |c| {
            Box::pin(async move {
                let family = c.arg(0).await?;
                let family_value = sdk_data::CONSTANTS
                    .get(&family)
                    .cloned()
                    .ok_or_else(|| flags_error(format!("Unknown constant family '{family}'.")))?;
                if c.arg_count() == 1 {
                    return Ok(family_value);
                }
                let name = c.arg(1).await?;
                match &family_value {
                    Value::Object(map) => Ok(map.get(&name).cloned().unwrap_or(Value::Null)),
                    _ => Ok(family_value),
                }
            })
        },
    );

    d.add(
        "flags",
        "util",
        2,
        4,
        "[família;operação;valor?;valor2?]",
        "Bitwise flag helpers over an @fluxerjs flag table.",
        |c| {
            Box::pin(async move {
                let family = c.arg(0).await?;
                let table = table(&family)?;
                let operation = c.arg(1).await?;

                match operation.as_str() {
                    "table" => Ok(json!(table
                        .iter()
                        .map(|(k, v)| {
                            (
                                k.clone(),
                                if *v <= u64::MAX as u128 {
                                    json!(*v as u64)
                                } else {
                                    json!(v.to_string())
                                },
                            )
                        })
                        .collect::<Map<_, _>>())),
                    "resolve" => {
                        let value = support::json_arg(&c.arg(2).await?);
                        let mask = resolve(&table, &value)?;
                        Ok(json!(mask.to_string()))
                    }
                    "names" => {
                        let value = support::json_arg(&c.arg(2).await?);
                        let mask = implies_all(&family, resolve(&table, &value)?, &table);
                        Ok(Value::Array(names_of(&table, mask)))
                    }
                    "serialize" => {
                        let value = support::json_arg(&c.arg(2).await?);
                        Ok(json!(resolve(&table, &value)?.to_string()))
                    }
                    "add" | "remove" => {
                        let base = resolve(&table, &support::json_arg(&c.arg(2).await?))?;
                        let flag = resolve(&table, &support::json_arg(&c.arg(3).await?))?;
                        let mask = if operation == "add" {
                            base | flag
                        } else {
                            base & !flag
                        };
                        Ok(json!(mask.to_string()))
                    }
                    "equals" | "has" | "any" | "missing" => {
                        let base = resolve(&table, &support::json_arg(&c.arg(2).await?))?;
                        let flag = resolve(&table, &support::json_arg(&c.arg(3).await?))?;
                        let base = implies_all(&family, base, &table);
                        let result = match operation.as_str() {
                            "equals" => base == flag,
                            "has" => base & flag == flag,
                            "any" => base & flag != 0,
                            _ => flag & !base != 0,
                        };
                        Ok(Value::Bool(result))
                    }
                    _ => Err(flags_error(format!(
                        "Unknown flags operation '{operation}'."
                    ))),
                }
            })
        },
    );
}
