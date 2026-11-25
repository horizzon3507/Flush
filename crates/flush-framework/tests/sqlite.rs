//! SQLite store: persistence, schema guard, cases, guild configuration.

use std::path::PathBuf;
use std::sync::Arc;

use flush_framework::{
    CaseStore, CaseType, GuildConfigPatch, GuildStore, SqliteStore, StoreError, VariableError,
    VariableQueryOptions, VariableScope, VariableService,
};
use serde_json::json;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("flush-sqlite-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn vars(store: &Arc<SqliteStore>) -> VariableService<Arc<SqliteStore>> {
    VariableService::new(store.clone(), Some("g1"), Some("u1"))
}

#[test]
fn data_survives_reopen() {
    let dir = TempDir::new("reopen");
    let path = dir.file("nested/flush.sqlite");
    {
        let store = Arc::new(SqliteStore::open(&path).unwrap());
        vars(&store)
            .set(VariableScope::Guild, "coins", json!(5))
            .unwrap();
        store
            .add_case("g1", CaseType::Warn, "u2", "u1", "spam")
            .unwrap();
        store
            .update_guild_config(
                "g1",
                GuildConfigPatch {
                    prefix: Some("  ?!xyz ".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        store.close().unwrap();
        assert!(matches!(
            vars(&store).get(VariableScope::Guild, "coins"),
            Err(VariableError::Store(StoreError::Closed))
        ));
        assert!(matches!(store.case("g1", 1), Err(StoreError::Closed)));
    }
    let store = Arc::new(SqliteStore::open(&path).unwrap());
    assert_eq!(
        vars(&store).get(VariableScope::Guild, "coins").unwrap(),
        Some(json!(5))
    );
    assert_eq!(store.case("g1", 1).unwrap().unwrap().reason, "spam");
    assert_eq!(store.prefix(Some("g1")).unwrap(), "?!x");
    assert_eq!(store.prefix(None).unwrap(), "");
}

#[cfg(unix)]
#[test]
fn database_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("perms");
    let path = dir.file("private/flush.sqlite");
    SqliteStore::open(&path).unwrap();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);
}

#[test]
fn two_connections_share_one_file() {
    let dir = TempDir::new("shared");
    let path = dir.file("flush.sqlite");
    let a = Arc::new(SqliteStore::open(&path).unwrap());
    let b = Arc::new(SqliteStore::open(&path).unwrap());
    let threads: Vec<_> = [a.clone(), b.clone()]
        .into_iter()
        .map(|store| {
            std::thread::spawn(move || {
                for _ in 0..50 {
                    vars(&store)
                        .increment(VariableScope::Global, "hits", 1.0)
                        .unwrap();
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(
        vars(&a).get(VariableScope::Global, "hits").unwrap(),
        Some(json!(100))
    );
}

#[test]
fn unknown_schema_is_refused_untouched() {
    let dir = TempDir::new("schema");
    let path = dir.file("flush.sqlite");
    SqliteStore::open(&path).unwrap().close().unwrap();
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute(
        "UPDATE flush_meta SET value = '99' WHERE key = 'schema'",
        [],
    )
    .unwrap();
    raw.execute(
        "INSERT INTO flush_variables VALUES ('global', '', '006e', 'n', '1', NULL)",
        [],
    )
    .unwrap();
    raw.pragma_update(None, "journal_mode", "DELETE").unwrap();
    drop(raw);
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        || std::fs::metadata(&path).unwrap().permissions().mode() & 0o777
    };
    assert!(matches!(SqliteStore::open(&path), Err(StoreError::Corrupt)));
    #[cfg(unix)]
    assert_eq!(mode(), 0o640);
    let raw = rusqlite::Connection::open(&path).unwrap();
    let journal: String = raw
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal, "delete");
    let (version, rows): (String, i64) = raw
        .query_row(
            "SELECT (SELECT value FROM flush_meta WHERE key = 'schema'), (SELECT COUNT(*) FROM flush_variables)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((version.as_str(), rows), ("99", 1));
}

#[test]
fn corrupt_value_is_reported_not_dropped() {
    let dir = TempDir::new("corrupt");
    let path = dir.file("flush.sqlite");
    let store = Arc::new(SqliteStore::open(&path).unwrap());
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute(
        "INSERT INTO flush_variables VALUES ('guild', 'g1', '006e', 'n', '{broken', NULL)",
        [],
    )
    .unwrap();
    let service = vars(&store);
    assert!(matches!(
        service.get(VariableScope::Guild, "n"),
        Err(VariableError::Store(StoreError::Corrupt))
    ));
    assert!(service
        .query(VariableScope::Guild, VariableQueryOptions::default())
        .is_err());
    assert!(service.increment(VariableScope::Guild, "n", 1.0).is_err());
    let json: String = raw
        .query_row("SELECT value_json FROM flush_variables", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(json, "{broken");
}

#[test]
fn cases_match_the_json_store() {
    let store = SqliteStore::open_in_memory().unwrap();
    let first = store
        .add_case("g1", CaseType::Warn, "u2", "u1", "one")
        .unwrap();
    let second = store
        .add_case("g1", CaseType::Ban, "u2", "u1", "two")
        .unwrap();
    let third = store
        .add_case("g1", CaseType::Warn, "u2", "u1", "three")
        .unwrap();
    store
        .add_case("g1", CaseType::Warn, "u3", "u1", "other user")
        .unwrap();
    let elsewhere = store
        .add_case("g2", CaseType::Kick, "u2", "u1", "other guild")
        .unwrap();
    assert_eq!((first.id, second.id, third.id, elsewhere.id), (1, 2, 3, 1));

    assert_eq!(store.case("g1", 2).unwrap(), Some(second.clone()));
    assert_eq!(store.case("g1", 99).unwrap(), None);
    assert_eq!(
        store.cases_for_user("g1", "u2", 2).unwrap(),
        vec![second.clone(), third.clone()]
    );
    assert_eq!(store.cases_for_user("g1", "u2", 10).unwrap().len(), 3);
    assert_eq!(store.active_warns("g1", "u2").unwrap(), 2);

    assert_eq!(store.revert_warn("g1", "u2", 2).unwrap(), None);
    assert_eq!(store.revert_warn("g1", "u3", 1).unwrap(), None);
    let reverted = store.revert_warn("g1", "u2", 1).unwrap().unwrap();
    assert!(reverted.reverted);
    assert_eq!(store.revert_warn("g1", "u2", 1).unwrap(), None);
    assert_eq!(store.active_warns("g1", "u2").unwrap(), 1);
}

#[test]
fn guild_config_round_trips() {
    let store = SqliteStore::open_in_memory().unwrap();
    let default = store.guild_config("g1").unwrap();
    assert_eq!(default, flush_framework::GuildConfig::default());
    let patched = store
        .update_guild_config(
            "g1",
            GuildConfigPatch {
                prefix: Some("?".into()),
                mod_log_channel_id: Some(Some("123".into())),
                bot_color: Some(Some(0xff00ff)),
            },
        )
        .unwrap();
    assert_eq!(patched.prefix, "?");
    let cleared = store
        .update_guild_config(
            "g1",
            GuildConfigPatch {
                mod_log_channel_id: Some(Some(String::new())),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(cleared.mod_log_channel_id, None);
    assert_eq!(cleared.bot_color, Some(0xff00ff));

    let mut config = cleared;
    config.automod.enabled = true;
    config.automod.ignored_channels = vec!["9".into()];
    store.set_guild_config("g1", &config).unwrap();
    assert_eq!(store.guild_config("g1").unwrap(), config);
    assert_eq!(store.guild_config("g2").unwrap(), default);
}

#[test]
fn imports_the_json_stores() {
    let dir = TempDir::new("import");
    let guilds_path = dir.file("guilds.json");
    let cases_path = dir.file("cases.json");
    let mut guilds = GuildStore::new(guilds_path.clone());
    guilds.update(
        "g1",
        GuildConfigPatch {
            prefix: Some("$".into()),
            ..Default::default()
        },
    );
    let mut cases = CaseStore::new(cases_path.clone());
    cases.add("g1", CaseType::Warn, "u2", "u1", "one").unwrap();
    cases
        .add("g1", CaseType::Timeout, "u2", "u1", "two")
        .unwrap();
    cases.revert_warn("g1", "u2", 1).unwrap();

    let store = SqliteStore::open_in_memory().unwrap();
    assert_eq!(store.import_guilds_json(&guilds_path).unwrap(), 1);
    assert_eq!(store.import_cases_json(&cases_path).unwrap(), 2);
    assert_eq!(store.prefix(Some("g1")).unwrap(), "$");
    assert_eq!(
        store.cases_for_user("g1", "u2", 10).unwrap(),
        cases.for_user("g1", "u2", 10)
    );
    // Ids continue after the imported ones, and a second import adds nothing.
    assert_eq!(
        store
            .add_case("g1", CaseType::Kick, "u2", "u1", "three")
            .unwrap()
            .id,
        3
    );
    assert_eq!(store.import_cases_json(&cases_path).unwrap(), 0);
    assert_eq!(
        store
            .add_case("g1", CaseType::Kick, "u2", "u1", "four")
            .unwrap()
            .id,
        4
    );

    // Each file parses as the other kind; importing it there is refused and
    // changes nothing.
    assert!(matches!(
        store.import_guilds_json(&cases_path),
        Err(StoreError::Import(_))
    ));
    assert_eq!(store.prefix(Some("g1")).unwrap(), "$");
    assert!(matches!(
        store.import_cases_json(&guilds_path),
        Err(StoreError::Import(_))
    ));
    assert_eq!(store.cases_for_user("g1", "u2", 10).unwrap().len(), 4);

    // A repeated field or guild id is refused, not settled by the last one.
    let repeated = dir.file("repeated.json");
    for text in [
        r#"{"g1": {"prefix": "!", "prefix": "?"}}"#,
        r#"{"g1": {"automod": {"enabled": true, "enabled": false}}}"#,
        r#"{"g1": {"prefix": "!"}, "g1": {"prefix": "?"}}"#,
    ] {
        std::fs::write(&repeated, text).unwrap();
        assert!(matches!(
            store.import_guilds_json(&repeated),
            Err(StoreError::Corrupt)
        ));
    }
    assert_eq!(store.prefix(Some("g1")).unwrap(), "$");
    for text in [
        r#"{"g9": {"next_id": 1, "cases": [], "cases": []}}"#,
        r#"{"g9": {"next_id": 1}, "g9": {"next_id": 2}}"#,
    ] {
        std::fs::write(&repeated, text).unwrap();
        assert!(matches!(
            store.import_cases_json(&repeated),
            Err(StoreError::Corrupt)
        ));
    }

    assert_eq!(
        store.import_cases_json(&dir.file("missing.json")).unwrap(),
        0
    );
    std::fs::write(&cases_path, "{not json").unwrap();
    assert!(matches!(
        store.import_cases_json(&cases_path),
        Err(StoreError::Corrupt)
    ));
}
