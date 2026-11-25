//! Entity/native builtins: argument validation, guard paths and a few
//! representative REST calls against a loopback HTTP server (no external
//! network).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_std::io::prelude::*;
use async_std::net::{TcpListener, TcpStream};
use async_std::task;
use serde_json::{json, Value};

use flush_core::Message;
use flush_functions::Runtime;
use flush_functions::{builtins::register_builtins, define_functions, ScriptContext};
use flush_rest::{RestClient, RestConfig};

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
}

type Log = Arc<Mutex<Vec<Seen>>>;
type Responder = Arc<dyn Fn(&Seen) -> (u16, Value) + Send + Sync>;

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
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
        let method = first.next().unwrap_or("").to_owned();
        let path = first.next().unwrap_or("").to_owned();
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
        let seen = Seen { method, path };
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

async fn start(respond: Responder) -> (String, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::default();
    let log2 = log.clone();
    task::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            task::spawn(serve(stream, log2.clone(), respond.clone()));
        }
    });
    (base, log)
}

async fn ctx(api_base: Option<String>, dm: bool) -> Arc<ScriptContext> {
    let mut message = json!({
        "id": "111",
        "channel_id": "222",
        "author": {"id": "444", "username": "tester"},
        "content": "!test"
    });
    if !dm {
        message["guild_id"] = json!("333");
    }
    let message: Message = serde_json::from_value(message).unwrap();
    let rest = RestClient::new(RestConfig {
        token: "test-token".into(),
        api_base: api_base.or(Some("http://127.0.0.1:9".into())),
        gateway_url: Some("ws://unused".into()),
        ..RestConfig::new("test-token")
    })
    .await
    .unwrap();
    Arc::new(ScriptContext {
        message,
        event: None,
        actor: None,
        prefix: "!".into(),
        input: "test".into(),
        rest: Arc::new(rest),
        bot_id: Some("555".parse().unwrap()),
        guild: None,
        channel: None,
        member: None,
        bot_member: None,
        vars: std::sync::Mutex::new(HashMap::new()),
        loop_frame: std::sync::atomic::AtomicU64::new(0),
        aborted: std::sync::atomic::AtomicBool::new(false),
        execution_id: std::sync::Mutex::new(None),
        last_case: std::sync::Mutex::new(None),
        moderation_applied: std::sync::atomic::AtomicBool::new(false),
        emoji: HashMap::new(),
        owners: vec![],
        dev_guilds: vec![],
        variables: None,
        cases: None,
        scheduler: None,
        collectors: None,
        now: Box::new(|| 0),
    })
}

fn runtime() -> Runtime {
    let rt = Runtime::new();
    rt.register_all(define_functions(register_builtins))
        .unwrap();
    rt
}

async fn run(source: &str, api_base: Option<String>, dm: bool) -> Result<String, String> {
    runtime()
        .run(&ctx(api_base, dm).await, source)
        .await
        .map_err(|e| e.code().to_owned())
}

// --- argument validation ---------------------------------------------------

#[async_std::test]
async fn missing_args_error() {
    for source in [
        "$channel[]",
        "$fetchMessage[222]",
        "$messages[]",
        "$removeReaction[333]",
        "$user[]",
        "$member[]",
        "$role[]",
        "$inviteInfo[]",
    ] {
        assert!(
            run(source, None, false).await.is_err(),
            "expected error for {source}"
        );
    }
}

#[async_std::test]
async fn invalid_ids_error() {
    for source in ["$channel[abc]", "$fetchMessage[222;nope]", "$user[@@@]"] {
        assert!(
            run(source, None, false).await.is_err(),
            "expected error for {source}"
        );
    }
}

// --- guard paths -------------------------------------------------------------

#[async_std::test]
async fn guild_only_functions_fail_in_dm() {
    for source in [
        "$bans[]",
        "$members[]",
        "$roles[]",
        "$emojis[]",
        "$createChannel[general]",
    ] {
        let err = run(source, None, true).await;
        assert!(err.is_err(), "expected guard error for {source}");
    }
}

#[async_std::test]
async fn collectors_without_collectors_error() {
    // ScriptContext.collectors is None here: awaitMessage/awaitReaction/confirm
    // must surface a native-unavailable error rather than hang.
    for source in ["$awaitMessage[1]", "$awaitReaction[1]", "$confirm[1]"] {
        assert!(
            run(source, None, false).await.is_err(),
            "expected error for {source}"
        );
    }
}

// --- loopback REST -----------------------------------------------------------

fn channel_body() -> Value {
    json!({"id": "222", "type": 0, "guild_id": "333", "name": "general"})
}

fn message_body() -> Value {
    json!({
        "id": "333",
        "channel_id": "222",
        "guild_id": "333",
        "author": {"id": "444", "username": "tester"},
        "content": "hi"
    })
}

#[async_std::test]
async fn channel_fetches_and_returns_property() {
    let (base, log) = start(Arc::new(|_| (200, channel_body()))).await;
    let out = run("$channel[222;id]", Some(base), false).await.unwrap();
    assert_eq!(out, "222");
    let seen = &log.lock().unwrap()[0];
    assert_eq!(
        (seen.method.as_str(), seen.path.as_str()),
        ("GET", "/v1/channels/222")
    );
}

#[async_std::test]
async fn channel_not_found_returns_empty() {
    let (base, _log) = start(Arc::new(|_| {
        (
            404,
            json!({"code": "UNKNOWN_CHANNEL", "message": "not found"}),
        )
    }))
    .await;
    let out = run("$channel[999]", Some(base), false).await.unwrap();
    assert_eq!(out, "");
}

#[async_std::test]
async fn fetch_message_round_trips() {
    let (base, log) = start(Arc::new(|_| (200, message_body()))).await;
    let out = run("$fetchMessage[222;333;content]", Some(base), false)
        .await
        .unwrap();
    assert_eq!(out, "hi");
    let seen = &log.lock().unwrap()[0];
    assert_eq!(
        (seen.method.as_str(), seen.path.as_str()),
        ("GET", "/v1/channels/222/messages/333")
    );
}

#[async_std::test]
async fn pinned_messages_lists() {
    let (base, log) = start(Arc::new(|_| {
        (
            200,
            json!([
                message_body(),
                json!({
                    "id": "334",
                    "channel_id": "222",
                    "author": {"id": "1", "username": "other"},
                    "content": "pinned"
                })
            ]),
        )
    }))
    .await;
    let out = run("$pinnedMessages[222]", Some(base), false)
        .await
        .unwrap();
    assert!(out.contains("334"), "unexpected output: {out}");
    let seen = &log.lock().unwrap()[0];
    assert_eq!(
        (seen.method.as_str(), seen.path.as_str()),
        ("GET", "/v1/channels/222/messages/pins")
    );
}
