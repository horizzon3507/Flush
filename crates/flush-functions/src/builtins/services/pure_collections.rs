//! Port of `src/functions/builtins/pure-collections.ts`.

use serde_json::{json, Value};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::errors::{codes, is_uncatchable, FnError, ScriptFailure};
use crate::types::Definitions;
use crate::values::{text, truthy};

use super::support;

const MAX_ENTRIES: usize = 1_000;

fn collection_error(message: impl Into<String>) -> ScriptFailure {
    FnError::new(codes::COLLECTION, message.into()).into()
}

/// Collection entries: unique `[key, value]` pairs, insertion ordered.
fn entry_list(value: &Value) -> Result<Vec<(Value, Value)>, ScriptFailure> {
    let items = match value {
        Value::Array(items) => items.clone(),
        _ => {
            return Err(collection_error(
                "Expected an array of [key, value] entries.",
            ))
        }
    };
    if items.len() > MAX_ENTRIES {
        return Err(collection_error(format!(
            "A collection takes at most {MAX_ENTRIES} entries."
        )));
    }
    let mut out: Vec<(Value, Value)> = Vec::new();
    for item in &items {
        let pair = item
            .as_array()
            .filter(|a| a.len() == 2)
            .ok_or_else(|| collection_error("Every entry must be a [key, value] pair."))?;
        let key = match &pair[0] {
            Value::String(_) | Value::Number(_) => pair[0].clone(),
            _ => {
                return Err(collection_error(
                    "Collection keys must be strings or integers.",
                ))
            }
        };
        if out.iter().any(|(k, _)| *k == key) {
            return Err(collection_error(format!(
                "Duplicate collection key '{0}'.",
                text(&key)
            )));
        }
        out.push((key, pair[1].clone()));
    }
    Ok(out)
}

static RNG_STATE: AtomicU64 = AtomicU64::new(0x9E37_79B9_7F4A_7C15);

fn rand_below(n: usize) -> usize {
    // SplitMix64 — deterministic-seeded, only needs uniform-ish draws.
    let mut z = RNG_STATE.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) as usize) % n.max(1)
}

fn describe(entries: &[(Value, Value)]) -> Value {
    json!(format!("Collection({})", entries.len()))
}

/// Loop state around a body call: clears the loop frame and snapshots the
/// `__loop*` vars (yuzuha's `withLoopState`).
async fn with_loop_state<R>(
    c: &crate::call::ScriptCall<'_>,
    index: usize,
    value: &Value,
    body: impl Future<Output = R>,
) -> R {
    let saved_index = c.ctx.var_get("__loopIndex");
    let saved_value = c.ctx.var_get("__loopValue");
    let frame = c.ctx.loop_frame();
    c.ctx.var_set("__loopIndex", Value::from(index as i64));
    c.ctx.var_set("__loopValue", value.clone());
    c.ctx.set_loop_frame(None);
    let out = body.await;
    c.ctx.set_loop_frame(frame);
    match saved_index {
        Some(v) => c.ctx.var_set("__loopIndex", v),
        None => c.ctx.var_delete("__loopIndex"),
    }
    match saved_value {
        Some(v) => c.ctx.var_set("__loopValue", v),
        None => c.ctx.var_delete("__loopValue"),
    }
    out
}

pub fn register(d: &mut Definitions) {
    d.add(
        "sdkCollection",
        "util",
        2,
        5,
        "[operação;entradas;valor?;valor2?;valor3?]",
        "Ordered map helpers over [key, value] entry arrays.",
        |c| {
            Box::pin(async move {
                let op = c.arg(0).await?;
                let entries = entry_list(&support::json_arg(&c.arg(1).await?))?;
                let arg = async |c: &crate::call::ScriptCall<'_>, i: usize| -> Result<Value, ScriptFailure> {
                    Ok(support::json_arg(&c.arg(i).await?))
                };
                match op.as_str() {
                    "clone" => Ok(json!(entries
                        .iter()
                        .map(|(k, v)| json!([k, v]))
                        .collect::<Vec<_>>())),
                    "concat" => {
                        let mut merged = entries;
                        for i in 2..c.arg_count() {
                            for (k, v) in entry_list(&arg(c, i).await?)? {
                                match merged.iter_mut().find(|(key, _)| *key == k) {
                                    Some(slot) => slot.1 = v,
                                    None => merged.push((k, v)),
                                }
                            }
                        }
                        Ok(json!(merged
                            .iter()
                            .map(|(k, v)| json!([k, v]))
                            .collect::<Vec<_>>()))
                    }
                    "first" => {
                        if c.arg_count() == 2 {
                            return Ok(entries.first().map(|(_, v)| v.clone()).unwrap_or(Value::Null));
                        }
                        let n = support::index(&arg(c, 2).await?, 0)?.min(entries.len());
                        Ok(Value::Array(entries[..n].iter().map(|(_, v)| v.clone()).collect()))
                    }
                    "last" => {
                        if c.arg_count() == 2 {
                            return Ok(entries.last().map(|(_, v)| v.clone()).unwrap_or(Value::Null));
                        }
                        let n = support::index(&arg(c, 2).await?, 0)?.min(entries.len());
                        Ok(Value::Array(entries[entries.len() - n..].iter().map(|(_, v)| v.clone()).collect()))
                    }
                    "random" => {
                        if entries.is_empty() {
                            return Ok(Value::Null);
                        }
                        if c.arg_count() == 2 {
                            return Ok(entries[rand_below(entries.len())].1.clone());
                        }
                        let n = support::index(&arg(c, 2).await?, 0)?.min(entries.len());
                        let mut values: Vec<Value> = entries.iter().map(|(_, v)| v.clone()).collect();
                        for i in 0..n {
                            let j = i + rand_below(values.len() - i);
                            values.swap(i, j);
                        }
                        Ok(Value::Array(values.into_iter().take(n).collect()))
                    }
                    "values" => {
                        Ok(Value::Array(entries.iter().map(|(_, v)| v.clone()).collect()))
                    }
                    "text" => {
                        let separator = if c.arg_count() > 2 { c.arg(2).await? } else { ", ".into() };
                        Ok(Value::String(
                            entries.iter().map(|(_, v)| text(v)).collect::<Vec<_>>().join(&separator),
                        ))
                    }
                    "describe" => Ok(describe(&entries)),
                    "set" => {
                        let key = arg(c, 2).await?;
                        let value = arg(c, 3).await?;
                        let mut entries = entries;
                        let mut evicted = Value::Null;
                        match &key {
                            Value::String(_) | Value::Number(_) => {}
                            _ => {
                                return Err(collection_error(
                                    "Collection keys must be strings or integers.",
                                ))
                            }
                        }
                        match entries.iter_mut().find(|(k, _)| *k == key) {
                            Some(slot) => slot.1 = value,
                            None => {
                                if entries.len() >= MAX_ENTRIES {
                                    let (k, _) = entries.remove(0);
                                    evicted = k;
                                }
                                entries.push((key, value));
                            }
                        }
                        Ok(json!({
                            "entries": entries.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
                            "evicted": evicted,
                        }))
                    }
                    "partition" | "sweep" => {
                        let mut pass: Vec<(Value, Value)> = Vec::new();
                        let mut fail: Vec<(Value, Value)> = Vec::new();
                        let mut removed = 0usize;
                        let mut kept: Vec<(Value, Value)> = Vec::new();
                        for (index, (key, value)) in entries.into_iter().enumerate() {
                            let result = with_loop_state(c, index, &value, async {
                                Box::pin(c.fresh(2)).await
                            })
                            .await;
                            let result = match result {
                                Ok(v) => v,
                                Err(e) if is_uncatchable(&e) => return Err(e),
                                Err(e) => return Err(e),
                            };
                            let keep = truthy(&Value::String(result));
                            if op == "partition" {
                                if keep {
                                    pass.push((key, value));
                                } else {
                                    fail.push((key, value));
                                }
                            } else if keep {
                                kept.push((key, value));
                            } else {
                                removed += 1;
                            }
                        }
                        if op == "partition" {
                            Ok(json!({
                                "pass": pass.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
                                "fail": fail.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
                            }))
                        } else {
                            Ok(json!({
                                "entries": kept.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
                                "removed": removed,
                            }))
                        }
                    }
                    _ => Err(collection_error(format!("Unknown sdkCollection operation '{op}'."))),
                }
            })
        },
    );
}
