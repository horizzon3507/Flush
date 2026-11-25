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
    assert_eq!(rt.run(&ctx, "$messageSlice[1]").await.unwrap(), "hi there");
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
        .run(&ctx, "$instanceValue[resolve;{\"api\":\"https://x.io/api/v3\"}]")
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
