use std::collections::HashMap;
use std::sync::Arc;

use flush_core::Message;
use flush_functions::{builtins::register_builtins, define_functions, ScriptContext};
use flush_functions::{Limits, Runtime, DEFAULT_LIMITS};
use flush_rest::{RestClient, RestConfig};

async fn ctx() -> Arc<ScriptContext> {
    let message: Message = serde_json::from_value(serde_json::json!({
        "id": "111",
        "channel_id": "222",
        "guild_id": "333",
        "author": {"id": "444", "username": "tester"},
        "content": "!say hi"
    }))
    .unwrap();
    let rest = RestClient::new(RestConfig {
        token: "test-token".into(),
        api_base: Some("http://127.0.0.1:9".into()),
        ..RestConfig::new("test-token")
    })
    .await
    .unwrap();
    Arc::new(ScriptContext {
        message,
        event: None,
        actor: None,
        prefix: "!".into(),
        input: "hi".into(),
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
        args: HashMap::new(),
        ping_ms: None,
        help: None,
        owners: vec![],
        dev_guilds: vec![],
        variables: None,
        cases: None,
        scheduler: None,
        cooldowns: None,
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

#[async_std::test]
async fn text_passthrough() {
    let out = runtime().run(&ctx().await, "hello world").await.unwrap();
    assert_eq!(out, "hello world");
}

#[async_std::test]
async fn escapes() {
    let out = runtime()
        .run(&ctx().await, r"\$let \[ \] \;")
        .await
        .unwrap();
    assert_eq!(out, "$let [ ] ;");
}

#[async_std::test]
async fn let_and_get() {
    let out = runtime()
        .run(&ctx().await, "$let[a;42]$get[a]")
        .await
        .unwrap();
    assert_eq!(out, "42");
}

#[async_std::test]
async fn unknown_function() {
    let err = runtime().run(&ctx().await, "$nope[]").await.unwrap_err();
    assert_eq!(err.code(), "UNKNOWN_FUNCTION");
}

#[async_std::test]
async fn call_limit() {
    let rt = Runtime::with_limits(Limits {
        calls: 3,
        ..DEFAULT_LIMITS
    });
    rt.register_all(define_functions(register_builtins))
        .unwrap();
    // `$let[a;$let[b;$get[a]]]` chains 3 calls; one more tips over.
    let err = rt
        .run(&ctx().await, "$let[a;$let[b;$get[a]]]$let[c;1]")
        .await;
    assert!(err.is_err());
}

#[async_std::test]
async fn depth_limit() {
    let rt = Runtime::with_limits(Limits {
        depth: 2,
        ..DEFAULT_LIMITS
    });
    rt.register_all(define_functions(register_builtins))
        .unwrap();
    let err = rt.run(&ctx().await, "$get[$get[$get[$get[a]]]]").await;
    assert!(err.is_err());
}
