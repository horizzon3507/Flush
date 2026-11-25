//! Resource/payload/embed builtins: normalization, validation and loopback REST.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_std::io::prelude::*;
use async_std::net::{TcpListener, TcpStream};
use async_std::task;
use serde_json::{json, Value};

use flush_core::Message;
use flush_functions::{builtins::register_builtins, define_functions, Runtime, ScriptContext};
use flush_rest::{RestClient, RestConfig};

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    body: Vec<u8>,
}

impl Seen {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

type Log = Arc<Mutex<Vec<Seen>>>;
type Responder = Arc<dyn Fn(&Seen) -> (u16, Value) + Send + Sync>;

fn find(buf: &[u8], needle: &[u8]) -> Option<usize> {
    buf.windows(needle.len()).position(|w| w == needle)
}

async fn serve(mut stream: TcpStream, log: Log, respond: Responder) {
    let mut buf = Vec::new();
    loop {
        let head_end = loop {
            if let Some(i) = find(&buf, b"\r\n\r\n") {
                break i;
            }
            let mut chunk = [0u8; 8192];
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let mut lines = head.split("\r\n");
        let mut first = lines.next().unwrap_or("").split(' ');
        let mut seen = Seen {
            method: first.next().unwrap_or("").to_owned(),
            path: first.next().unwrap_or("").to_owned(),
            body: Vec::new(),
        };
        let mut length = 0usize;
        for line in lines {
            let (k, v) = line.split_once(':').unwrap_or((line, ""));
            if k.eq_ignore_ascii_case("content-length") {
                length = v.trim().parse().unwrap_or(0);
            }
        }
        let start = head_end + 4;
        while buf.len() < start + length {
            let mut chunk = [0u8; 8192];
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        seen.body = buf[start..start + length].to_vec();
        buf.drain(..start + length);
        let (status, value) = respond(&seen);
        log.lock().unwrap().push(seen);
        let text = if value.is_null() {
            String::new()
        } else {
            value.to_string()
        };
        let out = format!(
            "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{text}",
            text.len()
        );
        if stream.write_all(out.as_bytes()).await.is_err() {
            return;
        }
    }
}

async fn ctx_with(rest: RestClient) -> Arc<ScriptContext> {
    ctx_full(rest, None, None, None, vec![]).await
}

async fn ctx_full(
    rest: RestClient,
    guild: Option<flush_core::Guild>,
    member: Option<flush_core::MemberUser>,
    bot_member: Option<flush_core::MemberUser>,
    owners: Vec<flush_core::Snowflake>,
) -> Arc<ScriptContext> {
    let message: Message = serde_json::from_value(json!({
        "id": "111", "channel_id": "222", "guild_id": "333",
        "author": {"id": "444", "username": "tester"},
        "content": "!x"
    }))
    .unwrap();
    Arc::new(ScriptContext {
        message,
        event: None,
        actor: None,
        prefix: "!".into(),
        input: "x".into(),
        rest: Arc::new(rest),
        bot_id: Some("555".parse().unwrap()),
        guild,
        channel: None,
        member,
        bot_member,
        vars: Mutex::new(HashMap::new()),
        loop_frame: std::sync::atomic::AtomicU64::new(0),
        aborted: std::sync::atomic::AtomicBool::new(false),
        execution_id: Mutex::new(None),
        last_case: Mutex::new(None),
        moderation_applied: std::sync::atomic::AtomicBool::new(false),
        emoji: HashMap::new(),
        owners,
        dev_guilds: vec![],
        variables: None,
        cases: None,
        scheduler: None,
        collectors: None,
        now: Box::new(|| 0),
    })
}

/// `ctx()` + `runtime()` wired to a loopback REST server. `respond` answers
/// `GET /v1/ping` by default so `RestClient::discover` always succeeds.
async fn rig(respond: Responder) -> (Runtime, Arc<ScriptContext>, Log, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::default();
    let log2 = log.clone();
    task::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            task::spawn(serve(stream, log2.clone(), respond.clone()));
        }
    });
    let mut config = RestConfig::new("tok");
    config.api_base = Some(base.clone());
    config.gateway_url = Some("ws://unused".into());
    let rest = RestClient::new(config).await.unwrap();
    let rt = Runtime::new();
    rt.register_all(define_functions(register_builtins))
        .unwrap();
    (rt, ctx_with(rest).await, log, base)
}

fn responder(f: impl Fn(&Seen) -> (u16, Value) + Send + Sync + 'static) -> Responder {
    Arc::new(f)
}

#[async_std::test]
async fn reply_sends_content_with_no_ping_mentions() {
    let (rt, ctx, log, _base) = rig(responder(|_| {
        (
            200,
            json!({"id": "9", "channel_id": "222", "author": {"id": "1", "username": "b"}}),
        )
    }))
    .await;
    rt.run(&ctx, "$reply[hi there]").await.unwrap();
    let seen = log.lock().unwrap()[0].json();
    assert_eq!(seen["content"], "hi there");
    assert_eq!(seen["allowed_mentions"]["parse"], json!([]));
    assert_eq!(seen["allowed_mentions"]["replied_user"], false);
    assert_eq!(log.lock().unwrap()[0].path, "/v1/channels/222/messages");
}

#[async_std::test]
async fn sendmessage_returns_id() {
    let (rt, ctx, _log, _base) = rig(responder(|_| {
        (
            200,
            json!({"id": "77", "channel_id": "222", "author": {"id": "1", "username": "b"}}),
        )
    }))
    .await;
    let out = rt.run(&ctx, "$sendMessage[222;yo]").await.unwrap();
    assert_eq!(out, "77");
}

#[async_std::test]
async fn reply_with_embed_object() {
    let (rt, ctx, log, _base) = rig(responder(|_| {
        (
            200,
            json!({"id": "9", "channel_id": "222", "author": {"id": "1", "username": "b"}}),
        )
    }))
    .await;
    rt.run(
        &ctx,
        "$reply[$embed[$title[Hello];$description[Body];$color[red]]]",
    )
    .await
    .unwrap();
    let seen = log.lock().unwrap()[0].json();
    assert_eq!(seen["embeds"][0]["title"], "Hello");
    assert_eq!(seen["embeds"][0]["description"], "Body");
    assert_eq!(seen["embeds"][0]["color"], 0xE74C3C);
}

#[async_std::test]
async fn embed_scalar_and_field_parts() {
    let (rt, ctx, log, _base) = rig(responder(|_| {
        (
            200,
            json!({"id": "9", "channel_id": "222", "author": {"id": "1", "username": "b"}}),
        )
    }))
    .await;
    rt.run(
        &ctx,
        "$reply[$embed[$author[Ana];$field[n;v;true]; $footer[f text]]]",
    )
    .await
    .unwrap();
    let embed = &log.lock().unwrap()[0].json()["embeds"][0];
    assert_eq!(embed["author"]["name"], "Ana");
    assert_eq!(embed["footer"]["text"], "f text");
    assert_eq!(
        embed["fields"][0],
        json!({"name": "n", "value": "v", "inline": true})
    );
}

#[async_std::test]
async fn payload_requires_content_or_embeds() {
    let (rt, ctx, _log, _base) = rig(responder(|_| (200, json!({})))).await;
    let err = rt.run(&ctx, "$reply[{ \"flags\": 4 }]").await.unwrap_err();
    assert_eq!(err.code(), "MESSAGE_LENGTH");
}

#[async_std::test]
async fn payload_rejects_unknown_message_key() {
    let (rt, ctx, _log, _base) = rig(responder(|_| (200, json!({})))).await;
    let err = rt
        .run(&ctx, "$reply[{ \"content\": \"x\", \"mystery\": 1 }]")
        .await
        .unwrap_err();
    assert_eq!(err.code(), "MESSAGE_KEY");
}

#[async_std::test]
async fn payload_rejects_overlong_text() {
    let (rt, ctx, _log, _base) = rig(responder(|_| (200, json!({})))).await;
    let long = "x".repeat(2001);
    let err = rt
        .run(&ctx, &format!("$reply[{{ \"content\": \"{long}\" }}]"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "MESSAGE_LENGTH");
}

#[async_std::test]
async fn embed_rejects_non_object_part() {
    let (rt, ctx, _log, _base) = rig(responder(|_| (200, json!({})))).await;
    let err = rt.run(&ctx, "$reply[$embed[42]]").await.unwrap_err();
    assert_eq!(err.code(), "EMBED");
}

#[async_std::test]
async fn embed_validates_color() {
    let (rt, ctx, _log, _base) = rig(responder(|_| (200, json!({})))).await;
    let err = rt
        .run(&ctx, "$reply[$embed[$color[notacolor]]]")
        .await
        .unwrap_err();
    assert_eq!(err.code(), "COLOR");
}

#[async_std::test]
async fn embed_validates_url() {
    let (rt, ctx, _log, _base) = rig(responder(|_| (200, json!({})))).await;
    let err = rt
        .run(&ctx, "$reply[$embed[$image[ftp://x]]]")
        .await
        .unwrap_err();
    assert_eq!(err.code(), "EMBED_URL");
}

#[async_std::test]
async fn embed_limits_fields_to_25() {
    let (rt, ctx, _log, _base) = rig(responder(|_| (200, json!({})))).await;
    let parts = (0..26)
        .map(|i| format!("$field[n{i};v]"))
        .collect::<Vec<_>>()
        .join(";");
    let err = rt
        .run(&ctx, &format!("$reply[$embed[{parts}]]"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "EMBED_LIMIT");
}

#[async_std::test]
async fn field_index_out_of_range() {
    let (rt, ctx, _log, _base) = rig(responder(|_| (200, json!({})))).await;
    let err = rt
        .run(&ctx, "$reply[$removeField[$embed[$field[a;b]];9]]")
        .await
        .unwrap_err();
    assert_eq!(err.code(), "EMBED_FIELD");
}

#[async_std::test]
async fn embedfromobject_normalizes_json_text() {
    let (rt, ctx, log, _base) = rig(responder(|_| {
        (
            200,
            json!({"id": "9", "channel_id": "222", "author": {"id": "1", "username": "b"}}),
        )
    }))
    .await;
    rt.run(
        &ctx,
        r##"$reply[$embedFromObject[{"title":"T","color":"#00ff00","fields":[{"name":"a","value":"b"}]}]]"##,
    )
    .await
    .unwrap();
    let embed = &log.lock().unwrap()[0].json()["embeds"][0];
    assert_eq!(embed["title"], "T");
    assert_eq!(embed["color"], 0x00FF00);
    assert_eq!(embed["fields"][0]["name"], "a");
}

#[async_std::test]
async fn messageoptions_rejects_bad_tts_and_parse() {
    let (rt, ctx, _log, _base) = rig(responder(|_| (200, json!({})))).await;
    let err = rt.run(&ctx, r#"$messageOptions[;;1.5]"#).await.unwrap_err();
    assert_eq!(err.code(), "MESSAGE_OPTION");
    let err = rt
        .run(
            &ctx,
            r#"$sendMessage[222;{"content":"x","allowedMentions":{"parse":["nobody"]}}]"#,
        )
        .await
        .unwrap_err();
    assert_eq!(err.code(), "MESSAGE_OPTION");
}

#[async_std::test]
async fn messageoptions_emits_wire_shape() {
    let (rt, ctx, _log, _base) = rig(responder(|_| (200, json!({})))).await;
    let out = rt.run(&ctx, r#"$messageOptions[;123;4]"#).await.unwrap();
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["stickerIds"], json!(["123"]));
    assert_eq!(v["flags"], 4);
}

#[async_std::test]
async fn gateway_constants_and_shard_math() {
    let (rt, ctx, _log, _base) = rig(responder(|_| (200, json!({})))).await;
    let out = rt
        .run(&ctx, "$recommendedShardCount[100000]")
        .await
        .unwrap();
    assert_eq!(out, "67");
    let out = rt.run(&ctx, "$gatewayShouldReconnect[4004]").await.unwrap();
    assert_eq!(out, "false");
    let out = rt.run(&ctx, "$gatewayShouldReconnect[4009]").await.unwrap();
    assert_eq!(out, "true");
    let out = rt.run(&ctx, "$shardIdForGuild[1234567890123;8]").await.unwrap();
    assert_eq!(out, ((1234567890123u64 >> 22) % 8).to_string());
}

#[async_std::test]
async fn rate_limits_simulator() {
    let (rt, ctx, _log, _base) = rig(responder(|_| (200, json!({})))).await;
    let out = rt
        .run(
            &ctx,
            r#"$restRateLimits[[{"op":"setBucket","route":"r","limit":1,"remaining":0,"resetAt":90000},{"op":"getWaitTime","route":"r"},{"op":"size"}]]"#,
        )
        .await
        .unwrap();
    assert_eq!(out, "[null,90000,1]");
    let err = rt
        .run(&ctx, r#"$restRateLimits[[{"op":"nope"}]]"#)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "GUARD");
}

#[async_std::test]
async fn webhook_message_edit_and_delete() {
    let (rt, _ctx, log, base) = rig(responder(|seen| {
        if seen.path.starts_with("/v1/webhooks/") && seen.method == "GET" {
            (
                200,
                json!({"id": "55", "token": "sekret", "channel_id": "222", "type": 1}),
            )
        } else if seen.method == "PATCH" {
            (
                200,
                json!({"id": "66", "channel_id": "222", "author": {"id": "1", "username": "b"}}),
            )
        } else {
            (200, json!({}))
        }
    }))
    .await;
    // A guild snapshot with an all-permissions role held by author and bot.
    let admin = flush_core::Permissions::ADMINISTRATOR.to_string();
    let guild: flush_core::Guild = serde_json::from_value(json!({
        "id": "333", "name": "g", "owner_id": "900",
        "roles": [{"id": "9", "position": 1, "permissions": admin}]
    }))
    .unwrap();
    let member: flush_core::MemberUser = serde_json::from_value(json!({
        "user": {"id": "444", "username": "tester"}, "roles": ["9"]
    }))
    .unwrap();
    let bot_member: flush_core::MemberUser = serde_json::from_value(json!({
        "user": {"id": "555", "username": "bot"}, "roles": ["9"]
    }))
    .unwrap();
    // Rebuild ctx against a second loopback REST client sharing the server.
    let mut config = RestConfig::new("tok");
    config.api_base = Some(base);
    config.gateway_url = Some("ws://unused".into());
    let rest = RestClient::new(config).await.unwrap();
    let ctx = ctx_full(rest, Some(guild), Some(member), Some(bot_member), vec![]).await;
    rt.run(&ctx, "$editWebhookMessage[55;66;new body]")
        .await
        .unwrap();
    let calls = log.lock().unwrap().clone();
    assert!(calls
        .iter()
        .any(|s| s.method == "GET" && s.path == "/v1/webhooks/55"));
    assert!(calls
        .iter()
        .any(|s| s.method == "PATCH" && s.path.contains("/v1/webhooks/55/sekret/messages/66")));
    rt.run(&ctx, "$deleteWebhookMessage[55;66]").await.unwrap();
    let calls = log.lock().unwrap().clone();
    assert!(calls
        .iter()
        .any(|s| s.method == "DELETE" && s.path.contains("/v1/webhooks/55/sekret/messages/66")));
}

#[async_std::test]
async fn owner_gates_instance_functions() {
    let (rt, ctx, _log, _base) = rig(responder(|_| (200, json!({})))).await;
    let err = rt.run(&ctx, "$gatewayStatus[]").await.unwrap_err();
    assert_eq!(err.code(), "GUARD");
    let err = rt.run(&ctx, "$clusterInfo[]").await.unwrap_err();
    assert_eq!(err.code(), "GUARD");
}
