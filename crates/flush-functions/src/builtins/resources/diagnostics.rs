//! `resource-diagnostics.ts`: readiness, sharding math, gateway constants and
//! the rate-limit simulator.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::errors::{FnResult, GuardError, GuardReason};
use crate::types::Definitions;

use super::{arg_value, integer, invalid, operator, snowflake};

const MAX_SHARD_COUNT: i64 = 16384;
const MAX_GUILDS_PER_SHARD: i64 = 2500;
const DEFAULT_GUILDS_PER_SHARD: i64 = 1500;
const DEFAULT_RESUME_TTL_MS: i64 = 60000;

const CLOSE_CODES: &[(&str, i64)] = &[
    ("Normal", 1000),
    ("GoingAway", 1001),
    ("ProtocolError", 1002),
    ("UnsupportedData", 1003),
    ("NoStatusReceived", 1005),
    ("AbnormalClosure", 1006),
    ("UnknownError", 4000),
    ("UnknownOpcode", 4001),
    ("DecodeError", 4002),
    ("NotAuthenticated", 4003),
    ("AuthenticationFailed", 4004),
    ("AlreadyAuthenticated", 4005),
    ("InvalidSeq", 4007),
    ("RateLimited", 4008),
    ("SessionTimeout", 4009),
    ("InvalidShard", 4010),
    ("ShardingRequired", 4011),
    ("InvalidAPIVersion", 4012),
    ("AckBackpressure", 4013),
];

const FATAL_CLOSE: &[i64] = &[1002, 1003, 4001, 4002, 4003, 4004, 4005, 4010];
const RECOVERABLE_GATEWAY: &[i64] = &[4000, 4007, 4008, 4009, 4012, 4013];
const NO_AUTO_RECONNECT: &[i64] = &[4010, 4011];

fn shard_id_for_guild(guild: u64, num_shards: i64) -> i64 {
    ((guild >> 22) % num_shards as u64) as i64
}

fn recommended_shard_count(guild_count: i64, per_shard: i64) -> i64 {
    if guild_count <= 0 {
        return 1;
    }
    let needed = (guild_count + per_shard - 1) / per_shard;
    needed.clamp(1, MAX_SHARD_COUNT)
}

fn should_reconnect(code: i64) -> bool {
    if FATAL_CLOSE.contains(&code) || NO_AUTO_RECONNECT.contains(&code) {
        return false;
    }
    code < 4000 || RECOVERABLE_GATEWAY.contains(&code)
}

fn narrow_gateway_payload(value: &Value) -> Value {
    let Value::Object(map) = value else {
        return Value::Null;
    };
    let Some(op) = map.get("op").and_then(|v| v.as_f64()) else {
        return Value::Null;
    };
    let mut out = json!({ "op": op });
    if let Some(d) = map.get("d") {
        out["d"] = d.clone();
    }
    if let Some(s) = map.get("s").and_then(|v| v.as_f64()) {
        out["s"] = json!(s);
    }
    if let Some(t) = map.get("t").and_then(|v| v.as_str()) {
        out["t"] = json!(t);
    }
    out
}

/// `RateLimitManager` from `@fluxerjs/rest`.
#[derive(Default)]
struct RateLimits {
    buckets: HashMap<String, Bucket>,
    route_buckets: HashMap<String, String>,
    global_reset_at: i64,
}

#[derive(Clone, serde::Serialize)]
struct Bucket {
    limit: i64,
    remaining: i64,
    #[serde(rename = "resetAt")]
    reset_at: i64,
}

impl RateLimits {
    fn key(&mut self, route: &str) -> String {
        match self.route_buckets.get(route) {
            Some(k) => k.clone(),
            None => route.to_string(),
        }
    }

    fn set_bucket(&mut self, route: &str, limit: i64, remaining: i64, reset_at: i64, now: i64) {
        let key = self.key(route);
        self.buckets.insert(
            key,
            Bucket {
                limit,
                remaining,
                reset_at,
            },
        );
        self.prune(now);
    }

    fn wait_time(&mut self, route: &str, now: i64) -> i64 {
        if self.global_reset_at > 0 && self.global_reset_at <= now {
            self.global_reset_at = 0;
        }
        let global_wait = (self.global_reset_at - now).max(0);
        let key = self.key(route);
        match self.buckets.get(&key) {
            Some(bucket) if bucket.reset_at <= now => {
                self.buckets.remove(&key);
                global_wait
            }
            Some(bucket) if bucket.remaining <= 0 => global_wait.max(bucket.reset_at - now),
            _ => global_wait,
        }
    }

    fn update_from_headers(&mut self, route: &str, headers: &HashMap<String, String>, now: i64) {
        let get = |name: &str| {
            headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        };
        let (Some(limit), Some(remaining)) = (
            get("x-ratelimit-limit").and_then(|v| v.parse::<i64>().ok()),
            get("x-ratelimit-remaining").and_then(|v| v.parse::<i64>().ok()),
        ) else {
            return;
        };
        let reset = get("x-ratelimit-reset").and_then(|v| v.parse::<f64>().ok());
        let reset_after = get("x-ratelimit-reset-after").and_then(|v| v.parse::<f64>().ok());
        if reset.is_none() && reset_after.is_none() {
            return;
        }
        if let Some(bucket) = get("x-ratelimit-bucket") {
            let re =
                regex::Regex::new(r"/(channels|guilds|webhooks)/\d{17,19}(?:/|$)").expect("regex");
            let major = re
                .find(route)
                .map(|m| m.as_str().trim_end_matches('/').to_string())
                .unwrap_or_default();
            let origin_re = regex::Regex::new(r"^\S+ (https?://[^/]+)").expect("regex");
            let origin = origin_re
                .captures(route)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            self.route_buckets
                .insert(route.to_string(), format!("{origin}:{bucket}:{major}"));
        }
        let reset_at = match reset_after {
            Some(after) if after.is_finite() => now + (after.max(0.0) * 1000.0) as i64,
            _ => match reset {
                Some(r) if r > 1e12 => r as i64,
                Some(r) => (r * 1000.0) as i64,
                None => 0,
            },
        };
        self.set_bucket(route, limit, remaining, reset_at, now);
    }

    fn prune(&mut self, now: i64) {
        self.buckets.retain(|_, b| b.reset_at > now);
        while self.buckets.len() > 2000 {
            let Some(oldest) = self.buckets.keys().next().cloned() else {
                break;
            };
            self.buckets.remove(&oldest);
        }
        while self.route_buckets.len() > 2000 {
            let Some(oldest) = self.route_buckets.keys().next().cloned() else {
                break;
            };
            self.route_buckets.remove(&oldest);
        }
    }
}

fn option<'a>(map: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a Value> {
    map.get(key).filter(|v| !v.is_null())
}

pub fn register(d: &mut Definitions) {
    d.add(
        "botreadiness",
        "instance",
        0,
        0,
        "",
        "Returns ready, readyAt and uptimeMs; times in milliseconds, absent before connecting.",
        |c| {
            Box::pin(async move {
                // A running script means the dispatch reached the bot.
                Ok(json!({ "ready": c.ctx.bot_id.is_some(), "readyAt": Value::Null, "uptimeMs": Value::Null }))
            })
        },
    );

    d.add(
        "requirebotready",
        "guards",
        0,
        0,
        "",
        "Ends the execution if the bot is not ready yet.",
        |c| {
            Box::pin(async move {
                if c.ctx.bot_id.is_none() {
                    return Err(
                        GuardError::new("The bot is not ready yet.", GuardReason::Generic).into(),
                    );
                }
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "shardinfo",
        "gateway",
        0,
        0,
        "",
        "Returns id, local ids, count and isLeader; empty without shard context.",
        |_c| Box::pin(async move { Ok(Value::Null) }),
    );

    d.add(
        "shardidforguild",
        "gateway",
        0,
        2,
        "[guild?;shards?]",
        "Computes the shard of a guild; uses the current guild and the context total when omitted.",
        |c| {
            Box::pin(async move {
                let guild = if c.arg_count() > 0 {
                    snowflake(&c.arg(0).await?)?.get()
                } else {
                    c.ctx.require_guild()?.get()
                };
                if c.arg_count() > 1 {
                    let count = integer(&arg_value(c, 1).await?, 1, MAX_SHARD_COUNT)?;
                    return Ok(json!(shard_id_for_guild(guild, count)));
                }
                invalid("Enter the shard total when there is no shard context.")
            })
        },
    );

    d.add(
        "guildmatchesshard",
        "gateway",
        3,
        3,
        "[guild;shard;total]",
        "Checks whether the guild belongs to the given shard, without touching the gateway.",
        |c| {
            Box::pin(async move {
                let guild = snowflake(&c.arg(0).await?)?.get();
                let count = integer(&arg_value(c, 2).await?, 1, MAX_SHARD_COUNT)?;
                let shard = integer(&arg_value(c, 1).await?, 0, count - 1)?;
                Ok(json!(shard_id_for_guild(guild, count) == shard))
            })
        },
    );

    d.add(
        "recommendedshardcount",
        "gateway",
        1,
        2,
        "[guilds;guildsPerShard?]",
        "Computes the recommended shard total from the guild count.",
        |c| {
            Box::pin(async move {
                let guilds = integer(&arg_value(c, 0).await?, 0, i64::MAX)?;
                let per = if c.arg_count() > 1 {
                    integer(&arg_value(c, 1).await?, 1, MAX_GUILDS_PER_SHARD)?
                } else {
                    DEFAULT_GUILDS_PER_SHARD
                };
                Ok(json!(recommended_shard_count(guilds, per)))
            })
        },
    );

    d.add(
        "gatewayconstants",
        "gateway",
        0,
        0,
        "",
        "Returns public sharding constants, resume window and close codes.",
        |_c| {
            Box::pin(async move {
                let codes: serde_json::Map<String, Value> = CLOSE_CODES
                    .iter()
                    .map(|(k, v)| (k.to_string(), json!(*v)))
                    .collect();
                Ok(json!({
                    "defaultGuildsPerShard": DEFAULT_GUILDS_PER_SHARD,
                    "maxGuildsPerShard": MAX_GUILDS_PER_SHARD,
                    "maxShardCount": MAX_SHARD_COUNT,
                    "defaultResumeTtlMs": DEFAULT_RESUME_TTL_MS,
                    "closeCodes": Value::Object(codes),
                }))
            })
        },
    );

    d.add(
        "gatewayshouldreconnect",
        "gateway",
        1,
        1,
        "[code]",
        "Classifies a close code; never reconnects the bot.",
        |c| {
            Box::pin(async move {
                let code = integer(&arg_value(c, 0).await?, 1000, 4999)?;
                Ok(json!(should_reconnect(code)))
            })
        },
    );

    d.add(
        "gatewaysessionfresh",
        "gateway",
        1,
        3,
        "[updatedMs;ttlMs?;nowMs?]",
        "Compares timestamps only, with the resume window; does not touch credentials or an active session.",
        |c| {
            Box::pin(async move {
                let updated = integer(&arg_value(c, 0).await?, 0, i64::MAX)?;
                let ttl = if c.arg_count() > 1 {
                    integer(&arg_value(c, 1).await?, 0, i64::MAX)?
                } else {
                    DEFAULT_RESUME_TTL_MS
                };
                let now = if c.arg_count() > 2 {
                    integer(&arg_value(c, 2).await?, 0, i64::MAX)?
                } else {
                    (c.ctx.now)() as i64
                };
                Ok(json!(now - updated <= ttl))
            })
        },
    );

    d.add(
        "normalizegatewaypayload",
        "gateway",
        1,
        1,
        "[object]",
        "Normalizes the JSON envelope supplied by the script itself; does not read or send gateway payloads.",
        |c| {
            Box::pin(async move {
                let value = arg_value(c, 0).await?;
                Ok(narrow_gateway_payload(&value))
            })
        },
    );

    d.add(
        "gatewaystatus",
        "gateway",
        0,
        0,
        "",
        "Returns URL, count, status and latency of shards; bot owners only. Empty when disconnected.",
        |c| {
            Box::pin(async move {
                operator(c)?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "clusterinfo",
        "gateway",
        0,
        1,
        "[id?]",
        "Returns the total and up to 100 runtimes with ID and status, or filters by ID; bot owners only.",
        |c| {
            Box::pin(async move {
                operator(c)?;
                if c.arg_count() > 0 {
                    let id = c.arg(0).await?;
                    if id.is_empty() || id.len() > 128 {
                        return invalid("Enter a runtime ID of 1 to 128 characters.");
                    }
                }
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "restdefaults",
        "gateway",
        0,
        0,
        "",
        "Returns the SDK default API, version, locale and user agent; sends no request.",
        |_c| {
            Box::pin(async move {
                Ok(json!({
                    "api": "https://api.fluxer.app",
                    "version": "1",
                    "locale": "en-US",
                    "userAgent": "fluxerjs (https://github.com/fluxerjs/core)",
                }))
            })
        },
    );

    d.add(
        "restratelimits",
        "gateway",
        1,
        1,
        "[operations]",
        "Simulates up to 20 operations setBucket/getBucket/setGlobalReset/getGlobalReset/getWaitTime/updateFromHeaders/prune/size; does not touch the bot's limits.",
        |c| {
            Box::pin(async move {
                let operations = arg_value(c, 0).await?;
                let Value::Array(items) = operations else {
                    return invalid("Enter 1 to 20 operations.");
                };
                if items.is_empty() || items.len() > 20 {
                    return invalid("Enter 1 to 20 operations.");
                }
                let mut manager = RateLimits::default();
                let now = (c.ctx.now)() as i64;
                let mut results = Vec::new();
                for raw in items {
                    let Value::Object(map) = &raw else {
                        return invalid("Each operation must be an object.");
                    };
                    let route = |m: &serde_json::Map<String, Value>| -> FnResult<String> {
                        match option(m, "route") {
                            Some(Value::String(s)) if !s.is_empty() && s.len() <= 256 => Ok(s.clone()),
                            _ => invalid("Enter a route of 1 to 256 characters."),
                        }
                    };
                    let time = |m: &serde_json::Map<String, Value>| -> FnResult<i64> {
                        integer(option(m, "resetAt").unwrap_or(&Value::Null), 0, i64::MAX)
                    };
                    let op = option(map, "op").and_then(|v| v.as_str()).unwrap_or_default();
                    results.push(match op {
                        "setBucket" => {
                            let r = route(map)?;
                            manager.set_bucket(
                                &r,
                                integer(option(map, "limit").unwrap_or(&Value::Null), 0, 1_000_000)?,
                                integer(option(map, "remaining").unwrap_or(&Value::Null), 0, 1_000_000)?,
                                time(map)?,
                                now,
                            );
                            Value::Null
                        }
                        "getBucket" => {
                            let r = route(map)?;
                            let key = manager.key(&r);
                            match manager.buckets.get(&key).cloned() {
                                Some(b) => json!(b),
                                None => Value::Null,
                            }
                        }
                        "setGlobalReset" => {
                            manager.global_reset_at = time(map)?;
                            Value::Null
                        }
                        "getGlobalReset" => json!(manager.global_reset_at),
                        "getWaitTime" => {
                            let r = route(map)?;
                            json!(manager.wait_time(&r, now))
                        }
                        "updateFromHeaders" => {
                            let r = route(map)?;
                            let Some(Value::Object(headers)) = option(map, "headers") else {
                                return invalid("Enter the rate limit headers.");
                            };
                            let mut parsed = HashMap::new();
                            for (key, value) in headers {
                                let valid = matches!(
                                    key.to_ascii_lowercase().as_str(),
                                    "x-ratelimit-bucket"
                                        | "x-ratelimit-limit"
                                        | "x-ratelimit-remaining"
                                        | "x-ratelimit-reset"
                                        | "x-ratelimit-reset-after"
                                ) && value
                                    .as_str()
                                    .is_some_and(|v| !v.is_empty() && v.len() <= 128 && v.is_ascii());
                                if !valid {
                                    return invalid("Invalid rate limit header.");
                                }
                                parsed.insert(key.clone(), value.as_str().unwrap().to_string());
                            }
                            manager.update_from_headers(&r, &parsed, now);
                            Value::Null
                        }
                        "prune" => {
                            manager.prune(now);
                            Value::Null
                        }
                        "size" => json!(manager.buckets.len()),
                        _ => return invalid("Unknown rate limit operation."),
                    });
                }
                Ok(Value::Array(results))
            })
        },
    );
}
