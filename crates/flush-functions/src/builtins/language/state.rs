//! Persistent state (`$incrementVar`/`push`/`pull`/`setPath` per scope) and
//! loop-state keys (yuzuha `builtins/language/state.ts`).

use std::sync::Arc;

use serde_json::Value;

use flush_framework::{VariableScope, VariableService, VariableStore};

use super::common::{arg_value, integer, json_clone};
use super::objects::{parse_path, set_json_path};
use crate::errors::{FnError, GuardError, GuardReason, ScriptFailure};
use crate::types::Definitions;
use crate::values::{deep_equal, text, typed_scalar};

pub const LOOP_INDEX_KEY: &str = "__loopIndex";
pub const LOOP_VALUE_KEY: &str = "__loopValue";
pub const LOOP_ACCUMULATOR_KEY: &str = "__loopAccumulator";
pub const LOOP_MAX: usize = 1000;

type Store = Arc<dyn VariableStore + Send + Sync>;

fn variable_error(error: flush_framework::VariableError) -> ScriptFailure {
    match error {
        flush_framework::VariableError::NotInGuild => {
            GuardError::new(crate::context::NOT_IN_GUILD, GuardReason::NotInGuild).into()
        }
        flush_framework::VariableError::NoAuthor => {
            GuardError::new(crate::context::NO_AUTHOR, GuardReason::NoAuthor).into()
        }
        other => FnError::new("VARIABLE", format!("{other}")).into(),
    }
}

fn variables(
    ctx: &crate::context::ScriptContext,
) -> Result<Arc<VariableService<Store>>, ScriptFailure> {
    ctx.variables
        .clone()
        .ok_or_else(|| FnError::new("STORE", "Variable store is not configured.").into())
}

/// `ctx.data[property].update(name, updater, null)` — the updater reports
/// builtin codes through `side` because the transaction signature only
/// carries `VariableError`.
fn scoped_update(
    service: &VariableService<Store>,
    scope: VariableScope,
    name: &str,
    update: impl Fn(Value) -> Result<Value, ScriptFailure>,
) -> Result<Value, ScriptFailure> {
    let side = std::sync::Mutex::new(None);
    let result = service.update(
        scope,
        name,
        Value::Null,
        &mut |current: Value| match update(current.clone()) {
            Ok(value) => Ok(value),
            Err(error) => {
                *side.lock().unwrap() = Some(error);
                Err(flush_framework::VariableError::Operation(
                    "builtin rejected the value".into(),
                ))
            }
        },
    );
    match result {
        Ok(value) => Ok(value),
        Err(error) => match side.into_inner().unwrap() {
            Some(error) => Err(error),
            None => Err(variable_error(error)),
        },
    }
}

fn register_scope(d: &mut Definitions, suffix: &'static str, scope: VariableScope) {
    d.add(
        &format!("increment{suffix}"),
        "language",
        1,
        2,
        "[name;delta?]",
        "Atomically increments a persistent number; returns the new value.",
        move |c| {
            Box::pin(async move {
                let name = c.arg(0).await?;
                let delta = if c.arg_count() > 1 {
                    crate::values::js_number(&c.arg(1).await?).unwrap_or(f64::NAN)
                } else {
                    1.0
                };
                if !delta.is_finite() {
                    return Err(FnError::new("INCREMENT", "Delta must be a finite number.").into());
                }
                let service = variables(c.ctx)?;
                scoped_update(&service, scope, &name, |current| {
                    let base = match &current {
                        Value::Null => 0.0,
                        Value::String(s) => {
                            let trimmed = s.trim();
                            if trimmed.is_empty() {
                                return Err(FnError::new(
                                    "INCREMENT",
                                    "Current variable is not a number.",
                                )
                                .into());
                            }
                            let n = crate::values::js_number(trimmed).unwrap_or(f64::NAN);
                            if !n.is_finite() {
                                return Err(FnError::new(
                                    "INCREMENT",
                                    "Current variable is not a number.",
                                )
                                .into());
                            }
                            n
                        }
                        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
                        _ => {
                            return Err(FnError::new(
                                "INCREMENT",
                                "Current variable is not a number.",
                            )
                            .into())
                        }
                    };
                    let next = base + delta;
                    if !next.is_finite() {
                        return Err(
                            FnError::new("INCREMENT", "Result is not a finite number.").into()
                        );
                    }
                    c.count_output(&text(&super::common::number(next)))?;
                    Ok(super::common::number(next))
                })
            })
        },
    );

    d.add(
        &format!("push{suffix}"),
        "language",
        2,
        2,
        "[name;value]",
        "Appends an item to a persistent array; creates it when absent.",
        move |c| {
            Box::pin(async move {
                let name = c.arg(0).await?;
                let item = typed_scalar(&json_clone(&arg_value(&c.arg(1).await?)));
                let service = variables(c.ctx)?;
                scoped_update(&service, scope, &name, |current| {
                    let next = match current {
                        Value::Null => Value::Array(vec![item.clone()]),
                        Value::Array(mut items) => {
                            items.push(item.clone());
                            Value::Array(items)
                        }
                        _ => {
                            return Err(
                                FnError::new("PUSH", "Current variable is not an array.").into()
                            )
                        }
                    };
                    c.count_output(&text(&next))?;
                    Ok(next)
                })
            })
        },
    );

    d.add(
        &format!("pull{suffix}"),
        "language",
        2,
        2,
        "[name;value]",
        "Removes every occurrence of the value from a persistent array.",
        move |c| {
            Box::pin(async move {
                let name = c.arg(0).await?;
                let target = typed_scalar(&json_clone(&arg_value(&c.arg(1).await?)));
                let service = variables(c.ctx)?;
                scoped_update(&service, scope, &name, |current| {
                    let next = match current {
                        Value::Null => Value::Array(Vec::new()),
                        Value::Array(items) => Value::Array(
                            items
                                .into_iter()
                                .filter(|item| !deep_equal(item, &target))
                                .collect(),
                        ),
                        _ => {
                            return Err(
                                FnError::new("PULL", "Current variable is not an array.").into()
                            )
                        }
                    };
                    c.count_output(&text(&next))?;
                    Ok(next)
                })
            })
        },
    );

    d.add(
        &format!("set{suffix}Path"),
        "language",
        3,
        3,
        "[name;path;new]",
        "Atomically sets a path on a persistent object; creates the missing structure and returns the full value.",
        move |c| {
            Box::pin(async move {
                let name = c.arg(0).await?;
                let segments = parse_path(&c.arg(1).await?)?;
                let fresh = typed_scalar(&json_clone(&arg_value(&c.arg(2).await?)));
                let service = variables(c.ctx)?;
                scoped_update(&service, scope, &name, |current| {
                    let next = set_json_path(&current, &segments, &fresh)?;
                    c.count_output(&text(&next))?;
                    Ok(next)
                })
            })
        },
    );
}

pub fn register(d: &mut Definitions) {
    for (suffix, scope) in [
        ("Var", VariableScope::Global),
        ("GuildVar", VariableScope::Guild),
        ("UserVar", VariableScope::User),
    ] {
        register_scope(d, suffix, scope);
    }

    d.add(
        "caseList",
        "language",
        0,
        2,
        "[page?;size?]",
        "Lists the server's cases; 1-based page, default size 10, maximum 100.",
        |c| {
            Box::pin(async move {
                let page = if c.arg_count() > 0 {
                    integer(&c.arg(0).await?, "Page")?
                } else {
                    1
                };
                let size = if c.arg_count() > 1 {
                    integer(&c.arg(1).await?, "Size")?
                } else {
                    10
                };

                if page < 1 {
                    return Err(FnError::new("CASE_LIST", "Page starts at 1.").into());
                }
                if !(1..=100).contains(&size) {
                    return Err(FnError::new("CASE_LIST", "Size must be between 1 and 100.").into());
                }
                let guild_id = c.ctx.require_guild()?;
                let Some(cases) = c.ctx.cases.clone() else {
                    return Err(FnError::new("STORE", "Case store is not configured.").into());
                };
                let records = cases.lock().unwrap().for_guild(&guild_id.to_string());
                let start = ((page - 1) * size) as usize;
                let end = (page * size) as usize;
                let slice: Vec<Value> = records
                    .iter()
                    .skip(start)
                    .take(end.saturating_sub(start))
                    .map(|case| serde_json::to_value(case).unwrap_or(Value::Null))
                    .collect();
                Ok(Value::Array(slice))
            })
        },
    );

    d.add(
        "caseCount",
        "language",
        0,
        0,
        "",
        "Number of cases of the current server.",
        |c| {
            Box::pin(async move {
                let guild_id = c.ctx.require_guild()?;
                let Some(cases) = c.ctx.cases.clone() else {
                    return Err(FnError::new("STORE", "Case store is not configured.").into());
                };
                let count = cases.lock().unwrap().count(&guild_id.to_string());
                Ok(Value::from(count as u64))
            })
        },
    );
}
