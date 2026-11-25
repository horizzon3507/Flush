//! Port of `src/functions/builtins/schedule.ts`.

use serde_json::{Map, Value};

use crate::errors::{codes, FnError, ScriptFailure};
use crate::scheduler::{ScheduledJob, ScriptScheduler};
use crate::types::Definitions;
use crate::values::text;

use super::support;

const RETRY_KEYS: [&str; 5] = [
    "maxAttempts",
    "retryDelayMs",
    "retryBackoff",
    "retryJitter",
    "priority",
];

fn scheduler(
    c: &crate::call::ScriptCall<'_>,
) -> Result<std::sync::Arc<ScriptScheduler>, ScriptFailure> {
    c.ctx.scheduler.clone().ok_or_else(|| {
        FnError::new(codes::NATIVE_UNAVAILABLE, "Scheduling is not configured.").into()
    })
}

fn schedule_error(message: impl Into<String>) -> ScriptFailure {
    FnError::new(codes::SCHEDULE, message.into()).into()
}

fn number(field: &str, value: &Value) -> Result<f64, ScriptFailure> {
    value
        .as_f64()
        .ok_or_else(|| schedule_error(format!("Schedule option '{field}' must be a number.")))
}

/// Validates a schedule options object against an allowlist and fills a job.
fn job_from_options(
    c: &crate::call::ScriptCall<'_>,
    options: &Map<String, Value>,
    allowed: &[&str],
) -> Result<ScheduledJob, ScriptFailure> {
    for key in options.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(schedule_error(format!("Unknown schedule option '{key}'.")));
        }
    }
    let command = options
        .get("command")
        .map(text)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| schedule_error("Schedule option 'command' is required."))?;
    let mut job = ScheduledJob {
        id: String::new(),
        command,
        input: options.get("input").map(text),
        due: None,
        interval_ms: None,
        cron: None,
        guild_id: c.ctx.guild_id().to_string(),
        channel_id: c.ctx.channel_id().to_string(),
        author_id: c.ctx.author_id().to_string(),
        priority: 0,
        max_attempts: 5,
        retry_delay_ms: 30_000,
        retry_backoff: 1.0,
        retry_jitter: 0.0,
        paused: false,
    };
    for key in RETRY_KEYS {
        if let Some(value) = options.get(key) {
            let n = number(key, value)?;
            match key {
                "maxAttempts" => job.max_attempts = n.max(1.0) as u64,
                "retryDelayMs" => job.retry_delay_ms = n.max(0.0) as u64,
                "retryBackoff" => job.retry_backoff = n,
                "retryJitter" => job.retry_jitter = n.clamp(0.0, 1.0),
                "priority" => job.priority = n as i64,
                _ => {}
            }
        }
    }
    Ok(job)
}

fn options_obj(raw: &str) -> Result<Map<String, Value>, ScriptFailure> {
    match support::json_arg(raw) {
        Value::Object(map) => Ok(map),
        _ => Err(schedule_error("Expected a schedule options object.")),
    }
}

fn scope(c: &crate::call::ScriptCall<'_>) -> (String, String) {
    (c.ctx.guild_id().to_string(), c.ctx.author_id().to_string())
}

pub fn register(d: &mut Definitions) {
    d.add(
        "schedule",
        "schedule",
        1,
        1,
        "[opções]",
        "Schedules a command to run once ({command, at|delayMs, ...}).",
        |c| {
            Box::pin(async move {
                let scheduler = scheduler(c)?;
                let options = options_obj(&c.arg(0).await?)?;
                let allowed = [
                    "command",
                    "input",
                    "at",
                    "delayMs",
                    "maxAttempts",
                    "retryDelayMs",
                    "retryBackoff",
                    "retryJitter",
                    "priority",
                ];
                let mut job = job_from_options(c, &options, &allowed)?;
                let now = (c.ctx.now)();
                job.due = Some(if let Some(at) = options.get("at") {
                    number("at", at)? as u64
                } else if let Some(delay) = options.get("delayMs") {
                    now + number("delayMs", delay)? as u64
                } else {
                    return Err(schedule_error(
                        "$schedule needs an 'at' or 'delayMs' option.",
                    ));
                });
                Ok(Value::String(scheduler.add(job)))
            })
        },
    );

    d.add(
        "scheduleEvery",
        "schedule",
        1,
        1,
        "[opções]",
        "Schedules a repeating command ({command, everyMs, ...}).",
        |c| {
            Box::pin(async move {
                let scheduler = scheduler(c)?;
                let options = options_obj(&c.arg(0).await?)?;
                let allowed = [
                    "command",
                    "input",
                    "everyMs",
                    "startAt",
                    "maxAttempts",
                    "retryDelayMs",
                    "retryBackoff",
                    "retryJitter",
                    "priority",
                ];
                let mut job = job_from_options(c, &options, &allowed)?;
                let every = options
                    .get("everyMs")
                    .ok_or_else(|| schedule_error("$scheduleEvery needs an 'everyMs' option."))?;
                job.interval_ms = Some(number("everyMs", every)? as u64);
                job.due = Some(match options.get("startAt") {
                    Some(at) => number("startAt", at)? as u64,
                    None => (c.ctx.now)() + job.interval_ms.unwrap(),
                });
                Ok(Value::String(scheduler.add(job)))
            })
        },
    );

    d.add(
        "scheduleCron",
        "schedule",
        1,
        1,
        "[opções]",
        "Schedules a command on a cron expression.",
        |c| {
            Box::pin(async move {
                let scheduler = scheduler(c)?;
                let options = options_obj(&c.arg(0).await?)?;
                let allowed = [
                    "command",
                    "input",
                    "expression",
                    "timezone",
                    "missedRun",
                    "maxAttempts",
                    "retryDelayMs",
                    "retryBackoff",
                    "retryJitter",
                    "priority",
                ];
                let mut job = job_from_options(c, &options, &allowed)?;
                let expression = options
                    .get("expression")
                    .map(text)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| schedule_error("$scheduleCron needs an 'expression' option."))?;
                let timezone = options
                    .get("timezone")
                    .map(text)
                    .unwrap_or_else(|| "UTC".into());
                let missed_run = match options.get("missedRun").map(text).as_deref() {
                    None | Some("skip") => "skip",
                    Some("runOnce") => "runOnce",
                    Some("error") => "error",
                    Some(other) => {
                        return Err(schedule_error(format!(
                        "Unknown cron missedRun '{other}': expected 'skip', 'runOnce' or 'error'."
                    )))
                    }
                };
                job.cron = Some((expression, timezone, missed_run.to_string()));
                Ok(Value::String(scheduler.add(job)))
            })
        },
    );

    d.add(
        "unschedule",
        "schedule",
        1,
        1,
        "[id]",
        "Cancels a scheduled job.",
        |c| {
            Box::pin(async move {
                let scheduler = scheduler(c)?;
                let (guild, author) = scope(c);
                Ok(Value::Bool(scheduler.cancel(
                    &c.arg(0).await?,
                    &guild,
                    &author,
                )))
            })
        },
    );

    d.add(
        "editSchedule",
        "schedule",
        2,
        2,
        "[id;opções]",
        "Edits a scheduled job's whitelisted options.",
        |c| {
            Box::pin(async move {
                let scheduler = scheduler(c)?;
                let id = c.arg(0).await?;
                let patch = options_obj(&c.arg(1).await?)?;
                let (guild, author) = scope(c);
                scheduler
                    .edit(&id, &patch, &guild, &author)
                    .map_err(schedule_error)
                    .map(Value::Bool)
            })
        },
    );

    d.add(
        "pauseSchedule",
        "schedule",
        1,
        1,
        "[id]",
        "Pauses a scheduled job.",
        |c| {
            Box::pin(async move {
                Ok(Value::Bool(scheduler(c)?.set_paused(
                    &c.arg(0).await?,
                    true,
                    None,
                )))
            })
        },
    );

    d.add(
        "resumeSchedule",
        "schedule",
        1,
        2,
        "[id;due?]",
        "Resumes a paused scheduled job.",
        |c| {
            Box::pin(async move {
                let due = if c.arg_count() > 1 {
                    let raw = c.arg(1).await?;
                    let value = support::parse_duration(&Value::String(raw.clone()))
                        .or_else(|_| support::finite(&Value::String(raw)).map(|n| n as u64))?;
                    Some(value)
                } else {
                    None
                };
                Ok(Value::Bool(scheduler(c)?.set_paused(
                    &c.arg(0).await?,
                    false,
                    due,
                )))
            })
        },
    );

    d.add(
        "schedules",
        "schedule",
        0,
        0,
        "",
        "Lists every scheduled job.",
        |c| Box::pin(async move { Ok(Value::Array(scheduler(c)?.list())) }),
    );

    d.add(
        "scheduleHistory",
        "schedule",
        0,
        0,
        "",
        "Lists finished scheduled runs.",
        |c| {
            Box::pin(async move {
                let (guild, author) = scope(c);
                Ok(Value::Array(scheduler(c)?.history(&guild, &author)))
            })
        },
    );

    d.add(
        "failedSchedules",
        "schedule",
        0,
        0,
        "",
        "Lists failed scheduled executions.",
        |c| {
            Box::pin(async move {
                let (guild, author) = scope(c);
                Ok(Value::Array(scheduler(c)?.failed(&guild, &author)))
            })
        },
    );

    d.add(
        "retrySchedule",
        "schedule",
        1,
        2,
        "[executionID;due?]",
        "Retries a failed scheduled execution.",
        |c| {
            Box::pin(async move {
                let execution_id = c.arg(0).await?;
                let due = if c.arg_count() > 1 {
                    support::finite(&Value::String(c.arg(1).await?))? as u64
                } else {
                    (c.ctx.now)()
                };
                let (guild, author) = scope(c);
                scheduler(c)?
                    .retry(&execution_id, due, &guild, &author)
                    .map_err(schedule_error)
                    .map(Value::String)
            })
        },
    );
}
