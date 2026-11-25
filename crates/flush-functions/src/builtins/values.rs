//! `values` category builtins (yuzuha `builtins/values.ts`).

use serde_json::Value;

use super::language::common::{arg_value, finite_number, number};
use crate::errors::FnError;
use crate::types::Definitions;
use crate::values::truthy;

pub fn register(d: &mut Definitions) {
    d.add(
        "prop",
        "values",
        2,
        2,
        "[object;property]",
        "Gets an object's own property.",
        |c| {
            Box::pin(async move {
                let value = arg_value(&c.arg(0).await?);
                let key = c.arg(1).await?;
                Ok(match value {
                    Value::Object(map) => map
                        .get(&key)
                        .cloned()
                        .unwrap_or(Value::String(String::new())),
                    Value::Array(items) => super::language::objects::is_index(&key)
                        .then(|| key.parse::<usize>().ok())
                        .flatten()
                        .and_then(|i| items.get(i))
                        .cloned()
                        .unwrap_or(Value::String(String::new())),
                    _ => Value::String(String::new()),
                })
            })
        },
    );

    for (name, description) in [
        ("eq", "Compares textual equality."),
        ("ne", "Compares textual difference."),
    ] {
        d.add(name, "values", 2, 2, "[a;b]", description, move |c| {
            Box::pin(async move {
                let equal = c.arg(0).await? == c.arg(1).await?;
                Ok(Value::Bool(if name == "eq" { equal } else { !equal }))
            })
        });
    }

    d.add(
        "not",
        "values",
        1,
        1,
        "[value]",
        "Inverts a condition.",
        |c| Box::pin(async move { Ok(Value::Bool(!truthy(&arg_value(&c.arg(0).await?)))) }),
    );

    for (name, description) in [
        ("and", "Logical AND with short-circuit evaluation."),
        ("or", "Logical OR with short-circuit evaluation."),
    ] {
        d.add(name, "values", 2, 32, "[a;b;...]", description, move |c| {
            Box::pin(async move {
                for i in 0..c.arg_count() {
                    let value = truthy(&arg_value(&c.arg(i).await?));
                    if name == "and" && !value {
                        return Ok(Value::Bool(false));
                    }
                    if name == "or" && value {
                        return Ok(Value::Bool(true));
                    }
                }
                Ok(Value::Bool(name == "and"))
            })
        });
    }

    d.add(
        "math",
        "values",
        3,
        3,
        "[a;operator;b]",
        "Calculates +, -, *, / or % without eval.",
        |c| {
            Box::pin(async move {
                let a = finite_number(&c.arg(0).await?, "Value")?;
                let op = c.arg(1).await?;
                let b = finite_number(&c.arg(2).await?, "Value")?;
                let result = match op.trim() {
                    "+" => a + b,
                    "-" => a - b,
                    "*" => a * b,
                    "/" => a / b,
                    "%" => a % b,
                    _ => f64::NAN,
                };
                if !result.is_finite() {
                    return Err(
                        FnError::new("MATH", "Invalid operation or non-finite result.").into(),
                    );
                }
                Ok(number(result))
            })
        },
    );

    d.add(
        "random",
        "values",
        2,
        2,
        "[min;max]",
        "Random integer with both bounds inclusive.",
        |c| {
            Box::pin(async move {
                let min = finite_number(&c.arg(0).await?, "Value")?;
                let max = finite_number(&c.arg(1).await?, "Value")?;
                const SAFE: f64 = 9_007_199_254_740_991.0;
                if min.fract() != 0.0
                    || max.fract() != 0.0
                    || min.abs() > SAFE
                    || max.abs() > SAFE
                    || max < min
                    || max - min >= 2f64.powi(48)
                    || max + 1.0 > SAFE
                {
                    return Err(FnError::new("RANDOM", "Invalid integer interval.").into());
                }
                let mut rng = rand::rng();
                Ok(Value::from(rand::Rng::random_range(
                    &mut rng,
                    min as i64..=max as i64,
                )))
            })
        },
    );

    for name in ["lowercase", "uppercase", "trim", "length"] {
        d.add(
            name,
            "values",
            1,
            1,
            "[text]",
            "Text transformation.",
            move |c| {
                Box::pin(async move {
                    let value = c.arg(0).await?;
                    Ok(match name {
                        "lowercase" => Value::String(value.to_lowercase()),
                        "uppercase" => Value::String(value.to_uppercase()),
                        "trim" => Value::String(value.trim().to_string()),
                        _ => Value::from(value.chars().count() as u64),
                    })
                })
            },
        );
    }

    d.add(
        "replace",
        "values",
        3,
        3,
        "[text;search;replacement]",
        "Replaces every literal occurrence.",
        |c| {
            Box::pin(async move {
                let value = c.arg(0).await?;
                let search = c.arg(1).await?;
                let replacement = c.arg(2).await?;
                if search.is_empty() {
                    return Err(FnError::new("REPLACE", "Empty search is not allowed.").into());
                }
                if value.len() * replacement.len().max(1) > 1_000_000 {
                    return Err(
                        FnError::new("REPLACE", "Potential result exceeds the limit.").into(),
                    );
                }
                Ok(Value::String(value.replace(&search, &replacement)))
            })
        },
    );

    d.add(
        "join",
        "values",
        2,
        32,
        "[separator;value;...]",
        "Joins values.",
        |c| {
            Box::pin(async move {
                let separator = c.arg(0).await?;
                let mut values = Vec::new();
                for i in 1..c.arg_count() {
                    values.push(c.arg(i).await?);
                }
                Ok(Value::String(values.join(&separator)))
            })
        },
    );

    d.add(
        "timestamp",
        "values",
        0,
        0,
        "",
        "Unix timestamp in milliseconds.",
        |c| Box::pin(async move { Ok(Value::from((c.ctx.now)())) }),
    );
}
