//! `$regexTest`/`$regexMatch`/`$regexReplace` (yuzuha
//! `builtins/language/regex.ts` + `runtime/regex*.mjs`).

use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Map, Value};

use super::common::integer;
use crate::errors::{FnError, ScriptFailure};
use crate::types::Definitions;

const MAX_OUTPUT: usize = 100_000;
const MAX_MATCHES: usize = 1000;
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

fn utf16_index(text: &str, byte_index: usize) -> usize {
    text[..byte_index].chars().map(char::len_utf16).sum()
}

fn substitute(
    replacement: &str,
    text: &str,
    caps: &regex::Captures<'_>,
    groups: &regex::Regex,
    output: &mut usize,
    chunks: &mut Vec<String>,
) -> Result<(), ScriptFailure> {
    fn append(
        output: &mut usize,
        chunks: &mut Vec<String>,
        value: &str,
    ) -> Result<(), ScriptFailure> {
        *output += value.len();
        if *output > MAX_OUTPUT {
            return Err(
                FnError::new("OUTPUT_LIMIT", "Regex result exceeds 100000 characters.").into(),
            );
        }
        chunks.push(value.to_string());
        Ok(())
    }
    let tokens = regex::Regex::new(r"\$(\$|&|`|'|<[^>]+>|\d{1,2})").unwrap();
    let mut last = 0;
    for token in tokens.captures_iter(replacement) {
        let whole = token.get(0).unwrap();
        append(output, chunks, &replacement[last..whole.start()])?;
        let part = token.get(1).unwrap().as_str();
        match part {
            "$" => append(output, chunks, "$")?,
            "&" => append(output, chunks, caps.get(0).unwrap().as_str())?,
            "`" => append(output, chunks, &text[..caps.get(0).unwrap().start()])?,
            "'" => append(output, chunks, &text[caps.get(0).unwrap().end()..])?,
            name if name.starts_with('<') => {
                let group = &name[1..name.len() - 1];
                let named = caps.name(group);
                match named {
                    Some(found) => append(output, chunks, found.as_str())?,
                    None if groups.capture_names().any(|n| n == Some(group)) => {
                        append(output, chunks, "")?
                    }
                    None => append(output, chunks, whole.as_str())?,
                }
            }
            digits => {
                let index: usize = digits.parse().unwrap();
                if index > 0 && index < caps.len() {
                    append(
                        output,
                        chunks,
                        caps.get(index).map(|m| m.as_str()).unwrap_or(""),
                    )?;
                } else if digits.len() == 2 && digits.as_bytes()[0] != b'0' {
                    let tens = (digits.as_bytes()[0] - b'0') as usize;
                    if tens < caps.len() {
                        let head = caps.get(tens).map(|m| m.as_str()).unwrap_or("");
                        append(output, chunks, &format!("{}{}", head, &digits[1..]))?;
                    } else {
                        append(output, chunks, whole.as_str())?;
                    }
                } else {
                    append(output, chunks, whole.as_str())?;
                }
            }
        }
        last = whole.end();
    }
    append(output, chunks, &replacement[last..])?;
    Ok(())
}

#[derive(Clone, Copy)]
enum Operation {
    Test,
    Match,
    Replace,
}

fn run_regex(
    operation: Operation,
    text: String,
    pattern: String,
    flags: String,
    replacement: String,
    timeout_ms: i64,
    aborted: bool,
) -> Result<Value, ScriptFailure> {
    if !(10..=2000).contains(&timeout_ms) {
        return Err(FnError::new(
            "REGEX_TIMEOUT",
            "Regex timeout must be between 10 and 2000 ms.",
        )
        .into());
    }
    if text.len() > 100_000 || pattern.len() > 512 || replacement.len() > 1000 {
        return Err(FnError::new(
            "EXECUTION_LIMIT",
            "Regex exceeds text (100000), pattern (512) or replacement (1000) limits.",
        )
        .into());
    }
    if !flags.chars().all(|ch| "gimsu".contains(ch))
        || flags
            .chars()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != flags.len()
    {
        return Err(FnError::new(
            "REGEX_FLAGS",
            "Accepted flags, without repetition: g, i, m, s, u.",
        )
        .into());
    }
    if aborted {
        return Err(FnError::new("EXECUTION_ABORTED", "Regex cancelled.").into());
    }
    if ACTIVE.load(Ordering::SeqCst) >= 4 {
        return Err(FnError::new(
            "EXECUTION_LIMIT",
            "Limit of four simultaneous regexes reached.",
        )
        .into());
    }
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    // Detached like the Node worker: a timed-out pattern keeps running
    // in the background while the script continues.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(execute(&operation, &text, &pattern, &flags, &replacement));
    });
    let result: Result<Result<Value, ScriptFailure>, FnError> = rx
        .recv_timeout(std::time::Duration::from_millis(timeout_ms as u64))
        .map_err(|_| FnError::new("EXECUTION_LIMIT", "Regex exceeded the execution timeout."));
    ACTIVE.fetch_sub(1, Ordering::SeqCst);
    result?
}

fn execute(
    operation: &Operation,
    text: &str,
    pattern: &str,
    flags: &str,
    replacement: &str,
) -> Result<Value, ScriptFailure> {
    let regex = regex::RegexBuilder::new(pattern)
        .case_insensitive(flags.contains('i'))
        .multi_line(flags.contains('m'))
        .dot_matches_new_line(flags.contains('s'))
        .build()
        .map_err(|_| FnError::new("REGEX_INVALID", "Invalid regex pattern."))?;
    if matches!(operation, Operation::Test) {
        return Ok(Value::Bool(regex.is_match(text)));
    }
    let global = flags.contains('g');
    let mut output = 0usize;
    let mut chunks: Vec<String> = Vec::new();
    let mut matches: Vec<Value> = Vec::new();
    let mut cursor = 0usize;
    let mut count = 0usize;
    let found = regex.captures_iter(text).peekable();
    for caps in found {
        count += 1;
        if count > MAX_MATCHES {
            return Err(FnError::new("EXECUTION_LIMIT", "Regex exceeds 1000 matches.").into());
        }
        let whole = caps.get(0).unwrap();
        match operation {
            Operation::Replace => {
                let between = &text[cursor..whole.start()];
                output += between.len();
                if output > MAX_OUTPUT {
                    return Err(FnError::new(
                        "OUTPUT_LIMIT",
                        "Regex result exceeds 100000 characters.",
                    )
                    .into());
                }
                chunks.push(between.to_string());
                substitute(replacement, text, &caps, &regex, &mut output, &mut chunks)?;
                cursor = whole.end();
            }
            _ => {
                let mut groups_value = Value::Null;
                let mut groups = Map::new();
                for name in regex.capture_names().flatten() {
                    groups.insert(
                        name.to_string(),
                        caps.name(name)
                            .map(|m| Value::String(m.as_str().to_string()))
                            .unwrap_or(Value::Null),
                    );
                }
                if !groups.is_empty() {
                    groups_value = Value::Object(groups);
                }
                let captures: Vec<Value> = (1..caps.len())
                    .map(|i| {
                        caps.get(i)
                            .map(|m| Value::String(m.as_str().to_string()))
                            .unwrap_or(Value::Null)
                    })
                    .collect();
                let value = serde_json::json!({
                    "value": whole.as_str(),
                    "index": utf16_index(text, whole.start()),
                    "captures": captures,
                    "groups": groups_value,
                });
                output += serde_json::to_string(&value).unwrap_or_default().len() + 1;
                if output + 2 > MAX_OUTPUT {
                    return Err(FnError::new(
                        "OUTPUT_LIMIT",
                        "Regex result exceeds 100000 characters.",
                    )
                    .into());
                }
                matches.push(value);
            }
        }
        if !global {
            break;
        }
    }
    if matches!(operation, Operation::Replace) {
        let tail = &text[cursor..];
        output += tail.len();
        if output > MAX_OUTPUT {
            return Err(
                FnError::new("OUTPUT_LIMIT", "Regex result exceeds 100000 characters.").into(),
            );
        }
        chunks.push(tail.to_string());
        return Ok(Value::String(chunks.concat()));
    }
    Ok(Value::Array(matches))
}

pub fn register(d: &mut Definitions) {
    for (name, operation) in [
        ("regexTest", Operation::Test),
        ("regexMatch", Operation::Match),
        ("regexReplace", Operation::Replace),
    ] {
        let replacing = matches!(operation, Operation::Replace);
        let (min, max, usage): (usize, usize, &str) = if replacing {
            (3, 5, "[text;pattern;replacement;flags?;timeoutMs?]")
        } else {
            (2, 4, "[text;pattern;flags?;timeoutMs?]")
        };
        d.add(
            name,
            "language",
            min,
            max,
            usage,
            "Regex on an isolated worker; flags gimsu, default timeout 500 ms, up to 1000 matches and limited output.",
            move |c| {
                Box::pin(async move {
                    let flag_index = if replacing { 3 } else { 2 };
                    let flags = if c.arg_count() > flag_index {
                        c.arg(flag_index).await?
                    } else {
                        String::new()
                    };
                    let timeout_ms = if c.arg_count() > flag_index + 1 {
                        integer(&c.arg(flag_index + 1).await?, "Timeout")?
                    } else {
                        500
                    };
                    let replacement = if replacing {
                        c.arg(2).await?
                    } else {
                        String::new()
                    };
                    let text = c.arg(0).await?;
                    let pattern = c.arg(1).await?;
                    let aborted = c.ctx.aborted.load(Ordering::Relaxed);
                    c.wait(async move {
                        run_regex(operation, text, pattern, flags, replacement, timeout_ms, aborted)
                    })
                    .await
                })
            },
        );
    }
}
