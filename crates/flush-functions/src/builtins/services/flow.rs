//! Port of `src/functions/builtins/flow.ts`.

use serde_json::Value;

use crate::errors::{codes, is_uncatchable, FnError, ScriptFailure, StopExecution};
use crate::types::Definitions;
use crate::values::truthy;

pub fn register(d: &mut Definitions) {
    d.add(
        "if",
        "flow",
        2,
        3,
        "[condição;então;senão?]",
        "Conditional branch.",
        |c| {
            Box::pin(async move {
                let condition = Value::String(c.arg(0).await?);
                if truthy(&condition) {
                    Ok(Value::String(c.arg(1).await?))
                } else if c.arg_count() > 2 {
                    Ok(Value::String(c.arg(2).await?))
                } else {
                    Ok(Value::Null)
                }
            })
        },
    );

    d.add(
        "stop",
        "flow",
        0,
        0,
        "",
        "Stops the command silently.",
        |_c| Box::pin(async move { Err(ScriptFailure::Stop(StopExecution)) }),
    );

    d.add(
        "throw",
        "flow",
        1,
        1,
        "[mensagem]",
        "Throws a user error.",
        |c| {
            Box::pin(async move {
                let message = c.arg(0).await?;
                Err(FnError::new(codes::USER_ERROR, message).into())
            })
        },
    );

    d.add(
        "try",
        "flow",
        2,
        2,
        "[código;fallback]",
        "Runs code, falling back on a caught failure.",
        |c| {
            Box::pin(async move {
                match Box::pin(c.fresh(0)).await {
                    Ok(v) => Ok(Value::String(v)),
                    Err(e) if is_uncatchable(&e) => Err(e),
                    Err(_) => Ok(Value::String(c.arg(1).await?)),
                }
            })
        },
    );
}
