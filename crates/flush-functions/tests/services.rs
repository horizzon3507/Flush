use std::collections::HashMap;
use std::sync::Arc;

use flush_core::Message;
use flush_framework::MemoryVariableStore;
use flush_functions::context::Variables;
use flush_functions::{builtins::register_builtins, define_functions, ScriptContext};
use flush_functions::{Runtime, ScriptFailure};
use flush_rest::{RestClient, RestConfig};

async fn base_ctx() -> ScriptContext {
    let message: Message = serde_json::from_value(serde_json::json!({
        "id": "111",
        "channel_id": "222",
        "guild_id": "333",
        "author": {"id": "444", "username": "tester"},
        "content": "!say hi there"
    }))
    .unwrap();
    let rest = RestClient::new(RestConfig {
        token: "test-token".into(),
        api_base: Some("http://127.0.0.1:9".into()),
        ..RestConfig::new("test-token")
    })
    .await
    .unwrap();
    ScriptContext {
        message,
        event: None,
        actor: None,
        prefix: "!".into(),
        input: "hi there".into(),
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
    }
}

async fn ctx() -> Arc<ScriptContext> {
    Arc::new(base_ctx().await)
}

/// Context backed by an in-memory `VariableStore`.
async fn vars_ctx() -> Arc<ScriptContext> {
    let mut ctx = base_ctx().await;
    let store: Arc<dyn flush_framework::VariableStore + Send + Sync> =
        Arc::new(MemoryVariableStore::new());
    ctx.variables = Some(Arc::new(Variables::new(store, Some("333"), Some("444"))));
    Arc::new(ctx)
}

fn runtime() -> Runtime {
    let rt = Runtime::new();
    rt.register_all(define_functions(register_builtins))
        .unwrap();
    rt
}

fn guard_code(err: &ScriptFailure) -> &'static str {
    err.code()
}

#[async_std::test]
async fn context_builtins() {
    let ctx = ctx().await;
    let rt = runtime();
    assert_eq!(rt.run(&ctx, "$authorID").await.unwrap(), "444");
    assert_eq!(rt.run(&ctx, "$authorName").await.unwrap(), "tester");
    assert_eq!(rt.run(&ctx, "$username").await.unwrap(), "tester");
    assert_eq!(rt.run(&ctx, "$botID").await.unwrap(), "555");
    assert_eq!(rt.run(&ctx, "$commandName").await.unwrap(), "say");
    assert_eq!(rt.run(&ctx, "$message").await.unwrap(), "hi there");
    assert_eq!(rt.run(&ctx, "$message[2]").await.unwrap(), "there");
    assert_eq!(rt.run(&ctx, "$messageSlice[1]").await.unwrap(), "there");
    assert_eq!(rt.run(&ctx, "$tokens").await.unwrap(), r#"["hi","there"]"#);
}

#[async_std::test]
async fn flow_if() {
    let ctx = ctx().await;
    let rt = runtime();
    assert_eq!(rt.run(&ctx, "$if[true;yes;no]").await.unwrap(), "yes");
    assert_eq!(rt.run(&ctx, "$if[false;yes;no]").await.unwrap(), "no");
    assert_eq!(rt.run(&ctx, "$if[1;yes;no]").await.unwrap(), "yes");
    assert_eq!(rt.run(&ctx, "$if[0;yes]").await.unwrap(), "");
}

#[async_std::test]
async fn flow_try_catches_fn_errors() {
    let ctx = ctx().await;
    let rt = runtime();
    assert_eq!(
        rt.run(&ctx, "$try[$throw[boom];caught]").await.unwrap(),
        "caught"
    );
}

#[async_std::test]
async fn flow_try_propagates_uncatchable() {
    let ctx = ctx().await;
    let rt = runtime();
    // A guard failure inside `$try` must not be swallowed.
    let err = rt
        .run(&ctx, "$try[$onlyIf[false;ok];fallback]")
        .await
        .unwrap_err();
    assert_eq!(guard_code(&err), "GUARD");
    // `$stop` is uncatchable too.
    let err = rt.run(&ctx, "$try[$stop[];fallback]").await.unwrap_err();
    assert_eq!(guard_code(&err), "STOP");
}

#[async_std::test]
async fn guards() {
    let ctx = ctx().await;
    let rt = runtime();
    assert_eq!(rt.run(&ctx, "$onlyIf[true;ok]").await.unwrap(), "");
    let err = rt.run(&ctx, "$onlyIf[false;blocked]").await.unwrap_err();
    assert_eq!(guard_code(&err), "GUARD");
    let err = rt.run(&ctx, "$require[false;blocked]").await.unwrap_err();
    assert_eq!(guard_code(&err), "GUARD");
    assert_eq!(rt.run(&ctx, "$requireGuild[]").await.unwrap(), "");
    // Missing member permission bits (perm name unknown to this context).
    assert!(rt.run(&ctx, "$onlyPerms[administrator]").await.is_err());
}

#[async_std::test]
async fn storage_vars() {
    let ctx = vars_ctx().await;
    let rt = runtime();
    assert_eq!(rt.run(&ctx, "$setUserVar[a;42]").await.unwrap(), "");
    assert_eq!(rt.run(&ctx, "$getUserVar[a]").await.unwrap(), "42");
    assert_eq!(rt.run(&ctx, "$getUserVar[missing;fb]").await.unwrap(), "fb");
    assert_eq!(rt.run(&ctx, "$getUserVar[missing]").await.unwrap(), "");
    assert_eq!(rt.run(&ctx, "$deleteUserVar[a]").await.unwrap(), "");
    assert_eq!(rt.run(&ctx, "$getUserVar[a;gone]").await.unwrap(), "gone");

    assert_eq!(rt.run(&ctx, "$setVar[g;7]").await.unwrap(), "");
    assert_eq!(rt.run(&ctx, "$getVar[g]").await.unwrap(), "7");

    // JSON values round-trip.
    assert_eq!(rt.run(&ctx, "$setUserVar[o;{\"k\":1}]").await.unwrap(), "");
    assert_eq!(rt.run(&ctx, "$getUserVar[o]").await.unwrap(), r#"{"k":1}"#);
}

#[async_std::test]
async fn storage_absent_store() {
    let ctx = ctx().await; // variables: None
    let rt = runtime();
    let err = rt.run(&ctx, "$getUserVar[a]").await.unwrap_err();
    assert_eq!(guard_code(&err), "NATIVE_UNAVAILABLE");
}

#[async_std::test]
async fn deferred_providers_unavailable() {
    let ctx = ctx().await;
    let rt = runtime();
    for call in [
        "$musicPlay[x]",
        "$voiceJoin[123]",
        "$canvasCreate[cv;10;10]",
    ] {
        let err = rt.run(&ctx, call).await.unwrap_err();
        assert_eq!(guard_code(&err), "NATIVE_UNAVAILABLE", "{call}");
    }
}

#[async_std::test]
async fn instance_value() {
    let ctx = ctx().await;
    let rt = runtime();

    // normalize strips a `/vN` suffix and trailing slashes.
    assert_eq!(
        rt.run(&ctx, "$instanceValue[normalize;https://ex.com/api/v1/]")
            .await
            .unwrap(),
        "https://ex.com/api"
    );
    assert_eq!(
        rt.run(&ctx, "$instanceValue[discoveryURL;https://ex.com/api/v2]")
            .await
            .unwrap(),
        "https://ex.com/api/.well-known/fluxer"
    );

    // resolve with no document yields the default endpoints.
    let out = rt.run(&ctx, "$instanceValue[resolve]").await.unwrap();
    let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["endpoints"]["invite"], "https://fluxer.gg");
    assert_eq!(doc["discovery"], serde_json::Value::Null);

    // resolve merges a partial map and back-fills api_public from api.
    let out = rt
        .run(
            &ctx,
            "$instanceValue[resolve;{\"api\":\"https://x.io/api/v3\"}]",
        )
        .await
        .unwrap();
    let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["endpoints"]["api"], "https://x.io/api");
    assert_eq!(doc["endpoints"]["api_public"], "https://x.io/api");
    assert_eq!(doc["endpoints"]["gateway"], "wss://gateway.fluxer.app");

    // parse validates the document and fills the optional sections.
    let full = "{\"api_code_version\":1,\"endpoints\":{\"api\":\"https://a.io/api\",\"api_client\":\"https://a.io/api\",\"api_public\":\"https://api.a.io\",\"gateway\":\"wss://gw.a.io\",\"media\":\"https://cdn.a.io/\",\"static_cdn\":\"https://s.a.io\",\"marketing\":\"https://a.io\",\"admin\":\"https://adm.a.io\",\"invite\":\"https://i.gg/\",\"gift\":\"https://g.gg\",\"webapp\":\"https://w.a.io\"}}";
    let call = format!("$instanceValue[parse;{full}]");
    let out = rt.run(&ctx, &call).await.unwrap();
    let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["endpoints"]["invite"], "https://i.gg");
    assert_eq!(doc["endpoints"]["media"], "https://cdn.a.io");
    assert_eq!(doc["features"]["voice_enabled"], true);

    // parse rejects non-object input.
    assert!(rt.run(&ctx, "$instanceValue[parse;[]]").await.is_err());
    // unknown operation fails.
    assert!(rt.run(&ctx, "$instanceValue[bogus;x]").await.is_err());
}

#[async_std::test]
async fn context_bindings() {
    let mut ctx = base_ctx().await;
    ctx.ping_ms = Some(12);
    ctx.help = Some("halp".into());
    ctx.args
        .insert("foo".into(), serde_json::Value::String("bar".into()));
    let ctx = Arc::new(ctx);
    let rt = runtime();
    assert_eq!(rt.run(&ctx, "$ping").await.unwrap(), "12");
    assert_eq!(rt.run(&ctx, "$help").await.unwrap(), "halp");
    assert_eq!(rt.run(&ctx, "$arg[foo]").await.unwrap(), "bar");
    assert_eq!(rt.run(&ctx, "$arg[missing]").await.unwrap(), "");
}

#[async_std::test]
async fn flags_accept_string_values() {
    let ctx = ctx().await;
    let rt = runtime();
    // sdk_constants.json serializes flag values as strings ("4").
    assert_eq!(
        rt.run(&ctx, "$flags[permissions;resolve;BanMembers]")
            .await
            .unwrap(),
        "4"
    );
    let out = rt.run(&ctx, "$flags[permissions;table]").await.unwrap();
    assert!(out.contains("BanMembers"));
}

#[async_std::test]
async fn message_slice_clamps_ranges() {
    let ctx = ctx().await;
    let rt = runtime();
    // Reversed range yields "" instead of panicking.
    assert_eq!(rt.run(&ctx, "$messageSlice[1;0]").await.unwrap(), "");
    // A start past the input yields "" instead of the last token.
    assert_eq!(rt.run(&ctx, "$messageSlice[9]").await.unwrap(), "");
    assert_eq!(rt.run(&ctx, "$messageSlice[0;1]").await.unwrap(), "hi");
}

#[async_std::test]
async fn storage_rejects_bad_ttl() {
    let ctx = vars_ctx().await;
    let rt = runtime();
    for ttl in ["-5", "1.5"] {
        let err = rt
            .run(&ctx, &format!("$setUserVar[x;v;{ttl}]"))
            .await
            .unwrap_err();
        assert_eq!(guard_code(&err), "VARIABLE", "ttl {ttl}");
        // The rejected write must not erase an existing value.
        assert_eq!(rt.run(&ctx, "$setUserVar[x;v]").await.unwrap(), "");
        let err = rt
            .run(&ctx, &format!("$setUserVar[x;w;{ttl}]"))
            .await
            .unwrap_err();
        assert_eq!(guard_code(&err), "VARIABLE");
        assert_eq!(rt.run(&ctx, "$getUserVar[x]").await.unwrap(), "v");
        assert_eq!(rt.run(&ctx, "$deleteUserVar[x]").await.unwrap(), "");
    }
}

#[async_std::test]
async fn invite_url_joins_code_to_base() {
    let ctx = ctx().await;
    let rt = runtime();
    assert_eq!(
        rt.run(&ctx, "$inviteURL[abc123]").await.unwrap(),
        "https://fluxer.gg/abc123"
    );
    assert_eq!(
        rt.run(&ctx, "$inviteURL[abc;https://i.gg/]").await.unwrap(),
        "https://i.gg/abc"
    );
}

#[async_std::test]
async fn http_stream_validates_method() {
    let ctx = ctx().await;
    let rt = runtime();
    let err = rt
        .run(&ctx, "$httpStream[YEET;http://127.0.0.1:9/x;$let[i;0]]")
        .await
        .unwrap_err();
    assert_eq!(guard_code(&err), "HTTP_METHOD");
}

#[async_std::test]
async fn http_enforces_body_limit() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut head = [0u8; 8192];
        let _ = conn.read(&mut head);
        let body = vec![b'a'; 6 * 1024 * 1024];
        write!(
            conn,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        conn.write_all(&body).unwrap();
    });

    let ctx = ctx().await;
    let rt = runtime();
    let err = rt
        .run(&ctx, &format!("$http[GET;http://127.0.0.1:{port}/big]"))
        .await
        .unwrap_err();
    assert_eq!(guard_code(&err), "HTTP");
}

#[test]
fn scheduler_edit_rejects_unknown_keys_atomically() {
    use flush_functions::scheduler::{ScheduledJob, ScriptScheduler};
    use serde_json::Map;

    let scheduler = ScriptScheduler::new();
    let id = scheduler.add(ScheduledJob {
        id: String::new(),
        command: "remind".into(),
        input: None,
        due: Some(10),
        interval_ms: None,
        cron: None,
        guild_id: "333".into(),
        channel_id: "222".into(),
        author_id: "444".into(),
        priority: 0,
        max_attempts: 3,
        retry_delay_ms: 0,
        retry_backoff: 1.0,
        retry_jitter: 0.0,
        paused: false,
    });
    let mut patch = Map::new();
    patch.insert("command".into(), serde_json::json!("changed"));
    patch.insert("bogus".into(), serde_json::json!(1));
    assert!(scheduler.edit(&id, &patch, "333", "444").is_err());
    // The valid field must not be applied after the error.
    let job = scheduler.list();
    assert_eq!(job[0]["command"], serde_json::json!("remind"));
}

#[test]
fn scheduler_retry_replays_job_snapshot() {
    use flush_functions::scheduler::{ScheduleHistoryEntry, ScheduledJob, ScriptScheduler};

    let scheduler = ScriptScheduler::new();
    let job = ScheduledJob {
        id: String::new(),
        command: "remind".into(),
        input: Some("drink water".into()),
        due: Some(10),
        interval_ms: None,
        cron: None,
        guild_id: "333".into(),
        channel_id: "222".into(),
        author_id: "444".into(),
        priority: 1,
        max_attempts: 3,
        retry_delay_ms: 0,
        retry_backoff: 1.0,
        retry_jitter: 0.0,
        paused: false,
    };
    scheduler.record(ScheduleHistoryEntry {
        job,
        execution_id: "exec-1".into(),
        job_id: "job-1".into(),
        guild_id: "333".into(),
        author_id: "444".into(),
        status: "failed".into(),
        value: serde_json::Value::Null,
    });
    let new_id = scheduler.retry("exec-1", 99, "333", "444").unwrap();
    let jobs = scheduler.list();
    let replayed = jobs
        .iter()
        .find(|j| j["id"] == serde_json::json!(new_id))
        .unwrap();
    assert_eq!(replayed["command"], serde_json::json!("remind"));
    assert_eq!(replayed["input"], serde_json::json!("drink water"));
    assert_eq!(replayed["channelID"], serde_json::json!("222"));
    assert_eq!(replayed["due"], serde_json::json!(99));
}

#[test]
fn case_type_covers_voice_actions() {
    use flush_framework::CaseType;
    for name in ["move", "mute", "deafen"] {
        assert_eq!(CaseType::parse(name).unwrap().as_str(), name);
    }
}
