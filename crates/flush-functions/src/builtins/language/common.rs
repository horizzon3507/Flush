//! Shared helpers of the language group (yuzuha `builtins/language/common.ts`).

use serde_json::{Number, Value};

use crate::errors::FnError;
use crate::values::to_json;

/// Largest `Number.isSafeInteger` magnitude.
const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

/// `jsonClone`: a script arg text back into a `Value` (objects/arrays keep
/// their JSON type; anything that is not JSON stays a string).
pub fn arg_value(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
}

/// `jsonClone` over a `Value` (non-finite numbers scrubbed), used where
/// yuzuha clones arg results before storing them.
pub fn json_clone(value: &Value) -> Value {
    to_json(value).unwrap_or(Value::Null)
}

/// `integer(raw, label)`: `Number(raw)` must be a safe integer.
pub fn integer(raw: &str, label: &str) -> Result<i64, FnError> {
    let value = crate::values::js_number(raw).unwrap_or(f64::NAN);
    if raw.trim().is_empty() || !value.is_finite() || value.fract() != 0.0 || value.abs() > MAX_SAFE
    {
        return Err(FnError::new(
            "INDEX",
            format!("{label} must be an integer."),
        ));
    }
    Ok(value as i64)
}

/// `finiteNumber(raw, label)`: `Number(raw)` must be finite.
pub fn finite_number(raw: &str, label: &str) -> Result<f64, FnError> {
    let value = crate::values::js_number(raw).unwrap_or(f64::NAN);
    if raw.trim().is_empty() || !value.is_finite() {
        return Err(FnError::new(
            "NUMBER",
            format!("{label} must be a valid number."),
        ));
    }
    Ok(value)
}

/// `requireArray(value)`: the value must be a JSON array.
pub fn require_array(value: Value) -> Result<Vec<Value>, FnError> {
    match value {
        Value::Array(items) => Ok(items),
        _ => Err(FnError::new("ARRAY", "Value must be an array.")),
    }
}

/// `f64` → JSON number, never non-finite (callers validate first).
pub fn number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() <= MAX_SAFE {
        return Value::Number(Number::from(value as i64));
    }
    Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}
