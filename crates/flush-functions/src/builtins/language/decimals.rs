//! `$decimal*` builtins (yuzuha `builtins/language/decimals.ts`).

use serde_json::Value;

use super::common::integer;
use crate::types::Definitions;

pub fn register(d: &mut Definitions) {
    d.add(
        "decimal",
        "language",
        3,
        4,
        "[a;operator;b;places?]",
        "Exact decimal arithmetic in text, up to 1000 digits; division uses 18 places by default and ties round away from zero.",
        |c| {
            Box::pin(async move {
                let scale = if c.arg_count() > 3 {
                    Some(integer(&c.arg(3).await?, "Places")?)
                } else {
                    None
                };
                Ok(Value::String(super::decimal::decimal(
                    &c.arg(0).await?,
                    &c.arg(1).await?,
                    &c.arg(2).await?,
                    scale,
                )?))
            })
        },
    );

    d.add(
        "decimalCompare",
        "language",
        2,
        2,
        "[a;b]",
        "Compares decimals without converting to Number; returns -1, 0 or 1.",
        |c| {
            Box::pin(async move {
                Ok(Value::from(super::decimal::decimal_compare(
                    &c.arg(0).await?,
                    &c.arg(1).await?,
                )?))
            })
        },
    );

    d.add(
        "decimalRound",
        "language",
        2,
        2,
        "[value;places]",
        "Rounds a decimal in text; ties away from zero and trailing zeros are removed.",
        |c| {
            Box::pin(async move {
                Ok(Value::String(super::decimal::decimal_round(
                    &c.arg(0).await?,
                    integer(&c.arg(1).await?, "Places")?,
                )?))
            })
        },
    );
}
