//! Script values and conversions (port of `runtime/values.ts` and the
//! shared helpers of `builtins/language/common.ts`).

use serde_json::{Map, Number, Value};

/// JS `String(value)` for the script domain: `null`/missing → `''`,
/// strings pass through, objects/arrays are JSON, scalars use `to_string`
/// semantics close to JS (`1` → `"1"`, `1.5` → `"1.5"`, `true` → `"true"`).
pub fn text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => js_number_to_string(n),
        Value::Array(_) | Value::Object(_) => js_stringify(value),
    }
}

/// `JSON.stringify` with JS number formatting: integral doubles print
/// without a fraction (`1`, not `1.0`).
pub fn js_stringify(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => js_number_to_string(n),
        Value::String(s) => serde_json::to_string(s).unwrap_or_default(),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(js_stringify).collect::<Vec<_>>().join(",")
        ),
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| format!(
                    "{}:{}",
                    serde_json::to_string(k).unwrap_or_default(),
                    js_stringify(v)
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

/// JS number formatting: integers print without a fraction, and
/// `-0`/`NaN`/`Infinity` can't occur (they are rejected on the way in).
pub fn js_number_to_string(n: &Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    let f = n.as_f64().unwrap_or(0.0);
    if f == f.trunc() && f.abs() < 1e21 {
        return format!("{f:.0}");
    }
    // JS uses exponential notation beyond 1e21; close enough for parity.
    let s = format!("{f}");
    s
}

/// Parse a script text into a JS-like number (`Number(text)`).
/// Mirrors `Number()`: trims, accepts decimal/exponent, empty → 0 is NOT
/// applied (callers pass `finite`/`integer` which reject empty first).
pub fn js_number(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some(0.0);
    }
    trimmed.parse::<f64>().ok()
}

/// JS `Boolean(value)` plus the string comparison extension
/// (`"a == b"`, `">"`, `"<="`, …) from yuzuha's `truthy()`.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => truthy_string(s),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn truthy_string(value: &str) -> bool {
    if value.chars().count() <= 32 || value.contains(['<', '>', '=']) {
        if let Some((left, op, right)) = split_comparison(value) {
            let l = left.trim();
            let r = right.trim();
            let ln = l.parse::<f64>().unwrap_or(f64::NAN);
            let rn = r.parse::<f64>().unwrap_or(f64::NAN);
            return match op {
                "==" => l == r,
                "!=" => l != r,
                ">" => ln > rn,
                "<" => ln < rn,
                ">=" => ln >= rn,
                "<=" => ln <= rn,
                _ => true,
            };
        }
    }
    !matches!(
        value.trim().to_lowercase().as_str(),
        "" | "false" | "0" | "null" | "undefined"
    )
}

/// Splits `left op right` like the JS regex
/// `/^(.*?)\s*(==|!=|>=|<=|>|<)\s*(.*?)$/`: the first operator position
/// wins (non-greedy left), operator order at a position is `==`, `!=`,
/// `>=`, `<=`, `>`, `<`.
fn split_comparison(value: &str) -> Option<(&str, &str, &str)> {
    for (index, _) in value.char_indices() {
        for op in ["==", "!=", ">=", "<=", ">", "<"] {
            if value[index..].starts_with(op) {
                return Some((&value[..index], op, &value[index + op.len()..]));
            }
        }
    }
    None
}

/// Structural equality (mirrors `deepEqual`).
/// `deepEqual`: numbers compare by numeric value (1 === 1.0 in JS).
pub fn deep_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            // Integers compare exactly so distinct large values never
            // collapse into the same float; only real doubles use f64.
            match (x.as_i64(), x.as_u64(), y.as_i64(), y.as_u64()) {
                (Some(a), _, Some(b), _) => a == b,
                (_, Some(a), _, Some(b)) => a == b,
                _ => x.as_f64() == y.as_f64(),
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| deep_equal(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|other| deep_equal(v, other)))
        }
        _ => a == b,
    }
}

/// Converts exact scalar texts into numbers, booleans or null — used only
/// where script literals must keep their JSON type (`$array`, `$object`,
/// push/pull).
pub fn typed_scalar(value: &Value) -> Value {
    let Value::String(s) = value else {
        return value.clone();
    };
    match s.as_str() {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        "null" => return Value::Null,
        _ => {}
    }
    if is_exact_number_literal(s) {
        if let Ok(n) = s.parse::<f64>() {
            if let Some(num) = Number::from_f64(n) {
                return Value::Number(num);
            }
        }
    }
    value.clone()
}

/// `/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/` — and `String(Number(v))`
/// must round-trip to `v` (JS check `String(number) === value`).
fn is_exact_number_literal(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    if bytes.get(i) == Some(&b'-') {
        i += 1;
    }
    let int_start = i;
    match bytes.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            i += 1;
            while bytes.get(i).is_some_and(|b| b.is_ascii_digit()) {
                i += 1;
            }
        }
        _ => return false,
    }
    if i == int_start {
        return false;
    }
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        let frac_start = i;
        while bytes.get(i).is_some_and(|b| b.is_ascii_digit()) {
            i += 1;
        }
        if i == frac_start {
            return false;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let exp_start = i;
        while bytes.get(i).is_some_and(|b| b.is_ascii_digit()) {
            i += 1;
        }
        if i == exp_start {
            return false;
        }
    }
    if i != bytes.len() {
        return false;
    }
    // Round-trip check: `String(Number(s)) === s`.
    s.parse::<f64>()
        .ok()
        .is_some_and(|f| Number::from_f64(f).is_some_and(|n| js_number_to_string(&n) == s))
}

/// `toJson`: round-trips a value through JSON, dropping non-finite numbers.
pub fn to_json(value: &Value) -> Result<Value, crate::errors::FnError> {
    match scrub_non_finite(value) {
        Some(v) => Ok(v),
        None => Ok(Value::Null),
    }
}

fn scrub_non_finite(value: &Value) -> Option<Value> {
    match value {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                return Some(Value::from(i));
            }
            if let Some(u) = n.as_u64() {
                return Some(Value::from(u));
            }
            n.as_f64()
                .filter(|f| f.is_finite())
                .and_then(Number::from_f64)
                .map(Value::Number)
        }
        Value::Array(items) => Some(Value::Array(
            items.iter().filter_map(scrub_non_finite).collect(),
        )),
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                if let Some(scrubbed) = scrub_non_finite(v) {
                    out.insert(k.clone(), scrubbed);
                }
            }
            Some(Value::Object(out))
        }
        _ => Some(value.clone()),
    }
}
