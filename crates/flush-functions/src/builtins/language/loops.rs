//! `$while`/`$repeat`/`$forEach`/`$break`/`$continue` and the loop-var
//! readers (yuzuha `builtins/language/loops.ts`).

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use super::common::{integer, require_array};
use super::state::{LOOP_ACCUMULATOR_KEY, LOOP_INDEX_KEY, LOOP_MAX, LOOP_VALUE_KEY};
use crate::call::ScriptCall;
use crate::context::ScriptContext;
use crate::errors::{FnError, LoopAction, LoopControl, ScriptFailure};
use crate::types::Definitions;
use crate::values::truthy;

static NEXT_FRAME: AtomicU64 = AtomicU64::new(1);

fn next_frame() -> u64 {
    NEXT_FRAME.fetch_add(1, Ordering::Relaxed)
}

const LOOP_KEYS: [&str; 3] = [LOOP_INDEX_KEY, LOOP_VALUE_KEY, LOOP_ACCUMULATOR_KEY];

fn snapshot(ctx: &ScriptContext) -> Vec<(String, Value)> {
    let vars = ctx.vars.lock().unwrap();
    LOOP_KEYS
        .iter()
        .filter_map(|key| vars.get(*key).map(|v| (key.to_string(), v.clone())))
        .collect()
}

fn restore(ctx: &ScriptContext, saved: &[(String, Value)]) {
    let mut vars = ctx.vars.lock().unwrap();
    for key in LOOP_KEYS {
        match saved.iter().find(|(k, _)| k == key) {
            Some((_, value)) => vars.insert(key.to_string(), value.clone()),
            None => vars.remove(key),
        };
    }
}

/// `withLoopState`: array callbacks get their own loop-var scope but are
/// not `$break`/`$continue` targets.
pub async fn with_loop_state<T>(
    ctx: &ScriptContext,
    run: impl Future<Output = Result<T, ScriptFailure>>,
) -> Result<T, ScriptFailure> {
    let saved = snapshot(ctx);
    let frame = ctx.loop_frame();
    ctx.set_loop_frame(None);
    let result = run.await;
    restore(ctx, &saved);
    ctx.set_loop_frame(frame);
    result
}

struct BodyRun {
    value: String,
    stop: bool,
}

/// The body may span several arguments; all are re-evaluated per iteration.
async fn run_body(c: &ScriptCall<'_>, frame: u64) -> Result<BodyRun, ScriptFailure> {
    let mut result = String::new();
    for i in 1..c.arg_count() {
        match c.fresh(i).await {
            Ok(part) => {
                result.push_str(&part);
                c.enforce_output_limit(&result)?;
            }
            Err(ScriptFailure::Loop(control)) if control.frame == frame => {
                return Ok(BodyRun {
                    value: result,
                    stop: control.action == LoopAction::Break,
                });
            }
            Err(other) => return Err(other),
        }
    }
    Ok(BodyRun {
        value: result,
        stop: false,
    })
}

/// Mirrors the `try/finally` of the yuzuha loops: the body runs in a
/// detached future and the vars + frame are restored no matter how it ends.
async fn looping(
    c: &ScriptCall<'_>,
    run: impl Future<Output = Result<String, ScriptFailure>>,
) -> Result<String, ScriptFailure> {
    let saved = snapshot(c.ctx);
    let previous_frame = c.ctx.loop_frame();
    let result = run.await;
    restore(c.ctx, &saved);
    c.ctx.set_loop_frame(previous_frame);
    result
}

pub fn register(d: &mut Definitions) {
    for action in ["break", "continue"] {
        d.add(
            action,
            "language",
            0,
            0,
            "",
            if action == "break" {
                "Ends the nearest loop; requires a repeat, forEach or while body."
            } else {
                "Skips to the next iteration of the nearest loop."
            },
            move |c| {
                Box::pin(async move {
                    let control = match c.ctx.loop_frame() {
                        Some(frame) => LoopControl {
                            frame,
                            action: if action == "break" {
                                LoopAction::Break
                            } else {
                                LoopAction::Continue
                            },
                        },
                        None => {
                            return Err(FnError::new(
                                "LOOP_CONTEXT",
                                format!("${action} requires an active loop body."),
                            )
                            .into())
                        }
                    };
                    Err(ScriptFailure::Loop(control))
                })
            },
        );
    }

    d.add(
        "while",
        "language",
        2,
        32,
        "[condition;body;...]",
        "Re-evaluates condition and body; at most 1000 iterations, with shared cancelation and budget.",
        |c| {
            Box::pin(async move {
                let frame = next_frame();
                looping(c, async move {
                    let mut result = String::new();
                    for index in 0.. {
                        c.ctx.set_loop_frame(None);
                        c.ctx.var_set(LOOP_INDEX_KEY, Value::from(index));
                        c.ctx.var_delete(LOOP_VALUE_KEY);
                        if !truthy(&Value::String(c.fresh(0).await?)) {
                            return Ok(result);
                        }
                        if index == LOOP_MAX as i64 {
                            return Err(FnError::new(
                                "EXECUTION_LIMIT",
                                format!("Loop exceeds {LOOP_MAX} iterations."),
                            )
                            .into());
                        }
                        c.ctx.set_loop_frame(Some(frame));
                        let part = run_body(c, frame).await?;
                        result.push_str(&part.value);
                        c.enforce_output_limit(&result)?;
                        if part.stop {
                            return Ok(result);
                        }
                    }
                    unreachable!()
                })
                .await
                .map(Value::String)
            })
        },
    );

    d.add(
        "repeat",
        "language",
        2,
        32,
        "[times;body;...]",
        "Repeats the body, re-evaluating it each iteration; returns the concatenation.",
        |c| {
            Box::pin(async move {
                let times = integer(&c.arg(0).await?, "Times")?;
                if times < 0 {
                    return Err(FnError::new("REPEAT_COUNT", "Times must be at least 0.").into());
                }
                if times as usize > LOOP_MAX {
                    return Err(FnError::new(
                        "REPEAT_COUNT",
                        format!("Times exceeds the limit of {LOOP_MAX}."),
                    )
                    .into());
                }
                let frame = next_frame();
                looping(c, async move {
                    let mut result = String::new();
                    for i in 0..times {
                        c.ctx.set_loop_frame(Some(frame));
                        c.ctx.var_set(LOOP_INDEX_KEY, Value::from(i));
                        c.ctx.var_delete(LOOP_VALUE_KEY);
                        let part = run_body(c, frame).await?;
                        result.push_str(&part.value);
                        c.enforce_output_limit(&result)?;
                        if part.stop {
                            break;
                        }
                    }
                    Ok(result)
                })
                .await
                .map(Value::String)
            })
        },
    );

    d.add(
        "forEach",
        "language",
        2,
        32,
        "[array;body;...]",
        "Concatenates the body for each item, with $loopValue and $loopIndex.",
        |c| {
            Box::pin(async move {
                let items = require_array(c.value_arg(0).await?)?;
                if items.len() > LOOP_MAX {
                    return Err(FnError::new(
                        "FOREACH",
                        format!("Array exceeds the limit of {LOOP_MAX} items."),
                    )
                    .into());
                }
                let frame = next_frame();
                looping(c, async move {
                    let mut result = String::new();
                    for (i, item) in items.iter().enumerate() {
                        c.ctx.set_loop_frame(Some(frame));
                        c.ctx.var_set(LOOP_INDEX_KEY, Value::from(i as i64));
                        c.ctx.var_set(LOOP_VALUE_KEY, item.clone());
                        let part = run_body(c, frame).await?;
                        result.push_str(&part.value);
                        c.enforce_output_limit(&result)?;
                        if part.stop {
                            break;
                        }
                    }
                    Ok(result)
                })
                .await
                .map(Value::String)
            })
        },
    );

    for (name, key, description) in [
        (
            "loopIndex",
            LOOP_INDEX_KEY,
            "Index of the current iteration; empty outside loops.",
        ),
        (
            "loopValue",
            LOOP_VALUE_KEY,
            "Value of the current iteration; empty outside loops.",
        ),
        (
            "loopAccumulator",
            LOOP_ACCUMULATOR_KEY,
            "Current accumulator of $arrayReduce; empty outside the reduction.",
        ),
    ] {
        d.add(name, "language", 0, 0, "", description, move |c| {
            Box::pin(async move {
                Ok(match c.ctx.var_get(key) {
                    Some(value) => value,
                    None => Value::String(String::new()),
                })
            })
        });
    }
}
