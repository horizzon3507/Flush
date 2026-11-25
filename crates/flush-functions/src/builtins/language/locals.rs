//! `$get`/`$let`/`$var`: the per-execution local map (`ctx.vars`).
//! Ported from yuzuha `builtins/language/values.ts` (locals part) —
//! this file doubles as the porting pattern reference.

use serde_json::Value;

use crate::types::Definitions;
use crate::values::{text, typed_scalar};

pub fn register(d: &mut Definitions) {
    d.add(
        "get",
        "language",
        1,
        usize::MAX,
        "[name;property?]",
        "Reads a local variable (or a property of it).",
        |c| {
            Box::pin(async move {
                let name = c.arg(0).await?;
                let mut value = match c.ctx.var_get(&name) {
                    Some(value) => value,
                    None => return Ok(Value::Null),
                };
                for i in 1..c.arg_count() {
                    let key = c.arg(i).await?;
                    let key_typed = typed_scalar(&Value::String(key.clone()));
                    value = match value {
                        Value::Object(ref map) => {
                            map.get(key.as_str()).cloned().unwrap_or(Value::Null)
                        }
                        Value::Array(ref list) => key_typed
                            .as_f64()
                            .filter(|f| f.fract() == 0.0 && *f >= 0.0)
                            .and_then(|i| list.get(i as usize))
                            .cloned()
                            .unwrap_or(Value::Null),
                        _ => return Ok(Value::Null),
                    };
                }
                Ok(value)
            })
        },
    );

    d.add(
        "let",
        "language",
        2,
        usize::MAX,
        "[name;value]",
        "Writes a local variable.",
        |c| {
            Box::pin(async move {
                let name = c.arg(0).await?;
                let value = c.arg(1).await?;
                c.ctx.var_set(name, typed_scalar(&Value::String(value)));
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "var",
        "language",
        1,
        usize::MAX,
        "[name;value?]",
        "Reads or writes a local variable.",
        |c| {
            Box::pin(async move {
                let name = c.arg(0).await?;
                if c.arg_count() > 1 {
                    let value = c.arg(1).await?;
                    c.ctx
                        .var_set(name.clone(), typed_scalar(&Value::String(value)));
                    return Ok(Value::Null);
                }
                match c.ctx.var_get(&name) {
                    Some(value) => Ok(Value::String(text(&value))),
                    None => Ok(Value::Null),
                }
            })
        },
    );
}
