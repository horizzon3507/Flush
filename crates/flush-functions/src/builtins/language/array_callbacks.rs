//! `$arrayMap`/`Filter`/`Some`/`Every`/`Find`/`Reduce`/`SortBy`
//! (yuzuha `builtins/language/array-callbacks.ts`).

use serde_json::Value;

use super::common::{arg_value, require_array};
use super::loops::with_loop_state;
use super::state::{LOOP_ACCUMULATOR_KEY, LOOP_INDEX_KEY, LOOP_MAX, LOOP_VALUE_KEY};
use crate::call::ScriptCall;
use crate::errors::{FnError, ScriptFailure};
use crate::types::Definitions;
use crate::values::{text, truthy};

async fn items(c: &ScriptCall<'_>) -> Result<Vec<Value>, ScriptFailure> {
    let value = require_array(c.value_arg(0).await?)?;
    if value.len() > LOOP_MAX {
        return Err(FnError::new(
            "EXECUTION_LIMIT",
            format!("Array exceeds the limit of {LOOP_MAX} items."),
        )
        .into());
    }
    Ok(value)
}

fn iteration(c: &ScriptCall<'_>, values: &[Value], index: usize) {
    c.ctx.var_set(LOOP_INDEX_KEY, Value::from(index as u64));
    c.ctx.var_set(LOOP_VALUE_KEY, values[index].clone());
}

pub fn register(d: &mut Definitions) {
    for (name, description) in [
        (
            "arrayMap",
            "Returns an array with the expression result for each item.",
        ),
        (
            "arrayFilter",
            "Returns the items whose expression is truthy.",
        ),
        (
            "arraySome",
            "Returns true at the first item with a truthy expression; false when none.",
        ),
        (
            "arrayEvery",
            "Returns false at the first item with a falsy expression; true when all, including an empty array.",
        ),
        (
            "arrayFind",
            "Returns the first item with a truthy expression, or null when absent.",
        ),
    ] {
        let description: &'static str = description;
        d.add(
            name,
            "language",
            2,
            2,
            "[array;expression]",
            format!("{description} Uses $loopValue/$loopIndex; maximum 1000 items.").leak(),
            move |c| {
                Box::pin(async move {
                    let values = items(c).await?;
                    with_loop_state(c.ctx, async move {
                        let mut result: Vec<Value> = Vec::new();
                        for index in 0..values.len() {
                            iteration(c, &values, index);
                            let (rendered, typed) = c.fresh_typed(1).await?;
                            let value = typed.unwrap_or_else(|| arg_value(&rendered));
                            let matches = truthy(&value);
                            if name == "arraySome" && matches {
                                return Ok(Value::Bool(true));
                            }
                            if name == "arrayEvery" && !matches {
                                return Ok(Value::Bool(false));
                            }
                            if name == "arrayFind" && matches {
                                return Ok(values[index].clone());
                            }
                            if name == "arrayMap" {
                                result.push(value.clone());
                            }
                            if name == "arrayFilter" && matches {
                                result.push(values[index].clone());
                            }
                            c.enforce_output_limit(&text(&Value::Array(result.clone())))?;
                        }
                        if name == "arraySome" {
                            return Ok(Value::Bool(false));
                        }
                        if name == "arrayEvery" {
                            return Ok(Value::Bool(true));
                        }
                        if name == "arrayFind" {
                            return Ok(Value::Null);
                        }
                        Ok(Value::Array(result))
                    })
                    .await
                })
            },
        );
    }

    d.add(
        "arrayReduce",
        "language",
        3,
        3,
        "[array;initial;expression]",
        "Reduces up to 1000 items using $loopAccumulator, $loopValue and $loopIndex.",
        |c| {
            Box::pin(async move {
                let values = items(c).await?;
                let mut accumulator = c.value_arg(1).await?;
                with_loop_state(c.ctx, async move {
                    for index in 0..values.len() {
                        iteration(c, &values, index);
                        c.ctx.var_set(LOOP_ACCUMULATOR_KEY, accumulator.clone());
                        let (rendered, typed) = c.fresh_typed(2).await?;
                        accumulator = typed.unwrap_or_else(|| arg_value(&rendered));
                        c.enforce_output_limit(&text(&accumulator))?;
                    }
                    Ok(accumulator)
                })
                .await
            })
        },
    );

    d.add(
        "arraySortBy",
        "language",
        2,
        3,
        "[array;key;direction?]",
        "Sorts up to 1000 items by same-type keys, text or number; computed once per item. Asc/desc preserve ties.",
        |c| {
            Box::pin(async move {
                let values = items(c).await?;
                let direction = if c.arg_count() > 2 {
                    c.arg(2).await?.trim().to_lowercase()
                } else {
                    "asc".to_string()
                };
                if direction != "asc" && direction != "desc" {
                    return Err(FnError::new("SORT", "Direction must be asc or desc.").into());
                }
                with_loop_state(c.ctx, async move {
                    let mut keyed: Vec<(Value, Value)> = Vec::new();
                    for index in 0..values.len() {
                        iteration(c, &values, index);
                        let (rendered, typed) = c.fresh_typed(1).await?;
                        let key = typed.unwrap_or_else(|| arg_value(&rendered));
                        let valid = match &key {
                            Value::String(_) => true,
                            Value::Number(n) => n.as_f64().is_some_and(f64::is_finite),
                            _ => false,
                        };
                        if !valid {
                            return Err(FnError::new(
                                "SORT_KEY",
                                "Sort key must be text or a finite number.",
                            )
                            .into());
                        }
                        if let Some((_, first)) = keyed.first() {
                            let same = matches!(
                                (first, &key),
                                (Value::String(_), Value::String(_))
                                    | (Value::Number(_), Value::Number(_))
                            );
                            if !same {
                                return Err(FnError::new(
                                    "SORT_KEY",
                                    "Sort keys must have the same type.",
                                )
                                .into());
                            }
                        }
                        keyed.push((values[index].clone(), key));
                    }
                    let sign: f64 = if direction == "asc" { 1.0 } else { -1.0 };
                    keyed.sort_by(|(_, a), (_, b)| {
                        let order = if let (Value::Number(x), Value::Number(y)) = (a, b) {
                            x.as_f64()
                                .partial_cmp(&y.as_f64())
                                .unwrap_or(std::cmp::Ordering::Equal)
                        } else {
                            text(a).cmp(&text(b))
                        };
                        match sign {
                            s if s < 0.0 => order.reverse(),
                            _ => order,
                        }
                    });
                    Ok(Value::Array(keyed.into_iter().map(|(v, _)| v).collect()))
                })
                .await
            })
        },
    );
}
