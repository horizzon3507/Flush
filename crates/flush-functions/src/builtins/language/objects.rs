//! `$object`/`$json*`/`$hasProp` (yuzuha `builtins/language/objects.ts`),
//! plus the JSON-path helpers `$set*Path` reuse.

use serde_json::{Map, Value};

use crate::errors::{FnError, ScriptFailure};
use crate::types::Definitions;
use crate::values::{text, to_json, typed_scalar};

fn json_path_error(message: &str) -> ScriptFailure {
    FnError::new("JSON_PATH", message.to_string()).into()
}

fn array_index(segment: &str) -> Result<usize, ScriptFailure> {
    if !is_index(segment) {
        return Err(json_path_error(
            "JSON path index must be between 0 and 9999.",
        ));
    }
    let index: usize = segment
        .parse()
        .map_err(|_| json_path_error("JSON path index must be between 0 and 9999."))?;
    if index >= 10_000 {
        return Err(json_path_error(
            "JSON path index must be between 0 and 9999.",
        ));
    }
    Ok(index)
}

pub fn parse_path(path: &str) -> Result<Vec<String>, ScriptFailure> {
    if path.is_empty() {
        return Ok(Vec::new());
    }
    let segments: Vec<String> = path.split('.').map(str::to_string).collect();
    if segments.iter().any(|segment| segment.is_empty()) {
        return Err(json_path_error(
            "Invalid JSON path. Use dots and indices, e.g. \"a.0.b\".",
        ));
    }
    Ok(segments)
}

pub fn is_index(segment: &str) -> bool {
    !segment.is_empty() && segment.chars().all(|ch| ch.is_ascii_digit())
}

fn get_by_path<'a>(value: &'a Value, segments: &[String]) -> Option<&'a Value> {
    let mut current = value;
    for segment in segments {
        match current {
            Value::Array(items) => {
                if !is_index(segment) {
                    return None;
                }
                let index: usize = segment.parse().ok()?;
                current = items.get(index)?;
            }
            Value::Object(map) => current = map.get(segment)?,
            _ => return None,
        }
    }
    Some(current)
}

pub fn set_json_path(
    base: &Value,
    segments: &[String],
    fresh: &Value,
) -> Result<Value, ScriptFailure> {
    if segments.is_empty() {
        return Ok(to_json(fresh).unwrap_or(Value::Null));
    }
    let mut root = match base {
        Value::Array(_) | Value::Object(_) => base.clone(),
        _ => Value::Object(Map::new()),
    };
    let mut current = &mut root;
    for (i, segment) in segments[..segments.len() - 1].iter().enumerate() {
        let next_is_index = is_index(&segments[i + 1]);
        match current {
            Value::Array(items) => {
                if !is_index(segment) {
                    return Err(json_path_error("Invalid JSON path for array."));
                }
                let index = array_index(segment)?;
                while items.len() <= index {
                    items.push(if next_is_index {
                        Value::Array(Vec::new())
                    } else {
                        Value::Object(Map::new())
                    });
                }
                if !matches!(items[index], Value::Array(_) | Value::Object(_)) {
                    items[index] = if next_is_index {
                        Value::Array(Vec::new())
                    } else {
                        Value::Object(Map::new())
                    };
                }
                current = &mut items[index];
            }
            Value::Object(map) => {
                if !matches!(map.get(segment), Some(Value::Array(_) | Value::Object(_))) {
                    map.insert(
                        segment.clone(),
                        if next_is_index {
                            Value::Array(Vec::new())
                        } else {
                            Value::Object(Map::new())
                        },
                    );
                }
                current = map.get_mut(segment).unwrap();
            }
            _ => return Err(json_path_error("Invalid JSON path.")),
        }
    }
    let last = segments.last().unwrap();
    match current {
        Value::Array(items) => {
            if !is_index(last) {
                return Err(json_path_error("Invalid JSON path for array."));
            }
            let index = array_index(last)?;
            while items.len() <= index {
                items.push(Value::Null);
            }
            items[index] = to_json(fresh).unwrap_or(Value::Null);
            Ok(root)
        }
        Value::Object(map) => {
            map.insert(last.clone(), to_json(fresh).unwrap_or(Value::Null));
            Ok(root)
        }
        _ => Err(json_path_error("Invalid JSON path.")),
    }
}

fn delete_json_path(base: &Value, segments: &[String]) -> Value {
    if segments.is_empty() {
        return Value::Null;
    }
    let mut root = match base {
        Value::Array(_) | Value::Object(_) => base.clone(),
        _ => return to_json(base).unwrap_or(Value::Null),
    };
    let mut current = &mut root;
    for segment in &segments[..segments.len() - 1] {
        current = match current {
            Value::Array(items) => {
                if !is_index(segment) {
                    return root;
                }
                let index: usize = match segment.parse() {
                    Ok(index) => index,
                    Err(_) => return root,
                };
                match items.get_mut(index) {
                    Some(next) => next,
                    None => return root,
                }
            }
            Value::Object(map) => match map.get_mut(segment) {
                Some(next) => next,
                None => return root,
            },
            _ => return root,
        };
    }
    let last = segments.last().unwrap();
    match current {
        Value::Array(items) => {
            if is_index(last) {
                if let Ok(index) = last.parse::<usize>() {
                    if index < items.len() {
                        items.remove(index);
                    }
                }
            }
        }
        Value::Object(map) => {
            map.remove(last);
        }
        _ => {}
    }
    root
}

fn merge_json(base: &Value, patch: &Value) -> Value {
    if let (Value::Object(left), Value::Object(right)) = (base, patch) {
        let mut result = left.clone();
        for (key, value) in right {
            let merged = match left.get(key) {
                Some(existing) => merge_json(existing, value),
                None => to_json(value).unwrap_or(Value::Null),
            };
            result.insert(key.clone(), merged);
        }
        return Value::Object(result);
    }
    to_json(patch).unwrap_or(Value::Null)
}

pub fn register(d: &mut Definitions) {
    for (name, values_fn) in [("jsonValues", false), ("jsonEntries", true)] {
        d.add(
            name,
            "language",
            1,
            1,
            "[value]",
            "Lists values or key-value pairs of an object or array.",
            move |c| {
                Box::pin(async move {
                    let value = c.value_arg(0).await?;
                    let entries: Vec<Value> = match value {
                        Value::Object(map) => {
                            if values_fn {
                                map.into_iter()
                                    .map(|(key, value)| {
                                        Value::Array(vec![Value::String(key), value])
                                    })
                                    .collect()
                            } else {
                                map.values().cloned().collect()
                            }
                        }
                        Value::Array(items) => {
                            if values_fn {
                                items
                                    .into_iter()
                                    .enumerate()
                                    .map(|(i, value)| {
                                        Value::Array(vec![Value::String(i.to_string()), value])
                                    })
                                    .collect()
                            } else {
                                items
                            }
                        }
                        _ => {
                            return Err(FnError::new(
                                "JSON_OBJECT",
                                "Value must be an object or array.",
                            )
                            .into())
                        }
                    };
                    c.enforce_output_limit(&text(&Value::Array(entries.clone())))?;
                    Ok(Value::Array(entries))
                })
            },
        );
    }

    d.add(
        "object",
        "language",
        0,
        64,
        "[key;value;...]",
        "Builds an object from key;value pairs. An odd argument count is an error.",
        |c| {
            Box::pin(async move {
                if c.arg_count() % 2 == 1 {
                    return Err(FnError::new(
                        "OBJECT",
                        "Odd number of arguments in $object. Use key;value pairs.",
                    )
                    .into());
                }
                let mut result = Map::new();
                let mut i = 0;
                while i < c.arg_count() {
                    let key = c.arg(i).await?;
                    result.insert(
                        key,
                        typed_scalar(&to_json(&c.value_arg(i + 1).await?).unwrap_or(Value::Null)),
                    );
                    i += 2;
                }
                c.enforce_output_limit(&text(&Value::Object(result.clone())))?;
                Ok(Value::Object(result))
            })
        },
    );

    d.add(
        "jsonEncode",
        "language",
        1,
        1,
        "[value]",
        "Encodes a value as JSON text.",
        |c| {
            Box::pin(async move {
                let value = c.value_arg(0).await?;
                Ok(Value::String(crate::values::js_stringify(&value)))
            })
        },
    );

    d.add(
        "jsonDecode",
        "language",
        1,
        1,
        "[text]",
        "Decodes JSON text. Error when invalid.",
        |c| {
            Box::pin(async move {
                let raw = c.arg(0).await?;
                match serde_json::from_str::<Value>(&raw) {
                    Ok(value) => {
                        c.enforce_output_limit(&text(&value))?;
                        Ok(value)
                    }
                    Err(_) => Err(FnError::new("JSON_DECODE", "Invalid JSON.").into()),
                }
            })
        },
    );

    d.add(
        "jsonGet",
        "language",
        2,
        3,
        "[value;path;default?]",
        "Reads a path with dots and indices, e.g. \"a.0.b\". Returns the default when absent.",
        |c| {
            Box::pin(async move {
                let value = c.value_arg(0).await?;
                let segments = parse_path(&c.arg(1).await?)?;
                if segments.is_empty() {
                    return Ok(value);
                }
                match get_by_path(&value, &segments) {
                    Some(found) => Ok(found.clone()),
                    None => Ok(if c.arg_count() > 2 {
                        c.value_arg(2).await?
                    } else {
                        Value::String(String::new())
                    }),
                }
            })
        },
    );

    d.add(
        "jsonSet",
        "language",
        3,
        3,
        "[value;path;new]",
        "Returns a copy with the path changed, without mutating the original.",
        |c| {
            Box::pin(async move {
                let base = c.value_arg(0).await?;
                let segments = parse_path(&c.arg(1).await?)?;
                let fresh = to_json(&c.value_arg(2).await?).unwrap_or(Value::Null);
                set_json_path(&base, &segments, &fresh)
            })
        },
    );

    d.add(
        "jsonDelete",
        "language",
        2,
        2,
        "[value;path]",
        "Returns a copy without the path; removes an array index. An absent path changes nothing.",
        |c| {
            Box::pin(async move {
                let base = c.value_arg(0).await?;
                let segments = parse_path(&c.arg(1).await?)?;
                Ok(delete_json_path(&base, &segments))
            })
        },
    );

    d.add(
        "jsonMerge",
        "language",
        1,
        64,
        "[value;...]",
        "Deep-merges objects without mutation; the last value wins and arrays are replaced.",
        |c| {
            Box::pin(async move {
                let mut result = to_json(&c.value_arg(0).await?).unwrap_or(Value::Null);
                for i in 1..c.arg_count() {
                    result = merge_json(&result, &c.value_arg(i).await?);
                }
                Ok(result)
            })
        },
    );

    d.add(
        "jsonKeys",
        "language",
        1,
        1,
        "[value]",
        "Lists the keys of an object or the indices of an array.",
        |c| {
            Box::pin(async move {
                let value = c.value_arg(0).await?;
                let keys = match value {
                    Value::Object(map) => map.keys().cloned().map(Value::String).collect(),
                    Value::Array(items) => (0..items.len())
                        .map(|i| Value::String(i.to_string()))
                        .collect(),
                    _ => {
                        return Err(
                            FnError::new("JSON_KEYS", "Value must be an object or array.").into(),
                        )
                    }
                };
                Ok(Value::Array(keys))
            })
        },
    );

    d.add(
        "hasProp",
        "language",
        2,
        2,
        "[value;key]",
        "Checks an object's own property.",
        |c| {
            Box::pin(async move {
                let value = c.value_arg(0).await?;
                let key = c.arg(1).await?;
                Ok(Value::Bool(match value {
                    Value::Object(map) => map.contains_key(&key),
                    Value::Array(items) => {
                        is_index(&key)
                            && key.parse::<usize>().is_ok_and(|index| index < items.len())
                    }
                    _ => false,
                }))
            })
        },
    );
}
