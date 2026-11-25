//! Text builtins (yuzuha `builtins/language/texts.ts`).

use serde_json::Value;

use super::common::integer;
use crate::errors::FnError;
use crate::types::Definitions;

const MENTION_TYPES: [&str; 3] = ["user", "role", "channel"];
const MARKDOWN_SPECIALS: [char; 7] = ['\\', '*', '_', '~', '`', '>', '|'];

fn pad(value: &str, size: usize, filling: &str, left: bool) -> Result<String, FnError> {
    if size > 100_000 {
        return Err(FnError::new("PAD", "Size must be between 0 and 100000."));
    }
    if filling.is_empty() {
        return Err(FnError::new("PAD", "Filling cannot be empty."));
    }
    let chars: Vec<char> = value.chars().collect();
    if chars.len() >= size {
        return Ok(value.to_string());
    }
    let missing = size - chars.len();
    let fill_chars: Vec<char> = filling.chars().collect();
    let mut filler = String::new();
    while filler.chars().count() < missing {
        filler.extend(fill_chars.iter());
    }
    let filler: String = filler.chars().take(missing).collect();
    Ok(if left {
        format!("{filler}{value}")
    } else {
        format!("{value}{filler}")
    })
}

pub fn register(d: &mut Definitions) {
    d.add(
        "split",
        "language",
        2,
        2,
        "[text;separator]",
        "Splits text; an empty separator returns the characters.",
        |c| {
            Box::pin(async move {
                let value = c.arg(0).await?;
                let separator = c.arg(1).await?;
                let parts: Vec<String> = if separator.is_empty() {
                    value.chars().map(|ch| ch.to_string()).collect()
                } else {
                    value.split(&separator).map(str::to_string).collect()
                };
                if parts.len() > 1000 {
                    return Err(FnError::new("SPLIT", "Result exceeds 1000 parts.").into());
                }
                Ok(Value::Array(parts.into_iter().map(Value::String).collect()))
            })
        },
    );

    d.add(
        "sliceText",
        "language",
        2,
        3,
        "[text;start;end?]",
        "Slices characters; end exclusive.",
        |c| {
            Box::pin(async move {
                let value: Vec<char> = c.arg(0).await?.chars().collect();
                let start = integer(&c.arg(1).await?, "Value")?;
                let end = if c.arg_count() > 2 {
                    integer(&c.arg(2).await?, "Value")?
                } else {
                    value.len() as i64
                };
                if start < 0 || start as usize > value.len() {
                    return Err(FnError::new("INDEX", "Start out of range.").into());
                }
                if end < start || end as usize > value.len() {
                    return Err(FnError::new("INDEX", "End out of range.").into());
                }
                Ok(Value::String(
                    value[start as usize..end as usize].iter().collect(),
                ))
            })
        },
    );

    for (name, description) in [
        ("includes", "Checks whether the text contains the search."),
        ("startsWith", "Checks the text's prefix."),
        ("endsWith", "Checks the text's suffix."),
    ] {
        d.add(
            name,
            "language",
            2,
            2,
            "[text;search]",
            description,
            move |c| {
                Box::pin(async move {
                    let value = c.arg(0).await?;
                    let search = c.arg(1).await?;
                    Ok(Value::Bool(match name {
                        "includes" => value.contains(&search),
                        "startsWith" => value.starts_with(&search),
                        _ => value.ends_with(&search),
                    }))
                })
            },
        );
    }

    for (name, left) in [("padStart", true), ("padEnd", false)] {
        d.add(
            name,
            "language",
            2,
            3,
            "[text;size;filling?]",
            if left {
                "Pads on the left up to the size."
            } else {
                "Pads on the right up to the size."
            },
            move |c| {
                Box::pin(async move {
                    let value = c.arg(0).await?;
                    let size = integer(&c.arg(1).await?, "Value")?;
                    if size < 0 {
                        return Err(
                            FnError::new("PAD", "Size must be between 0 and 100000.").into()
                        );
                    }
                    let filling = if c.arg_count() > 2 {
                        c.arg(2).await?
                    } else {
                        " ".to_string()
                    };
                    Ok(Value::String(pad(&value, size as usize, &filling, left)?))
                })
            },
        );
    }

    d.add(
        "repeatText",
        "language",
        2,
        2,
        "[text;times]",
        "Repeats text; at most 100000 characters.",
        |c| {
            Box::pin(async move {
                let value = c.arg(0).await?;
                let times = integer(&c.arg(1).await?, "Value")?;
                if times < 0 {
                    return Err(FnError::new("REPEAT", "Times must be at least 0.").into());
                }
                if value.len() * times as usize > 100_000 {
                    return Err(FnError::new("REPEAT", "Result exceeds 100000 characters.").into());
                }
                Ok(Value::String(value.repeat(times as usize)))
            })
        },
    );

    d.add(
        "truncate",
        "language",
        2,
        3,
        "[text;limit;suffix?]",
        "Cuts the text at the limit and appends the suffix.",
        |c| {
            Box::pin(async move {
                let value = c.arg(0).await?;
                let limit = integer(&c.arg(1).await?, "Value")?;
                let suffix = if c.arg_count() > 2 {
                    c.arg(2).await?
                } else {
                    "…".to_string()
                };
                if limit < 0 {
                    return Err(FnError::new("TRUNCATE", "Limit must be at least 0.").into());
                }
                let chars: Vec<char> = value.chars().collect();
                if chars.len() <= limit as usize {
                    return Ok(Value::String(value));
                }
                Ok(Value::String(format!(
                    "{}{suffix}",
                    chars[..limit as usize].iter().collect::<String>()
                )))
            })
        },
    );

    d.add(
        "escapeMarkdown",
        "language",
        1,
        1,
        "[text]",
        "Escapes Markdown special characters.",
        |c| {
            Box::pin(async move {
                let value = c.arg(0).await?;
                Ok(Value::String(
                    value
                        .chars()
                        .map(|ch| {
                            if MARKDOWN_SPECIALS.contains(&ch) {
                                format!("\\{ch}")
                            } else {
                                ch.to_string()
                            }
                        })
                        .collect(),
                ))
            })
        },
    );

    d.add(
        "mention",
        "language",
        1,
        2,
        "[id;type?]",
        "Mentions a user, role or channel; the ID must be numeric.",
        |c| {
            Box::pin(async move {
                let id = c.arg(0).await?.trim().to_string();
                let kind = if c.arg_count() > 1 {
                    c.arg(1).await?.trim().to_lowercase()
                } else {
                    "user".to_string()
                };
                if !id.chars().all(|ch| ch.is_ascii_digit()) || id.is_empty() {
                    return Err(FnError::new("MENTION", "ID must be numeric.").into());
                }
                if !MENTION_TYPES.contains(&kind.as_str()) {
                    return Err(
                        FnError::new("MENTION", "Type must be user, role or channel.").into(),
                    );
                }
                Ok(Value::String(match kind.as_str() {
                    "user" => format!("<@{id}>"),
                    "role" => format!("<@&{id}>"),
                    _ => format!("<#{id}>"),
                }))
            })
        },
    );
}
