//! Port of `src/functions/builtins/http.ts`.

use std::str::FromStr;
use std::time::Duration;

use futures::io::AsyncReadExt;
use serde_json::{json, Map, Value};

use crate::errors::{codes, FnError, ScriptFailure};
use crate::types::Definitions;
use crate::values::text;

use super::support;

const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
/// Response body cap for `$http`/`$httpJson`/`$httpStream` (5 MiB).
const BODY_LIMIT: usize = 5 * 1024 * 1024;
/// `$httpStream` chunk cap.
const MAX_CHUNKS: usize = 1_000;
/// `$httpMultipart` part cap.
const MAX_PARTS: usize = 100;
/// Frame ids allocated by `$httpStream` loop runs start high to never
/// collide with loop frames owned by the language builtins.
static HTTP_STREAM_FRAME: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 62);

fn http_error(code: &'static str, message: impl Into<String>) -> ScriptFailure {
    FnError::new(code, message.into()).into()
}

fn method(raw: &str) -> Result<surf::http::Method, ScriptFailure> {
    let upper = raw.trim().to_uppercase();
    if !METHODS.contains(&upper.as_str()) {
        return Err(http_error(
            codes::HTTP_METHOD,
            format!("Unknown HTTP method '{raw}': expected one of {METHODS:?}."),
        ));
    }
    surf::http::Method::from_str(&upper)
        .map_err(|_| http_error(codes::HTTP_METHOD, format!("Unknown HTTP method '{raw}'.")))
}

fn url(raw: &str) -> Result<surf::http::Url, ScriptFailure> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(http_error(codes::HTTP_URL, "Expected a URL."));
    }
    surf::http::Url::parse(trimmed)
        .map_err(|_| http_error(codes::HTTP_URL, format!("Invalid URL '{trimmed}'.")))
}

/// `headers()`: `\n`-separated, or comma-separated when each part still
/// looks like `Name: value`.
fn headers(raw: &str) -> Result<Vec<(String, String)>, ScriptFailure> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    // Split on newlines and on commas followed by 'Name:'.
    let mut parts: Vec<&str> = Vec::new();
    for line in trimmed.split('\n') {
        let mut start = 0;
        for (i, _) in line.match_indices(',') {
            let rest = line[i + 1..].trim_start_matches(' ');
            let name_len = rest
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
                .count();
            if name_len > 0 && rest.chars().nth(name_len) == Some(':') {
                parts.push(&line[start..i]);
                start = i + 1;
            }
        }
        parts.push(&line[start..]);
    }
    let mut out = Vec::new();
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some(colon) = part.find(':') else {
            return Err(http_error(
                codes::HTTP_HEADER,
                format!("Malformed header '{part}': expected 'Name: value'."),
            ));
        };
        let (name, value) = (part[..colon].trim(), part[colon + 1..].trim());
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(http_error(
                codes::HTTP_HEADER,
                format!("Malformed header name '{name}'."),
            ));
        }
        out.push((name.to_string(), value.to_string()));
    }
    Ok(out)
}

fn client(c: &crate::call::ScriptCall<'_>) -> surf::Client {
    let timeout = Duration::from_millis(c.evaluator.remaining_ms().clamp(1, 30_000));
    surf::Config::new()
        .set_timeout(Some(timeout))
        .try_into()
        .unwrap_or_else(|_| surf::Client::new())
}

fn transport(e: surf::Error) -> ScriptFailure {
    http_error(codes::HTTP, format!("HTTP request failed: {e}"))
}

/// Reads a response body incrementally, failing as soon as it exceeds
/// `BODY_LIMIT` instead of buffering it whole.
async fn read_body_limited(
    c: &crate::call::ScriptCall<'_>,
    response: &mut surf::Response,
    code: &'static str,
) -> Result<Vec<u8>, ScriptFailure> {
    let mut body = response.take_body();
    let mut buf = vec![0u8; 8 * 1024];
    let mut out = Vec::new();
    loop {
        let read = c
            .wait(async {
                body.read(&mut buf)
                    .await
                    .map_err(|e| http_error(code, format!("HTTP request failed: {e}")))
            })
            .await?;
        if read == 0 {
            return Ok(out);
        }
        if out.len() + read > BODY_LIMIT {
            return Err(http_error(code, "The response body exceeded the limit."));
        }
        out.extend_from_slice(&buf[..read]);
    }
}

async fn request(
    c: &crate::call::ScriptCall<'_>,
    method: surf::http::Method,
    url: surf::http::Url,
    body: Option<Vec<u8>>,
    headers: Vec<(String, String)>,
    content_type: Option<String>,
) -> Result<surf::Response, ScriptFailure> {
    if body.is_some() && matches!(method, surf::http::Method::Get | surf::http::Method::Head) {
        return Err(http_error(
            codes::HTTP_BODY,
            format!("{method} cannot carry a body."),
        ));
    }
    let mut req = client(c).request(method, url);
    for (name, value) in headers {
        req = req.header(name.as_str(), value.as_str());
    }
    if let (Some(ct), true) = (content_type, body.is_some()) {
        req = req.header("Content-Type", ct.as_str());
    }
    let req = match body {
        Some(data) => req.body(surf::Body::from_bytes(data)),
        None => req,
    };
    c.wait(async { req.await.map_err(transport) }).await
}

/// `text(arg)` → bytes when non-empty.
async fn body_arg(
    c: &crate::call::ScriptCall<'_>,
    index: usize,
) -> Result<Option<Vec<u8>>, ScriptFailure> {
    if c.arg_count() <= index {
        return Ok(None);
    }
    let raw = c.arg(index).await?;
    if raw.is_empty() {
        Ok(None)
    } else {
        Ok(Some(raw.into_bytes()))
    }
}

pub fn register(d: &mut Definitions) {
    d.add(
        "http",
        "http",
        2,
        4,
        "[método;url;corpo?;cabeçalhos?]",
        "Issues an HTTP request and returns the status, headers and body.",
        |c| {
            Box::pin(async move {
                let method = method(&c.arg(0).await?)?;
                let url = url(&c.arg(1).await?)?;
                let body = body_arg(c, 2).await?;
                let headers = headers(&if c.arg_count() > 3 {
                    c.arg(3).await?
                } else {
                    String::new()
                })?;
                let mut response = request(c, method, url, body, headers, None).await?;
                let status = u16::from(response.status());
                let headers_json: Map<String, Value> = response
                    .iter()
                    .map(|(name, values)| {
                        (
                            name.as_str().to_lowercase(),
                            json!(values
                                .iter()
                                .map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")),
                        )
                    })
                    .collect();
                let body = read_body_limited(c, &mut response, codes::HTTP).await?;
                Ok(json!({
                    "status": status,
                    "headers": headers_json,
                    "body": String::from_utf8_lossy(&body),
                }))
            })
        },
    );

    d.add(
        "httpJson",
        "http",
        2,
        4,
        "[método;url;corpo?;cabeçalhos?]",
        "Issues an HTTP request and parses the body as JSON.",
        |c| {
            Box::pin(async move {
                let method = method(&c.arg(0).await?)?;
                let url = url(&c.arg(1).await?)?;
                let body = body_arg(c, 2).await?;
                let headers = headers(&if c.arg_count() > 3 {
                    c.arg(3).await?
                } else {
                    String::new()
                })?;
                let mut response = request(
                    c,
                    method,
                    url,
                    body,
                    headers,
                    Some("application/json".into()),
                )
                .await?;
                let status = u16::from(response.status());
                let body = read_body_limited(c, &mut response, codes::HTTP_JSON).await?;
                let parsed = if body.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&body).map_err(|e| {
                        http_error(
                            codes::HTTP_JSON,
                            format!("The response body is not JSON: {e}"),
                        )
                    })?
                };
                Ok(json!({ "status": status, "body": parsed }))
            })
        },
    );

    d.add(
        "httpMultipart",
        "http",
        3,
        4,
        "[url;partes;cabeçalhos?;método?]",
        "Issues a multipart/form-data POST request.",
        |c| {
            Box::pin(async move {
                let url = url(&c.arg(0).await?)?;
                let parts = match support::json_arg(&c.arg(1).await?) {
                    Value::Array(parts) => parts,
                    _ => {
                        return Err(http_error(
                            codes::HTTP_MULTIPART,
                            "Expected a JSON array of multipart parts.",
                        ))
                    }
                };
                if parts.is_empty() || parts.len() > MAX_PARTS {
                    return Err(http_error(
                        codes::HTTP_MULTIPART,
                        format!("A multipart body takes 1 to {MAX_PARTS} parts."),
                    ));
                }
                let headers = headers(&if c.arg_count() > 2 {
                    c.arg(2).await?
                } else {
                    String::new()
                })?;
                let method = if c.arg_count() > 3 {
                    method(&c.arg(3).await?)?
                } else {
                    surf::http::Method::Post
                };
                let boundary = format!("----yuzuha{:x}", (c.ctx.now)());
                let mut body = Vec::new();
                for part in &parts {
                    let name = part
                        .get("name")
                        .map(text)
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| {
                            http_error(codes::HTTP_MULTIPART, "Every part needs a 'name'.")
                        })?;
                    let (bytes, filename) = match part.get("data") {
                        Some(data) => {
                            let raw = text(data);
                            let decoded = base64_decode(&raw).ok_or_else(|| {
                                http_error(
                                    codes::HTTP_MULTIPART,
                                    format!("Part '{name}' has invalid base64 'data'."),
                                )
                            })?;
                            let filename = part
                                .get("filename")
                                .map(text)
                                .filter(|s| !s.is_empty())
                                .ok_or_else(|| {
                                    http_error(
                                        codes::HTTP_MULTIPART,
                                        format!("Part '{name}' with 'data' needs a 'filename'."),
                                    )
                                })?;
                            (decoded, Some(filename))
                        }
                        None => (
                            text(part.get("content").unwrap_or(&Value::Null)).into_bytes(),
                            None,
                        ),
                    };
                    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
                    let mut disposition =
                        format!("Content-Disposition: form-data; name=\"{name}\"");
                    if let Some(filename) = &filename {
                        disposition.push_str(&format!("; filename=\"{filename}\""));
                    }
                    body.extend_from_slice(disposition.as_bytes());
                    body.extend_from_slice(b"\r\n");
                    if let Some(content_type) =
                        part.get("contentType").map(text).filter(|s| !s.is_empty())
                    {
                        body.extend_from_slice(
                            format!("Content-Type: {content_type}\r\n").as_bytes(),
                        );
                    }
                    body.extend_from_slice(b"\r\n");
                    body.extend_from_slice(&bytes);
                    body.extend_from_slice(b"\r\n");
                }
                body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
                let content_type = format!("multipart/form-data; boundary={boundary}");
                let mut response =
                    request(c, method, url, Some(body), headers, Some(content_type)).await?;
                let status = u16::from(response.status());
                let body = read_body_limited(c, &mut response, codes::HTTP_MULTIPART).await?;
                Ok(json!({
                    "status": status,
                    "body": String::from_utf8_lossy(&body),
                }))
            })
        },
    );

    d.add(
        "httpStream",
        "http",
        3,
        5,
        "[método;url;código;corpo?;cabeçalhos?]",
        "Streams a response body in chunks through a loop body.",
        |c| {
            Box::pin(async move {
                let method = method(&c.arg(0).await?)?;
                let url = url(&c.arg(1).await?)?;
                let body = body_arg(c, 3).await?;
                let headers = headers(&if c.arg_count() > 4 {
                    c.arg(4).await?
                } else {
                    String::new()
                })?;
                let mut response = request(c, method, url, body, headers, None).await?;
                let status = u16::from(response.status());
                if !(200..300).contains(&status) {
                    return Err(http_error(codes::HTTP, format!("HTTP {status}.")));
                }
                let frame = HTTP_STREAM_FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let previous_frame = c.ctx.loop_frame();
                c.ctx.set_loop_frame(Some(frame));
                let result = async {
                    let mut body = response.take_body();
                    let mut buf = vec![0u8; 8 * 1024];
                    let mut pending: Vec<u8> = Vec::new();
                    let mut index_ = 0usize;
                    let mut total = 0usize;
                    let mut output = String::new();
                    async fn run_chunk(
                        c: &crate::call::ScriptCall<'_>,
                        index_: &mut usize,
                        output: &mut String,
                        bytes: &[u8],
                    ) -> Result<(), ScriptFailure> {
                        c.ctx.var_set(
                            "__httpChunk",
                            Value::String(String::from_utf8_lossy(bytes).into_owned()),
                        );
                        c.ctx
                            .var_set("__httpChunkIndex", Value::from(*index_ as i64));
                        *index_ += 1;
                        let out = Box::pin(c.fresh(2)).await?;
                        *output = c.check_output(std::mem::take(output), &out);
                        Ok(())
                    }
                    loop {
                        if index_ >= MAX_CHUNKS {
                            return Err(http_error(
                                codes::EXECUTION_LIMIT,
                                format!("$httpStream is capped at {MAX_CHUNKS} chunks."),
                            ));
                        }
                        let read = c
                            .wait(async {
                                body.read(&mut buf).await.map_err(|e| {
                                    http_error(codes::HTTP, format!("HTTP request failed: {e}"))
                                })
                            })
                            .await?;
                        if read == 0 {
                            break;
                        }
                        total += read;
                        if total > BODY_LIMIT {
                            return Err(http_error(
                                codes::HTTP,
                                "The response body exceeded the limit.",
                            ));
                        }
                        pending.extend_from_slice(&buf[..read]);
                        // Emit the longest UTF-8-valid prefix; keep the
                        // trailing partial sequence for the next chunk.
                        let valid = match std::str::from_utf8(&pending) {
                            Ok(_) => pending.len(),
                            Err(e) => e.valid_up_to(),
                        };
                        if valid == 0 {
                            continue;
                        }
                        let chunk: Vec<u8> = pending.drain(..valid).collect();
                        run_chunk(c, &mut index_, &mut output, &chunk).await?;
                    }
                    if !pending.is_empty() && index_ < MAX_CHUNKS {
                        run_chunk(c, &mut index_, &mut output, &pending).await?;
                    }
                    Ok(output)
                }
                .await;
                c.ctx.set_loop_frame(previous_frame);
                let output = result?;
                Ok(Value::String(output))
            })
        },
    );

    d.add(
        "httpChunk",
        "http",
        0,
        0,
        "",
        "The current streamed chunk.",
        |c| Box::pin(async move { Ok(c.ctx.var_get("__httpChunk").unwrap_or(Value::Null)) }),
    );

    d.add(
        "httpChunkIndex",
        "http",
        0,
        0,
        "",
        "The 1-based index of the current chunk.",
        |c| Box::pin(async move { Ok(c.ctx.var_get("__httpChunkIndex").unwrap_or(Value::Null)) }),
    );
}

/// Canonical base64 decode (no whitespace, correct padding).
fn base64_decode(raw: &str) -> Option<Vec<u8>> {
    let raw = raw.trim();
    if raw.len() % 4 != 0 {
        return None;
    }
    let table = |c: u8| -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(raw.len() / 4 * 3);
    let n_chunks = bytes.len() / 4;
    for (index, chunk) in bytes.chunks(4).enumerate() {
        let last = index == n_chunks - 1;
        let pad = chunk.iter().filter(|&&b| b == b'=').count();
        if pad > 2
            || (pad > 0 && !last)
            || chunk[..4 - pad].contains(&b'=')
            || !chunk[4 - pad..].iter().all(|&b| b == b'=')
        {
            return None;
        }
        let mut acc = 0u32;
        for &b in chunk {
            acc = (acc << 6) | if b == b'=' { 0 } else { table(b)? as u32 };
        }
        // '=' counts as 0 bits at the low end, so the first `3 - pad`
        // bytes of the accumulator are the decoded output.
        out.extend_from_slice(&acc.to_be_bytes()[1..4 - pad]);
    }
    Some(out)
}
