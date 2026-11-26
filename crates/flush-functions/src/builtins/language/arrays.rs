//! `$array*` functions (yuzuha `builtins/language/arrays.ts`).

use std::cmp::Ordering;

use serde_json::Value;

use super::common::{integer, json_clone, require_array};
use crate::errors::FnError;
use crate::types::Definitions;
use crate::values::{deep_equal, text, typed_scalar};

fn compare_values(a: &Value, b: &Value) -> Ordering {
    if let (Value::Number(x), Value::Number(y)) = (a, b) {
        return x
            .as_f64()
            .partial_cmp(&y.as_f64())
            .unwrap_or(Ordering::Equal);
    }
    text(a).cmp(&text(b))
}

pub fn register(d: &mut Definitions) {
    d.add(
        "array",
        "language",
        0,
        64,
        "[value;...]",
        "Builds an array with the given values.",
        |c| {
            Box::pin(async move {
                let mut result = Vec::new();
                for i in 0..c.arg_count() {
                    result.push(typed_scalar(&json_clone(&c.value_arg(i).await?)));
                }
                Ok(Value::Array(result))
            })
        },
    );

    d.add(
        "arrayPush",
        "language",
        2,
        64,
        "[array;value;...]",
        "Returns a new array with the values appended.",
        |c| {
            Box::pin(async move {
                let mut result = require_array(c.value_arg(0).await?)?;
                for i in 1..c.arg_count() {
                    result.push(typed_scalar(&json_clone(&c.value_arg(i).await?)));
                }
                Ok(Value::Array(result))
            })
        },
    );

    d.add(
        "arrayGet",
        "language",
        2,
        3,
        "[array;index;default?]",
        "Gets an item by index; returns the default when absent.",
        |c| {
            Box::pin(async move {
                let base = require_array(c.value_arg(0).await?)?;
                let index = integer(&c.arg(1).await?, "Index")?;
                if index < 0 || index as usize >= base.len() {
                    return Ok(match c.arg_count() > 2 {
                        true => c.value_arg(2).await?,
                        false => Value::Null,
                    });
                }
                Ok(base[index as usize].clone())
            })
        },
    );

    d.add(
        "arrayLength",
        "language",
        1,
        1,
        "[array]",
        "Item count of the array.",
        |c| {
            Box::pin(async move {
                Ok(Value::from(
                    require_array(c.value_arg(0).await?)?.len() as u64
                ))
            })
        },
    );

    d.add(
        "arraySlice",
        "language",
        2,
        3,
        "[array;start;end?]",
        "Slice of the array; end exclusive.",
        |c| {
            Box::pin(async move {
                let base = require_array(c.value_arg(0).await?)?;
                let start = integer(&c.arg(1).await?, "Start")?;
                let end = if c.arg_count() > 2 {
                    integer(&c.arg(2).await?, "End")?
                } else {
                    base.len() as i64
                };
                if start < 0 || start as usize > base.len() {
                    return Err(FnError::new("INDEX", "Start out of range.").into());
                }
                if end < start || end as usize > base.len() {
                    return Err(FnError::new("INDEX", "End out of range.").into());
                }
                Ok(Value::Array(base[start as usize..end as usize].to_vec()))
            })
        },
    );

    d.add(
        "arrayIncludes",
        "language",
        2,
        2,
        "[array;value]",
        "Checks whether the array contains the value.",
        |c| {
            Box::pin(async move {
                let base = require_array(c.value_arg(0).await?)?;
                let target = typed_scalar(&json_clone(&c.value_arg(1).await?));
                Ok(Value::Bool(
                    base.iter().any(|item| deep_equal(item, &target)),
                ))
            })
        },
    );

    d.add(
        "arrayIndexOf",
        "language",
        2,
        2,
        "[array;value]",
        "Position of the first occurrence, or -1.",
        |c| {
            Box::pin(async move {
                let base = require_array(c.value_arg(0).await?)?;
                let target = typed_scalar(&json_clone(&c.value_arg(1).await?));
                let index = base.iter().position(|item| deep_equal(item, &target));
                Ok(match index {
                    Some(i) => Value::from(i as u64),
                    None => Value::from(-1_i64),
                })
            })
        },
    );

    d.add(
        "arrayJoin",
        "language",
        2,
        2,
        "[array;separator]",
        "Joins items converted to text.",
        |c| {
            Box::pin(async move {
                let base = require_array(c.value_arg(0).await?)?;
                let separator = c.arg(1).await?;
                Ok(Value::String(
                    base.iter().map(text).collect::<Vec<_>>().join(&separator),
                ))
            })
        },
    );

    d.add(
        "arraySort",
        "language",
        1,
        2,
        "[array;direction?]",
        "Sorts a copy of the array; direction asc or desc.",
        |c| {
            Box::pin(async move {
                let base = require_array(c.value_arg(0).await?)?;
                let direction = if c.arg_count() > 1 {
                    c.arg(1).await?.trim().to_lowercase()
                } else {
                    "asc".to_string()
                };
                if direction != "asc" && direction != "desc" {
                    return Err(FnError::new("SORT", "Direction must be asc or desc.").into());
                }
                let mut result = base.clone();
                result.sort_by(compare_values);
                if direction == "desc" {
                    result.reverse();
                }
                Ok(Value::Array(result))
            })
        },
    );

    d.add(
        "arrayReverse",
        "language",
        1,
        1,
        "[array]",
        "Returns a reversed copy of the array.",
        |c| {
            Box::pin(async move {
                let mut result = require_array(c.value_arg(0).await?)?;
                result.reverse();
                Ok(Value::Array(result))
            })
        },
    );

    d.add(
        "arrayUnique",
        "language",
        1,
        1,
        "[array]",
        "Removes duplicates, keeping the first occurrence.",
        |c| {
            Box::pin(async move {
                let base = require_array(c.value_arg(0).await?)?;
                let mut result: Vec<Value> = Vec::new();
                for item in base {
                    if result.iter().any(|known| deep_equal(known, &item)) {
                        continue;
                    }
                    result.push(item);
                }
                Ok(Value::Array(result))
            })
        },
    );

    for (name, description) in [
        (
            "arrayUnion",
            "Union without duplicates; preserves first-occurrence order.",
        ),
        (
            "arrayIntersection",
            "Unique items of the first array that are present in the second.",
        ),
        (
            "arrayDifference",
            "Unique items of the first array that are absent from the second.",
        ),
    ] {
        d.add(
            name,
            "language",
            2,
            2,
            "[array;otherArray]",
            format!("{description} Maximum of 1000 items combined.").leak(),
            move |c| {
                Box::pin(async move {
                    let left = require_array(c.value_arg(0).await?)?;
                    let right = require_array(c.value_arg(1).await?)?;
                    if left.len() + right.len() > 1000 {
                        return Err(FnError::new(
                            "ARRAY_SET",
                            "Set operations accept at most 1000 items combined.",
                        )
                        .into());
                    }
                    let union = name == "arrayUnion";
                    let difference = name == "arrayDifference";
                    let source: Vec<Value> = if union {
                        left.iter().chain(right.iter()).cloned().collect()
                    } else {
                        left.clone()
                    };
                    let mut result: Vec<Value> = Vec::new();
                    for item in source {
                        if result.iter().any(|known| deep_equal(known, &item)) {
                            continue;
                        }
                        let contains = union || right.iter().any(|known| deep_equal(known, &item));
                        if if difference { !contains } else { contains } {
                            result.push(item);
                        }
                    }
                    Ok(Value::Array(result))
                })
            },
        );
    }

    d.add(
        "range",
        "language",
        2,
        3,
        "[start;end;step?]",
        "Number sequence; at most 1000 items.",
        |c| {
            Box::pin(async move {
                let start = integer(&c.arg(0).await?, "Start")? as i128;
                let end = integer(&c.arg(1).await?, "End")? as i128;
                let step = if c.arg_count() > 2 {
                    integer(&c.arg(2).await?, "Step")? as i128
                } else if start <= end {
                    1
                } else {
                    -1
                };
                if step == 0 {
                    return Err(FnError::new("RANGE", "Step cannot be zero.").into());
                }
                if (step > 0 && start > end) || (step < 0 && start < end) {
                    return Err(FnError::new("RANGE", "Empty interval for the given step.").into());
                }
                let size = (end - start) / step + 1;
                if size > 1000 {
                    return Err(FnError::new("RANGE", "Interval exceeds 1000 items.").into());
                }
                Ok(Value::Array(
                    (0..size)
                        .map(|i| Value::from((start + i * step) as i64))
                        .collect(),
                ))
            })
        },
    );

    d.add(
        "paginate",
        "language",
        3,
        3,
        "[array;page;size]",
        "1-based page of the array.",
        |c| {
            Box::pin(async move {
                let base = require_array(c.value_arg(0).await?)?;
                let page = integer(&c.arg(1).await?, "Page")?;
                let size = integer(&c.arg(2).await?, "Size")?;
                if page < 1 {
                    return Err(FnError::new("PAGINATE", "Page starts at 1.").into());
                }
                if size < 1 {
                    return Err(FnError::new("PAGINATE", "Size must be at least 1.").into());
                }
                let start = ((page - 1) * size) as usize;
                let end = (page * size) as usize;
                Ok(Value::Array(
                    base.iter()
                        .skip(start)
                        .take(end.saturating_sub(start))
                        .cloned()
                        .collect(),
                ))
            })
        },
    );

    d.add(
        "arrayConcat",
        "language",
        1,
        64,
        "[array;...]",
        "Concatenates arrays without altering the originals.",
        |c| {
            Box::pin(async move {
                let mut result: Vec<Value> = Vec::new();
                for i in 0..c.arg_count() {
                    result.extend(require_array(c.value_arg(i).await?)?);
                    c.enforce_output_limit(&text(&Value::Array(result.clone())))?;
                }
                Ok(Value::Array(result))
            })
        },
    );

    d.add(
        "arrayFlat",
        "language",
        1,
        2,
        "[array;depth?]",
        "Flattens arrays; integer depth between 0 and 64.",
        |c| {
            Box::pin(async move {
                let items = require_array(c.value_arg(0).await?)?;
                let depth = if c.arg_count() > 1 {
                    integer(&c.arg(1).await?, "Depth")?
                } else {
                    1
                };
                if !(0..=64).contains(&depth) {
                    return Err(
                        FnError::new("ARRAY_DEPTH", "Depth must be between 0 and 64.").into(),
                    );
                }
                fn flat(items: Vec<Value>, depth: i64) -> Vec<Value> {
                    let mut out = Vec::new();
                    for item in items {
                        match item {
                            Value::Array(inner) if depth > 0 => out.extend(flat(inner, depth - 1)),
                            other => out.push(other),
                        }
                    }
                    out
                }
                Ok(Value::Array(flat(items, depth)))
            })
        },
    );

    d.add(
        "arrayChunk",
        "language",
        2,
        2,
        "[array;size]",
        "Splits the array into groups of the given size.",
        |c| {
            Box::pin(async move {
                let items = require_array(c.value_arg(0).await?)?;
                let size = integer(&c.arg(1).await?, "Size")?;
                if size < 1 {
                    return Err(FnError::new("ARRAY_CHUNK", "Size must be at least 1.").into());
                }
                Ok(Value::Array(
                    items
                        .chunks(size as usize)
                        .map(|chunk| Value::Array(chunk.to_vec()))
                        .collect(),
                ))
            })
        },
    );
}
