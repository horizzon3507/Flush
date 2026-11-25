//! Port of `src/functions/builtins/pure-errors.ts`.

use serde_json::{json, Value};

use crate::errors::{codes, is_uncatchable, FnError, ScriptFailure};
use crate::types::Definitions;

fn sdk_error(message: impl Into<String>) -> ScriptFailure {
    FnError::new(codes::SDK, message.into()).into()
}

/// Runs the body; SDK errors (rest transport failures surfaced as
/// `API_ERROR`) are classified, anything else is rethrown. Never exposes
/// the message, url or body (yuzuha's sanitization).
async fn classify(c: &crate::call::ScriptCall<'_>) -> Result<Option<()>, ScriptFailure> {
    match Box::pin(c.fresh(1)).await {
        Ok(_) => Ok(None),
        Err(e) if is_uncatchable(&e) => Err(e),
        Err(e) => {
            if let ScriptFailure::Fn(f) = &e {
                if f.code == codes::API_ERROR || f.code == codes::FUNCTION_ERROR {
                    return Ok(Some(()));
                }
            }
            Err(e)
        }
    }
}

pub fn register(d: &mut Definitions) {
    d.add(
        "sdkError",
        "util",
        2,
        2,
        "[operação;código]",
        "Inspects SDK errors: info | text | isFluxer.",
        |c| {
            Box::pin(async move {
                let op = c.arg(0).await?;
                let failure = classify(c).await;
                match failure {
                    Ok(None) => match op.as_str() {
                        "info" => Ok(Value::Null),
                        "text" => Ok(Value::String(String::new())),
                        "isFluxer" => Ok(Value::Bool(false)),
                        _ => Err(sdk_error(format!("Unknown sdkError operation '{op}'."))),
                    },
                    Ok(Some(())) => match op.as_str() {
                        // Rust collapses every rest failure to API_ERROR;
                        // report sanitized metadata only.
                        "info" => Ok(json!({
                            "name": "HTTPError",
                            "code": Value::Null,
                            "statusCode": Value::Null,
                            "isRetryable": Value::Null,
                            "method": Value::Null,
                        })),
                        "text" => Ok(Value::String("HTTPError".to_string())),
                        "isFluxer" => Ok(Value::Bool(false)),
                        _ => Err(sdk_error(format!("Unknown sdkError operation '{op}'."))),
                    },
                    Err(e) => Err(e),
                }
            })
        },
    );
}
