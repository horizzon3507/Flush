//! `$sortLeft`/`$sortRight`/`$arraySortWith` (yuzuha `builtins/language/sort.ts`).

use serde_json::Value;

use super::common::{arg_value, require_array};
use super::loops::with_loop_state;
use super::state::LOOP_MAX;
use crate::errors::{FnError, ScriptFailure};
use crate::types::Definitions;
use crate::values::text;

const LEFT: &str = "__sortLeft";
const RIGHT: &str = "__sortRight";

pub fn register(d: &mut Definitions) {
    for (name, key, description) in [
        (
            "sortLeft",
            LEFT,
            "Left item in the $arraySortWith comparator; empty outside it.",
        ),
        (
            "sortRight",
            RIGHT,
            "Right item in the $arraySortWith comparator; empty outside it.",
        ),
    ] {
        d.add(name, "language", 0, 0, "", description, move |c| {
            Box::pin(async move {
                Ok(match c.ctx.var_get(key) {
                    Some(value) => value,
                    None => Value::String(String::new()),
                })
            })
        });
    }

    d.add(
        "arraySortWith",
        "language",
        2,
        2,
        "[array;comparator]",
        "Stable sort with a numeric negative/zero/positive expression; maximum 1000 items and 10000 comparisons.",
        |c| {
            Box::pin(async move {
                let input = require_array(arg_value(&c.arg(0).await?))?;
                if input.len() > LOOP_MAX {
                    return Err(FnError::new(
                        "EXECUTION_LIMIT",
                        format!("Array exceeds the limit of {LOOP_MAX} items."),
                    )
                    .into());
                }
                let saved: Vec<(String, Value)> = [LEFT, RIGHT]
                    .iter()
                    .filter_map(|key| c.ctx.var_get(key).map(|v| (key.to_string(), v)))
                    .collect();
                let result = with_loop_state(c.ctx, async move {
                    let mut values = input.clone();
                    let mut comparisons = 0usize;
                    let inner: Result<Vec<Value>, ScriptFailure> = async {
                        let mut width = 1usize;
                        while width < values.len() {
                            let mut merged: Vec<Value> = Vec::new();
                            let mut start = 0usize;
                            while start < values.len() {
                                let middle = (start + width).min(values.len());
                                let end = (start + 2 * width).min(values.len());
                                let mut left = start;
                                let mut right = middle;
                                while left < middle && right < end {
                                    comparisons += 1;
                                    if comparisons > 10_000 {
                                        return Err(FnError::new(
                                            "EXECUTION_LIMIT",
                                            "Comparison limit exceeded.",
                                        )
                                        .into());
                                    }
                                    c.ctx.var_set(LEFT, values[left].clone());
                                    c.ctx.var_set(RIGHT, values[right].clone());
                                    let raw = c.fresh(1).await?;
                                    let order: f64 = if raw.trim().is_empty() {
                                        f64::NAN
                                    } else {
                                        raw.trim().parse::<f64>().unwrap_or(f64::NAN)
                                    };
                                    if !order.is_finite() {
                                        return Err(FnError::new(
                                            "SORT_COMPARATOR",
                                            "Comparator must return a finite number.",
                                        )
                                        .into());
                                    }
                                    if order <= 0.0 {
                                        merged.push(values[left].clone());
                                        left += 1;
                                    } else {
                                        merged.push(values[right].clone());
                                        right += 1;
                                    }
                                }
                                merged.extend(values[left..middle].iter().cloned());
                                merged.extend(values[right..end].iter().cloned());
                                start += 2 * width;
                            }
                            values = merged;
                            width *= 2;
                        }
                        Ok(values)
                    }
                    .await;
                    for key in [LEFT, RIGHT] {
                        match saved.iter().find(|(k, _)| k == key) {
                            Some((_, value)) => c.ctx.var_set(key, value.clone()),
                            None => c.ctx.var_delete(key),
                        }
                    }
                    let values = inner?;
                    c.count_output(&text(&Value::Array(values.clone())))?;
                    Ok(Value::Array(values))
                })
                .await;
                result
            })
        },
    );
}
