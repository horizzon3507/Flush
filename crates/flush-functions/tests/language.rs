use std::collections::HashMap;
use std::sync::Arc;

use flush_core::Message;
use flush_framework::{MemoryVariableStore, VariableService, VariableStore};
use flush_functions::{builtins::register_builtins, define_functions, ScriptContext};
use flush_functions::{Runtime, DEFAULT_LIMITS};
use flush_rest::{RestClient, RestConfig};

async fn ctx() -> Arc<ScriptContext> {
    ctx_with(true).await
}

async fn ctx_with(variables: bool) -> Arc<ScriptContext> {
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
        owners: vec![],
        dev_guilds: vec![],
        variables: variables.then(|| {
            Arc::new(VariableService::new(
                Arc::new(MemoryVariableStore::new()) as Arc<dyn VariableStore + Send + Sync>,
                Some("333"),
                Some("444"),
            ))
        }),
        cases: None,
        scheduler: None,
        collectors: None,
        now: Box::new(|| 1_700_000_000_000),
    })
}

fn runtime() -> Runtime {
    let rt = Runtime::with_limits(DEFAULT_LIMITS);
    rt.register_all(define_functions(register_builtins))
        .unwrap();
    rt
}

async fn run(source: &str) -> String {
    runtime().run(&ctx().await, source).await.unwrap()
}

#[async_std::test]
async fn escapes() {
    assert_eq!(run(r"\$let \[ \] \;").await, "$let [ ] ;");
}

#[async_std::test]
async fn lazy_args() {
    // `$if` and friends must not evaluate the untaken branch.
    assert_eq!(run("$and[false;$nope[]]").await, "false");
    assert_eq!(run("$or[true;$nope[]]").await, "true");
    // `arg`-style readers keep laziness too: `join` only reads what it joins.
    assert_eq!(run("$join[-;a;b;c]").await, "a-b-c");
}

#[async_std::test]
async fn repeat_loop() {
    assert_eq!(run("$repeat[3;x]").await, "xxx");
    assert_eq!(run("$repeat[3;$loopIndex]").await, "012");
}

#[async_std::test]
async fn while_loop() {
    assert_eq!(
        run("$let[i;0]$while[$ne[$get[i];3];$let[i;$math[$get[i];+;1]]x]").await,
        "xxx"
    );
}

#[async_std::test]
async fn break_and_continue() {
    // `$jsonGet` reads its default lazily: `$break`/`$continue` only run
    // when the "true" key is absent (at index 2).
    assert_eq!(
        run("$repeat[5;$jsonGet[$object[$ne[$loopIndex;2];];true;$break]$loopIndex]").await,
        "01"
    );
    assert_eq!(
        run("$repeat[5;$jsonGet[$object[$ne[$loopIndex;2];];true;$continue]$loopIndex]").await,
        "0134"
    );
}

#[async_std::test]
async fn for_each() {
    assert_eq!(
        run("$forEach[$jsonDecode[[10,20]];$loopValue:$loopIndex,]").await,
        "10:0,20:1,"
    );
}

#[async_std::test]
async fn vars_and_state() {
    let ctx = ctx_with(true).await;
    let out = runtime()
        .run(&ctx, "$incrementVar[score;2]$incrementVar[score;3]")
        .await
        .unwrap();
    assert_eq!(out, "25");
    let out = runtime()
        .run(&ctx, "$pushVar[list;1]$pushVar[list;2]$pushVar[list;a]")
        .await
        .unwrap();
    assert_eq!(out, "[1][1,2][1,2,\"a\"]");
    let out = runtime().run(&ctx, "$setVarPath[obj;a.b;7]").await.unwrap();
    assert_eq!(out, "{\"a\":{\"b\":7}}");
    let out = runtime().run(&ctx, "$pullVar[list;a]").await.unwrap();
    assert_eq!(out, "[1,2]");
}

#[async_std::test]
async fn typed_args_and_objects() {
    // `jsonValues`/`jsonEntries` were swapped (review finding).
    assert_eq!(run("$jsonValues[$object[a;1;b;2]]").await, "[1,2]");
    assert_eq!(run("$jsonEntries[$object[a;1]]").await, "[[\"a\",1]]");
    assert_eq!(
        run("$jsonEntries[$jsonDecode[[10,20]]]").await,
        "[[\"0\",10],[\"1\",20]]"
    );
    // Typed args: a single-$-call argument keeps its JSON type (null
    // survives as `null`, not `""`).
    assert_eq!(run("$jsonEncode[$jsonDecode[null]]").await, "null");
    assert_eq!(
        run("$arrayMap[$jsonDecode[[null,1,\"x\"]];$loopValue]").await,
        "[null,1,\"x\"]"
    );
    // `parseDate` validates fields instead of panicking or normalizing.
    for bad in [
        "2024-02-30",
        "2024-01-01T",
        "2024-01-01T99:99Z",
        "2024-13-01",
    ] {
        let err = runtime()
            .run(&ctx().await, &format!("$parseDate[{bad}]"))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "DATE", "input {bad}");
    }
    assert_eq!(
        run("$parseDate[2024-02-29T12:30:00Z]").await,
        "1709209800000"
    );
    // `deep_equal`: distinct large integers must not collapse via f64.
    let ctx = ctx_with(true).await;
    let out = runtime()
        .run(
            &ctx,
            "$pushVar[big;$jsonDecode[9007199254740992]]$pushVar[big;$jsonDecode[9007199254740993]]$pullVar[big;9007199254740992]",
        )
        .await
        .unwrap();
    assert_eq!(
        out,
        "[9007199254740992][9007199254740992,9007199254740993][9007199254740993]"
    );
}

#[async_std::test]
async fn arrays() {
    assert_eq!(run("$arrayJoin[$array[1;2;3];,]").await, "1,2,3");
    assert_eq!(run("$arrayLength[$array[1;2;3]]").await, "3");
    assert_eq!(run("$arrayGet[$array[1;2];1]").await, "2");
    assert_eq!(run("$arrayGet[$array[1];9;none]").await, "none");
    assert_eq!(run("$arrayPush[$jsonDecode[[1]];2]").await, "[1,2]");
    assert_eq!(run("$arraySlice[$array[1;2;3];1]").await, "[2,3]");
    assert_eq!(run("$arrayIncludes[$array[1;2];2]").await, "true");
    assert_eq!(run("$arrayIndexOf[$array[a;b];b]").await, "1");
    assert_eq!(run("$arrayReverse[$array[1;2]]").await, "[2,1]");
    assert_eq!(run("$arrayUnique[$array[1;1;2]]").await, "[1,2]");
    assert_eq!(
        run("$arrayFlat[$jsonDecode[[1,[2,[3]]]]]").await,
        "[1,2,[3]]"
    );
    assert_eq!(run("$arrayChunk[$range[0;4];2]").await, "[[0,1],[2,3],[4]]");
    assert_eq!(run("$range[1;5]").await, "[1,2,3,4,5]");
    assert_eq!(run("$paginate[$range[1;9];2;3]").await, "[4,5,6]");
}

#[async_std::test]
async fn array_callbacks() {
    assert_eq!(
        run("$arrayMap[$array[1;2];$math[$loopValue;*;2]]").await,
        "[2,4]"
    );
    assert_eq!(
        run("$arrayFilter[$array[1;2;3];$math[$loopValue;%;2]]").await,
        "[1,3]"
    );
    assert_eq!(
        run("$arraySome[$array[1;2];$eq[$loopValue;2]]").await,
        "true"
    );
    assert_eq!(
        run("$arrayEvery[$array[1;2];$eq[$loopValue;1]]").await,
        "false"
    );
    assert_eq!(run("$arrayFind[$array[5;6];$eq[$loopValue;6]]").await, "6");
    assert_eq!(
        run("$arrayReduce[$array[1;2;3];0;$math[$loopAccumulator;+;$loopValue]]").await,
        "6"
    );
}

#[async_std::test]
async fn sort() {
    assert_eq!(run("$arraySort[$array[3;1;2]]").await, "[1,2,3]");
    assert_eq!(run("$arraySort[$array[3;1;2];desc]").await, "[3,2,1]");
    assert_eq!(
        run("$arraySortWith[$array[bb;ccc;a];$math[$length[$sortRight];-;$length[$sortLeft]]]")
            .await,
        "[\"ccc\",\"bb\",\"a\"]"
    );
    assert_eq!(
        run("$arraySortBy[$array[bb;a;ccc];$length[$loopValue]]").await,
        "[\"a\",\"bb\",\"ccc\"]"
    );
}

#[async_std::test]
async fn texts() {
    assert_eq!(run("$split[a,b;,]").await, "[\"a\",\"b\"]");
    assert_eq!(run("$split[ab;]").await, "[\"a\",\"b\"]");
    assert_eq!(run("$sliceText[hello;1;3]").await, "el");
    assert_eq!(run("$includes[hello;ell]").await, "true");
    assert_eq!(run("$startsWith[hello;he]").await, "true");
    assert_eq!(run("$endsWith[hello;lo]").await, "true");
    assert_eq!(run("$padStart[5;3;0]").await, "005");
    assert_eq!(run("$padEnd[5;3;0]").await, "500");
    assert_eq!(run("$repeatText[ab;3]").await, "ababab");
    assert_eq!(run("$truncate[hello world;5;...]").await, "hello...");
    assert_eq!(run("$escapeMarkdown[a*b]").await, "a\\*b");
    assert_eq!(run("$mention[123]").await, "<@123>");
    assert_eq!(run("$mention[123;channel]").await, "<#123>");
}

#[async_std::test]
async fn objects_and_values() {
    assert_eq!(run("$object[a;1;b;2]").await, "{\"a\":1,\"b\":2}");
    assert_eq!(run("$jsonGet[$object[a;$object[b;7]];a.b]").await, "7");
    assert_eq!(run("$jsonGet[$object[a;1];b;x]").await, "x");
    assert_eq!(run("$jsonSet[$object[a;1];a;2]").await, "{\"a\":2}");
    assert_eq!(run("$jsonKeys[$object[a;1;b;2]]").await, "[\"a\",\"b\"]");
    assert_eq!(
        run("$jsonMerge[$object[a;1];$object[b;2]]").await,
        "{\"a\":1,\"b\":2}"
    );
    assert_eq!(run("$hasProp[$object[a;1];a]").await, "true");
    assert_eq!(run("$prop[$object[a;5];a]").await, "5");
    assert_eq!(run("$eq[a;a]").await, "true");
    assert_eq!(run("$math[2;+;3]").await, "5");
    assert_eq!(run("$replace[a-b-c;-;+]").await, "a+b+c");
    assert_eq!(run("$decimal[0.1;+;0.2]").await, "0.3");
    assert_eq!(run("$decimalCompare[1.10;1.2]").await, "-1");
    assert_eq!(run("$toBase[255;16]").await, "ff");
    assert_eq!(run("$fromBase[ff;16]").await, "255");
}

#[async_std::test]
async fn regex() {
    assert_eq!(run("$regexTest[a1;\\d]").await, "true");
    assert_eq!(run("$regexMatch[a1b22;\\d+;g]").await, "[{\"value\":\"1\",\"index\":1,\"captures\":[],\"groups\":null},{\"value\":\"22\",\"index\":3,\"captures\":[],\"groups\":null}]");
    assert_eq!(run("$regexReplace[a1b2;\\d;x;g]").await, "axbx");
    assert_eq!(run("$regexReplace[a1;(\\d);[$&][$1]]").await, "a[1][1]");
    let err = runtime()
        .run(&ctx().await, "$regexTest[x;(]")
        .await
        .unwrap_err();
    assert_eq!(err.code(), "REGEX_INVALID");
    let err = runtime()
        .run(&ctx().await, "$regexTest[x;y;gg]")
        .await
        .unwrap_err();
    assert_eq!(err.code(), "REGEX_FLAGS");
}
