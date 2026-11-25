//! In-memory scheduler records for the `schedule` script builtins.
//!
//! Mirrors `commands/context.ts` (`ctx.scheduler`) of yuzuha: job metadata,
//! pause/resume, history and retry tracking. Firing the stored command is a
//! host concern — this holds the book-keeping the builtins expose.

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{json, Map, Value};

/// One scheduled job record.
#[derive(Debug, Clone)]
pub struct ScheduledJob {
    pub id: String,
    pub command: String,
    pub input: Option<String>,
    /// Next due timestamp (epoch ms); absent for cron jobs.
    pub due: Option<u64>,
    pub interval_ms: Option<u64>,
    /// `[expression, timezone, missedRun]` for cron jobs.
    pub cron: Option<(String, String, String)>,
    pub guild_id: String,
    pub channel_id: String,
    pub author_id: String,
    pub priority: i64,
    pub max_attempts: u64,
    pub retry_delay_ms: u64,
    pub retry_backoff: f64,
    pub retry_jitter: f64,
    pub paused: bool,
}

impl ScheduledJob {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "command": self.command,
            "input": self.input,
            "due": self.due,
            "intervalMs": self.interval_ms,
            "cron": self.cron.as_ref().map(|(e, t, m)| json!({
                "expression": e,
                "timezone": t,
                "missedRun": m,
            })),
            "guildID": self.guild_id,
            "channelID": self.channel_id,
            "authorID": self.author_id,
            "priority": self.priority,
            "maxAttempts": self.max_attempts,
            "retryDelayMs": self.retry_delay_ms,
            "retryBackoff": self.retry_backoff,
            "retryJitter": self.retry_jitter,
            "paused": self.paused,
        })
    }
}

/// A finished run of a scheduled job.
#[derive(Debug, Clone)]
pub struct ScheduleHistoryEntry {
    pub execution_id: String,
    pub job_id: String,
    pub guild_id: String,
    pub author_id: String,
    pub status: String,
    pub value: Value,
}

/// In-memory scheduler store.
#[derive(Debug, Default)]
pub struct ScriptScheduler {
    next: Mutex<u64>,
    jobs: Mutex<HashMap<String, ScheduledJob>>,
    history: Mutex<Vec<ScheduleHistoryEntry>>,
    failed: Mutex<Vec<ScheduleHistoryEntry>>,
}

impl ScriptScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    fn take_id(&self, prefix: &str) -> String {
        let mut next = self.next.lock().unwrap();
        *next += 1;
        format!("{prefix}-{}", *next)
    }

    /// `scheduler.add(options)` — returns the new job id.
    pub fn add(&self, mut job: ScheduledJob) -> String {
        job.id = self.take_id("job");
        let id = job.id.clone();
        self.jobs.lock().unwrap().insert(id.clone(), job);
        id
    }

    /// `scheduler.cancel(id, scope)` — only the author's guild-scoped jobs.
    pub fn cancel(&self, id: &str, guild_id: &str, author_id: &str) -> bool {
        let mut jobs = self.jobs.lock().unwrap();
        match jobs.get(id) {
            Some(job) if job.guild_id == guild_id && job.author_id == author_id => {
                jobs.remove(id);
                true
            }
            _ => false,
        }
    }

    /// `scheduler.edit(id, patch, scope)` — applies whitelisted fields.
    pub fn edit(
        &self,
        id: &str,
        patch: &Map<String, Value>,
        guild_id: &str,
        author_id: &str,
    ) -> Result<bool, String> {
        let mut jobs = self.jobs.lock().unwrap();
        let job = match jobs.get_mut(id) {
            Some(job) if job.guild_id == guild_id && job.author_id == author_id => job,
            _ => return Ok(false),
        };
        for (key, value) in patch {
            match key.as_str() {
                "command" => job.command = as_text(value),
                "input" => job.input = value.as_str().map(str::to_string),
                "due" => job.due = value.as_u64(),
                "intervalMs" => job.interval_ms = value.as_u64(),
                "priority" => job.priority = value.as_i64().unwrap_or(job.priority),
                "paused" => job.paused = value.as_bool().unwrap_or(job.paused),
                "maxAttempts" => {
                    job.max_attempts = value.as_u64().unwrap_or(job.max_attempts);
                }
                "retryDelayMs" => {
                    job.retry_delay_ms = value.as_u64().unwrap_or(job.retry_delay_ms);
                }
                "retryBackoff" => {
                    job.retry_backoff = value.as_f64().unwrap_or(job.retry_backoff);
                }
                "retryJitter" => {
                    job.retry_jitter = value.as_f64().unwrap_or(job.retry_jitter);
                }
                other => {
                    return Err(format!("Unknown schedule option '{other}'."));
                }
            }
        }
        Ok(true)
    }

    pub fn set_paused(&self, id: &str, paused: bool, due: Option<u64>) -> bool {
        let mut jobs = self.jobs.lock().unwrap();
        match jobs.get_mut(id) {
            Some(job) => {
                job.paused = paused;
                if let Some(due) = due {
                    job.due = Some(due);
                }
                true
            }
            None => false,
        }
    }

    /// `scheduler.list()` — every job record.
    pub fn list(&self) -> Vec<Value> {
        self.jobs
            .lock()
            .unwrap()
            .values()
            .map(ScheduledJob::to_json)
            .collect()
    }

    /// `scheduler.history(scope)` — finished runs for the author's guild.
    pub fn history(&self, guild_id: &str, author_id: &str) -> Vec<Value> {
        self.history
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.guild_id == guild_id && e.author_id == author_id)
            .map(history_json)
            .collect()
    }

    /// `scheduler.failed(scope)` — failed executions for the author's guild.
    pub fn failed(&self, guild_id: &str, author_id: &str) -> Vec<Value> {
        self.failed
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.guild_id == guild_id && e.author_id == author_id)
            .map(history_json)
            .collect()
    }

    /// `scheduler.retry(executionID, due, scope)` — reschedules a failed run.
    /// Returns the new job id.
    pub fn retry(
        &self,
        execution_id: &str,
        due: u64,
        guild_id: &str,
        author_id: &str,
    ) -> Result<String, String> {
        let mut failed = self.failed.lock().unwrap();
        let position = failed
            .iter()
            .position(|e| {
                e.execution_id == execution_id && e.guild_id == guild_id && e.author_id == author_id
            })
            .ok_or_else(|| format!("No failed execution '{execution_id}'."))?;
        let entry = failed.remove(position);
        let job = ScheduledJob {
            id: String::new(),
            command: String::new(),
            input: None,
            due: Some(due),
            interval_ms: None,
            cron: None,
            guild_id: entry.guild_id,
            channel_id: String::new(),
            author_id: entry.author_id,
            priority: 0,
            max_attempts: 5,
            retry_delay_ms: 30_000,
            retry_backoff: 1.0,
            retry_jitter: 0.0,
            paused: false,
        };
        drop(failed);
        Ok(self.add(job))
    }

    /// Records a finished execution (used by tests and by the scheduler
    /// driver when one is wired in).
    pub fn record(&self, entry: ScheduleHistoryEntry) {
        let target = if entry.status == "failed" {
            &self.failed
        } else {
            &self.history
        };
        target.lock().unwrap().push(entry);
    }
}

fn history_json(e: &ScheduleHistoryEntry) -> Value {
    json!({
        "executionID": e.execution_id,
        "jobID": e.job_id,
        "guildID": e.guild_id,
        "authorID": e.author_id,
        "status": e.status,
        "value": e.value,
    })
}

fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => crate::values::text(other),
    }
}
