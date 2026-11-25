//! Port of `src/functions/builtins/storage.ts`.

use serde_json::{json, Value};

use flush_framework::{execute_operations, VariableOperation, VariableScope};

use crate::errors::{codes, FnError, ScriptFailure};
use crate::types::Definitions;

use super::support;

fn variable_error(e: flush_framework::VariableError) -> ScriptFailure {
    FnError::new(codes::VARIABLE, e.to_string()).into()
}

/// TTL argument: `null` → keep the variable persistent (no expiry),
/// otherwise a positive duration in ms.
fn ttl_arg(raw: &str) -> Result<Option<u64>, ScriptFailure> {
    if raw.trim().eq_ignore_ascii_case("null") {
        return Ok(None);
    }
    Ok(Some(
        support::finite(&Value::String(raw.to_string()))? as u64
    ))
}

fn var_fns(d: &mut Definitions, scope: VariableScope, suffix: &str, requires_guild: bool) {
    let get_name = format!("get{suffix}Var");
    let set_name = format!("set{suffix}Var");
    let delete_name = format!("delete{suffix}Var");
    let ttl_name = format!("ttl{suffix}Var");
    let expire_name = format!("expire{suffix}Var");

    d.add(
        &get_name,
        "storage",
        1,
        2,
        "[nome;fallback?]",
        "Reads a stored variable.",
        move |c| {
            Box::pin(async move {
                let vars = support::variables(c.ctx)?;
                if requires_guild {
                    c.ctx.require_guild()?;
                }
                let name = c.arg(0).await?;
                let fallback = if c.arg_count() > 1 {
                    support::json_arg(&c.arg(1).await?)
                } else {
                    Value::Null
                };
                vars.get_or(scope, &name, fallback).map_err(variable_error)
            })
        },
    );

    d.add(
        &set_name,
        "storage",
        2,
        3,
        "[nome;valor;ttl?]",
        "Stores a variable.",
        move |c| {
            Box::pin(async move {
                let vars = support::variables(c.ctx)?;
                if requires_guild {
                    c.ctx.require_guild()?;
                }
                let name = c.arg(0).await?;
                let value = support::json_arg(&c.arg(1).await?);
                match c.arg_count() {
                    3 => {
                        let ttl = support::finite(&Value::String(c.arg(2).await?))? as u64;
                        vars.set_with_ttl(scope, &name, value, ttl)
                            .map_err(variable_error)
                    }
                    _ => vars.set(scope, &name, value).map_err(variable_error),
                }
                .map(|_| Value::Null)
            })
        },
    );

    d.add(
        &delete_name,
        "storage",
        1,
        1,
        "[nome]",
        "Deletes a stored variable.",
        move |c| {
            Box::pin(async move {
                let vars = support::variables(c.ctx)?;
                if requires_guild {
                    c.ctx.require_guild()?;
                }
                vars.delete(scope, &c.arg(0).await?)
                    .map_err(variable_error)?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        &ttl_name,
        "storage",
        1,
        1,
        "[nome]",
        "The remaining ttl of a variable in ms (-2 missing, -1 persistent).",
        move |c| {
            Box::pin(async move {
                let vars = support::variables(c.ctx)?;
                if requires_guild {
                    c.ctx.require_guild()?;
                }
                let ttl = vars.ttl(scope, &c.arg(0).await?).map_err(variable_error)?;
                Ok(Value::from(ttl.as_millis()))
            })
        },
    );

    d.add(
        &expire_name,
        "storage",
        2,
        2,
        "[nome;ttl]",
        "Sets ('ms') or clears ('null') a variable's expiry.",
        move |c| {
            Box::pin(async move {
                let vars = support::variables(c.ctx)?;
                if requires_guild {
                    c.ctx.require_guild()?;
                }
                let name = c.arg(0).await?;
                let ttl = ttl_arg(&c.arg(1).await?)?;
                let changed = vars.expire(scope, &name, ttl).map_err(variable_error)?;
                Ok(Value::Bool(changed))
            })
        },
    );
}

pub fn register(d: &mut Definitions) {
    var_fns(d, VariableScope::Global, "", false);
    var_fns(d, VariableScope::Guild, "Guild", true);
    var_fns(d, VariableScope::User, "User", false);

    d.add(
        "queryVars",
        "storage",
        1,
        2,
        "[escopo;opções?]",
        "Queries stored variables with a prefix/cursor/limit options object.",
        |c| {
            Box::pin(async move {
                let vars = support::variables(c.ctx)?;
                let scope = support::parse_scope(&Value::String(c.arg(0).await?))?;
                if scope == VariableScope::Guild {
                    c.ctx.require_guild()?;
                }
                let options = if c.arg_count() > 1 {
                    support::json_arg(&c.arg(1).await?)
                } else {
                    Value::Null
                };
                let query = flush_framework::VariableQueryOptions {
                    prefix: options
                        .get("prefix")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    after: options
                        .get("after")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    limit: options
                        .get("limit")
                        .and_then(Value::as_u64)
                        .map(|n| n as usize),
                };
                let page = vars.query(scope, query).map_err(variable_error)?;
                Ok(json!({
                    "entries": page.entries.iter().map(|e| json!({
                        "name": e.name,
                        "value": e.value,
                        "ttl": e.ttl.as_millis(),
                    })).collect::<Vec<_>>(),
                    "nextCursor": page.next_cursor,
                }))
            })
        },
    );

    d.add(
        "case",
        "storage",
        1,
        1,
        "[id]",
        "Reads a moderation case by id.",
        |c| {
            Box::pin(async move {
                let guild_id = c.ctx.require_guild()?;
                let store = c.ctx.cases.clone().ok_or_else(|| {
                    FnError::new(codes::NATIVE_UNAVAILABLE, "Case storage is not configured.")
                })?;
                let case_id = support::index(&Value::String(c.arg(0).await?), 1)? as u64;
                let case = store
                    .lock()
                    .unwrap()
                    .get(&guild_id.to_string(), case_id)
                    .ok_or_else(|| {
                        FnError::new(
                            codes::FUNCTION_ERROR,
                            format!("Case {case_id} was not found."),
                        )
                    })?;
                Ok(json!({
                    "id": case.id,
                    "kind": case.kind.as_str(),
                    "targetID": case.target_id,
                    "moderatorID": case.moderator_id,
                    "reason": case.reason,
                    "timestamp": case.timestamp,
                    "reverted": case.reverted,
                }))
            })
        },
    );

    d.add(
        "dataTransaction",
        "storage",
        1,
        1,
        "[operações]",
        "Runs a list of variable operations atomically.",
        |c| {
            Box::pin(async move {
                let vars = support::variables(c.ctx)?;
                let raw = c.arg(0).await?;
                let operations: Vec<VariableOperation> =
                    serde_json::from_str(&raw).map_err(|e| {
                        FnError::new(codes::DATA_TRANSACTION, format!("Invalid operations: {e}"))
                    })?;
                vars.transaction(|tx| execute_operations(tx, &operations))
                    .map(Value::Array)
                    .map_err(variable_error)
            })
        },
    );
}
