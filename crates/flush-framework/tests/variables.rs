//! Variable service contract, run against both stores.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use flush_framework::{
    execute_operations, MemoryVariableStore, SqliteStore, Ttl, VariableError, VariableOperation,
    VariableQueryOptions, VariableScope, VariableService, VariableStore,
};
use serde_json::{json, Value};

use VariableScope::{Global, Guild, User};

struct Fixture<S> {
    vars: VariableService<Arc<S>>,
    clock: Arc<AtomicU64>,
}

fn fixture<S: VariableStore>(store: S) -> Fixture<S> {
    let clock = Arc::new(AtomicU64::new(1_000));
    let tick = clock.clone();
    let vars = VariableService::new(Arc::new(store), Some("g1"), Some("u1"))
        .with_clock(move || tick.load(Ordering::SeqCst));
    Fixture { vars, clock }
}

fn names(page: &flush_framework::VariablePage) -> Vec<&str> {
    page.entries.iter().map(|e| e.name.as_str()).collect()
}

fn contract<S: VariableStore>(store: S) {
    let Fixture { vars, clock } = fixture(store);

    // Scopes are isolated from each other and from other subjects.
    vars.set(Global, "n", json!(1)).unwrap();
    vars.set(Guild, "n", json!({"a": [1, "x", null]})).unwrap();
    vars.set(User, "n", json!("mine")).unwrap();
    assert_eq!(vars.get(Global, "n").unwrap(), Some(json!(1)));
    assert_eq!(
        vars.get(Guild, "n").unwrap(),
        Some(json!({"a": [1, "x", null]}))
    );
    assert_eq!(vars.get(User, "n").unwrap(), Some(json!("mine")));
    let other = vars.for_user("u2").unwrap();
    assert_eq!(other.get(User, "n").unwrap(), None);
    assert_eq!(
        other.get(Guild, "n").unwrap(),
        vars.get(Guild, "n").unwrap()
    );
    assert_eq!(vars.get_or(Guild, "missing", json!(7)).unwrap(), json!(7));

    // TTL: -2 missing, -1 persistent, otherwise remaining ms.
    assert_eq!(vars.ttl(Guild, "missing").unwrap(), Ttl::Missing);
    assert_eq!(vars.ttl(Guild, "n").unwrap().as_millis(), -1);
    vars.set_with_ttl(Guild, "temp", json!(true), 500).unwrap();
    clock.store(1_200, Ordering::SeqCst);
    assert_eq!(vars.ttl(Guild, "temp").unwrap(), Ttl::Remaining(300));
    clock.store(1_500, Ordering::SeqCst);
    assert_eq!(vars.get(Guild, "temp").unwrap(), None);
    assert_eq!(vars.ttl(Guild, "temp").unwrap().as_millis(), -2);
    assert_eq!(vars.sweep_expired(10).unwrap(), 1);
    assert_eq!(vars.sweep_expired(10).unwrap(), 0);

    // update keeps the TTL; expire sets and clears it.
    vars.set_with_ttl(Guild, "count", json!(1), 1_000).unwrap();
    let next = vars.increment(Guild, "count", 2.0).unwrap();
    assert_eq!(next, json!(3));
    assert_eq!(vars.ttl(Guild, "count").unwrap(), Ttl::Remaining(1_000));
    assert!(vars.expire(Guild, "count", None).unwrap());
    assert_eq!(vars.ttl(Guild, "count").unwrap(), Ttl::Persistent);
    assert!(vars.expire(Guild, "count", Some(50)).unwrap());
    assert_eq!(vars.ttl(Guild, "count").unwrap(), Ttl::Remaining(50));
    assert!(!vars.expire(Guild, "missing", Some(50)).unwrap());
    let updated = vars
        .update(Guild, "list", json!([]), |v| {
            let mut items = v.as_array().cloned().unwrap_or_default();
            items.push(json!("a"));
            Ok(Value::Array(items))
        })
        .unwrap();
    assert_eq!(updated, json!(["a"]));

    // A write over an expired entry does not inherit its old expiry.
    clock.store(10_000, Ordering::SeqCst);
    assert_eq!(vars.increment(Guild, "count", 1.0).unwrap(), json!(1));
    assert_eq!(vars.ttl(Guild, "count").unwrap(), Ttl::Persistent);

    vars.delete(Guild, "count").unwrap();
    assert_eq!(vars.get(Guild, "count").unwrap(), None);
}

fn transactions<S: VariableStore>(store: S) {
    let Fixture { vars, .. } = fixture(store);
    vars.set(Guild, "coins", json!(10)).unwrap();

    // Every write of a failed transaction is rolled back.
    let failed = vars.transaction(|tx| {
        tx.guild().set("coins", json!(0), None)?;
        tx.user().set("bought", json!(true), None)?;
        tx.guild().set("", json!(1), None)
    });
    assert!(matches!(failed, Err(VariableError::Name)));
    assert_eq!(vars.get(Guild, "coins").unwrap(), Some(json!(10)));
    assert_eq!(vars.get(User, "bought").unwrap(), None);

    // Writes are visible to later reads in the same transaction.
    let seen = vars
        .transaction(|tx| {
            tx.guild().increment("coins", -4.0, 0.0)?;
            tx.user_for("u2")?.set("gift", json!(4), Some(100))?;
            tx.guild().get("coins")
        })
        .unwrap();
    assert_eq!(seen, Some(json!(6)));
    assert_eq!(
        vars.for_user("u2").unwrap().get(User, "gift").unwrap(),
        Some(json!(4))
    );

    // JSON-shaped batch, as Yuzuha's data transactions.
    let operations: Vec<VariableOperation> = serde_json::from_value(json!([
        {"op": "set", "scope": "guild", "name": "a", "value": 1, "ttlMs": 500},
        {"op": "increment", "scope": "guild", "name": "a", "delta": 2},
        {"op": "ttl", "scope": "guild", "name": "a"},
        {"op": "expire", "scope": "guild", "name": "a", "ttlMs": null},
        {"op": "ttl", "scope": "guild", "name": "a"},
        {"op": "get", "scope": "user", "name": "gift", "userID": "u2"},
        {"op": "get", "scope": "global", "name": "nope", "fallback": "x"},
        {"op": "delete", "scope": "guild", "name": "a"},
        {"op": "ttl", "scope": "guild", "name": "a"}
    ]))
    .unwrap();
    let results = vars
        .transaction(|tx| execute_operations(tx, &operations))
        .unwrap();
    assert_eq!(
        results,
        vec![
            json!(null),
            json!(3),
            json!(500),
            json!(true),
            json!(-1),
            json!(4),
            json!("x"),
            json!(null),
            json!(-2)
        ]
    );

    // A failing batch leaves nothing behind; size is capped at 100.
    let bad: Vec<VariableOperation> = serde_json::from_value(json!([
        {"op": "set", "scope": "guild", "name": "b", "value": "text"},
        {"op": "increment", "scope": "guild", "name": "b"}
    ]))
    .unwrap();
    assert!(vars.transaction(|tx| execute_operations(tx, &bad)).is_err());
    assert_eq!(vars.get(Guild, "b").unwrap(), None);
    let many = vec![bad[0].clone(); 101];
    assert!(vars
        .transaction(|tx| execute_operations(tx, &many))
        .is_err());
    assert!(vars.transaction(|tx| execute_operations(tx, &[])).is_err());
    assert!(serde_json::from_value::<VariableOperation>(
        json!({"op": "drop", "scope": "guild", "name": "b"})
    )
    .is_err());
}

fn queries<S: VariableStore>(store: S) {
    let Fixture { vars, clock } = fixture(store);
    // UTF-16 order: "Z" < "a" < "é" < U+1F600 (D83D DE00) < U+FF5E.
    for name in [
        "item:b",
        "item:a",
        "item:Z",
        "item:é",
        "item:\u{ff5e}",
        "item:\u{1f600}",
        "item%",
        "other",
    ] {
        vars.set(Guild, name, json!(name)).unwrap();
    }
    vars.set_with_ttl(Guild, "item:gone", json!(0), 10).unwrap();
    vars.set_with_ttl(Guild, "item:soon", json!(0), 500)
        .unwrap();
    vars.set(User, "item:user", json!(0)).unwrap();
    clock.store(1_100, Ordering::SeqCst);

    let options = |after: Option<&str>, limit| VariableQueryOptions {
        prefix: Some("item:".into()),
        after: after.map(str::to_owned),
        limit: Some(limit),
    };
    let first = vars.query(Guild, options(None, 3)).unwrap();
    assert_eq!(names(&first), ["item:Z", "item:a", "item:b"]);
    assert_eq!(first.next_cursor.as_deref(), Some("item:b"));
    let second = vars
        .query(Guild, options(first.next_cursor.as_deref(), 3))
        .unwrap();
    assert_eq!(names(&second), ["item:soon", "item:é", "item:\u{1f600}"]);
    assert_eq!(second.entries[0].ttl, Ttl::Remaining(400));
    assert_eq!(second.entries[1].ttl, Ttl::Persistent);
    let third = vars
        .query(Guild, options(second.next_cursor.as_deref(), 3))
        .unwrap();
    assert_eq!(names(&third), ["item:\u{ff5e}"]);
    assert_eq!(third.next_cursor, None);

    // The prefix is literal: `%` is not a wildcard.
    let literal = vars
        .query(
            Guild,
            VariableQueryOptions {
                prefix: Some("item%".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(names(&literal), ["item%"]);
    assert_eq!(
        vars.query(Guild, VariableQueryOptions::default())
            .unwrap()
            .entries
            .len(),
        9
    );
    assert_eq!(
        names(&vars.query(User, options(None, 50)).unwrap()),
        ["item:user"]
    );
    assert!(vars
        .query(Global, options(None, 50))
        .unwrap()
        .entries
        .is_empty());

    for limit in [0, 101] {
        assert!(matches!(
            vars.query(Guild, options(None, limit)),
            Err(VariableError::Query)
        ));
    }
    let long = "x".repeat(129);
    assert!(matches!(
        vars.query(
            Guild,
            VariableQueryOptions {
                prefix: Some(long),
                ..Default::default()
            }
        ),
        Err(VariableError::Query)
    ));
}

fn validation<S: VariableStore>(store: S) {
    let Fixture { vars, .. } = fixture(store);
    let longest = "n".repeat(128);
    assert!(matches!(
        vars.set(Guild, "", json!(1)),
        Err(VariableError::Name)
    ));
    assert!(matches!(
        vars.set(Guild, &"n".repeat(129), json!(1)),
        Err(VariableError::Name)
    ));
    vars.set(Guild, &longest, json!(1)).unwrap();
    assert!(matches!(
        vars.set_with_ttl(Guild, "n", json!(1), u64::MAX),
        Err(VariableError::Ttl)
    ));
    assert!(matches!(vars.for_user(" u2"), Err(VariableError::UserId)));
    assert!(matches!(vars.for_user(""), Err(VariableError::UserId)));
    vars.increment(Guild, &longest, f64::MAX).unwrap();
    assert!(matches!(
        vars.increment(Guild, &longest, f64::MAX),
        Err(VariableError::Operation(_))
    ));

    let dm = VariableService::new(vars.store().clone(), None, Some("u1"));
    assert!(matches!(
        dm.set(Guild, "n", json!(1)),
        Err(VariableError::NotInGuild)
    ));
    assert!(matches!(dm.for_user("u2"), Err(VariableError::NotInGuild)));
    dm.set(User, "n", json!(1)).unwrap();
    dm.set(Global, "n", json!(1)).unwrap();
    let nobody = VariableService::new(vars.store().clone(), Some("g1"), None);
    assert!(matches!(
        nobody.get(User, "n"),
        Err(VariableError::NoAuthor)
    ));
}

macro_rules! both_stores {
    ($($name:ident),*) => {$(
        mod $name {
            use super::*;
            #[test]
            fn memory() { super::$name(MemoryVariableStore::new()); }
            #[test]
            fn sqlite() { super::$name(SqliteStore::open_in_memory().unwrap()); }
        }
    )*};
}

both_stores!(contract, transactions, queries, validation);
