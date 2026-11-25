//! `flush data`: inspect, back up and restore the JSON stores
//! (`guilds.json`, `cases.json`) without a config.

use std::io::Write;
use std::path::Path;

use flush_framework::{GuildConfig, ModCase, StoreLock};
use serde_json::{json, Map, Value};

use crate::arguments::{DataOperation, RestorePolicy};
use crate::error::CliError;

const MAX_BACKUP_BYTES: u64 = 50_000_000;

type Store = Map<String, Value>;

fn check_extension(path: &Path) -> Result<(), CliError> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => Ok(()),
        Some("sqlite" | "db") => Err(CliError::new(
            "UNSUPPORTED",
            "SQLite stores are not available in Flush yet (see docs/PARITY.md).",
        )),
        _ => Err(CliError::new("DATA_FORMAT", "The store must end in .json.")),
    }
}

fn read_store(path: &Path) -> Result<Store, CliError> {
    check_extension(path)?;
    if !path.is_file() {
        return Err(CliError::new("DATA_FORMAT", "Pass an existing store file."));
    }
    match serde_json::from_slice(&std::fs::read(path)?) {
        Ok(Value::Object(store)) => Ok(store),
        _ => Err(CliError::new(
            "STORE_CORRUPT",
            "The store is not a JSON object keyed by guild id.",
        )),
    }
}

/// Kind the framework's own file names imply (`cases.json`, `guilds.json`).
fn kind_from_name(path: &Path) -> Option<&'static str> {
    match path.file_stem().and_then(|stem| stem.to_str()) {
        Some("cases") => Some("cases"),
        Some("guilds") => Some("guilds"),
        _ => None,
    }
}

/// `cases` when every guild entry is a case record (a `cases` list or, for a
/// guild without one yet, its `next_id` counter), otherwise `guilds`. An
/// empty store has no entries to tell.
fn kind_from_entries(store: &Store) -> Option<&'static str> {
    if store.is_empty() {
        return None;
    }
    let cases = store.values().all(|entry| {
        entry.get("cases").is_some_and(Value::is_array)
            || entry.get("next_id").is_some_and(Value::is_u64)
    });
    Some(if cases { "cases" } else { "guilds" })
}

/// Kind of a readable store: its entries decide, then its file name.
fn kind(store: &Store, path: &Path) -> &'static str {
    kind_from_entries(store)
        .or_else(|| kind_from_name(path))
        .unwrap_or("guilds")
}

/// Store to restore over, with the kind a backup must have to go into it
/// (`None` when neither the file name nor the contents tell).
///
/// A `replace` restore is how a deleted or damaged store is recovered, so
/// there anything unreadable counts as empty. `merge` keeps the existing
/// entries, so they have to be readable by the bot.
fn read_restore_target(
    path: &Path,
    policy: RestorePolicy,
) -> Result<(Store, Option<&'static str>), CliError> {
    check_extension(path)?;
    if path.is_dir() {
        return Err(CliError::new("DATA_FORMAT", "The store must be a file."));
    }
    let store = match read_store(path) {
        Ok(store) => store,
        Err(error) if error.code == "DATA_FORMAT" => Store::new(),
        Err(error) if error.code == "STORE_CORRUPT" && policy == RestorePolicy::Replace => {
            Store::new()
        }
        Err(error) => return Err(error),
    };
    let expected = kind_from_name(path).or_else(|| kind_from_entries(&store));
    let unreadable = expected.and_then(|kind| invalid_entry(kind, &store));
    match (unreadable, policy) {
        (None, _) => Ok((store, expected)),
        (Some(_), RestorePolicy::Replace) => Ok((Store::new(), expected)),
        (Some(guild), RestorePolicy::Merge) => Err(CliError::new(
            "STORE_CORRUPT",
            format!(
                "The store entry for guild {guild} is unreadable; restore with --policy replace."
            ),
        )),
    }
}

/// Checked even when one side is empty: the bot cannot read the other kind,
/// and an empty backup would still wipe the store.
fn check_kind(expected: Option<&str>, backup_kind: &str) -> Result<(), CliError> {
    match expected.filter(|expected| *expected != backup_kind) {
        Some(expected) => Err(CliError::new(
            "BACKUP_INVALID",
            format!("The backup holds {backup_kind} but the store holds {expected}."),
        )),
        None => Ok(()),
    }
}

fn combine(policy: RestorePolicy, store: Store, data: Store) -> Store {
    match policy {
        RestorePolicy::Replace => data,
        RestorePolicy::Merge => {
            let mut merged = store;
            merged.extend(data);
            merged
        }
    }
}

/// Writes the restore while holding the store's exclusive lock. A running
/// bot keeps its stores in memory and writes them back whole, which would
/// undo the restore, so an open store is refused. The store is read again
/// under the lock: another restore may have written it since the first read.
fn apply_restore(
    path: &Path,
    policy: RestorePolicy,
    backup_kind: &str,
    data: Store,
) -> Result<Store, CliError> {
    let Some(_lock) = StoreLock::try_exclusive(path)? else {
        return Err(CliError::new(
            "STORE_IN_USE",
            "The store is open in a running bot. Stop the bot, then restore again.",
        ));
    };
    let (store, expected) = read_restore_target(path, policy)?;
    check_kind(expected, backup_kind)?;
    let restored = combine(policy, store, data);
    write_atomic(path, &restored)?;
    Ok(restored)
}

fn case_count(store: &Store) -> usize {
    store
        .values()
        .filter_map(|entry| entry.get("cases").and_then(Value::as_array))
        .map(Vec::len)
        .sum()
}

fn stats(store: &Store, path: &Path, bytes: u64) -> Value {
    let kind = kind(store, path);
    let mut out = json!({ "kind": kind, "guilds": store.len(), "bytes": bytes });
    if kind == "cases" {
        out["cases"] = json!(case_count(store));
    }
    out
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// First guild whose entry the stores could not read back. One such entry
/// makes the bot discard the whole file on its next load.
fn invalid_entry<'a>(kind: &str, data: &'a Store) -> Option<&'a str> {
    data.iter()
        .find(|(_, entry)| {
            let valid = if kind == "cases" {
                entry.get("next_id").is_some_and(Value::is_u64)
                    // The bot reads a missing `cases` as an empty list.
                    && entry.get("cases").is_none_or(|cases| {
                        cases.as_array().is_some_and(|cases| {
                            cases
                                .iter()
                                .all(|case| serde_json::from_value::<ModCase>(case.clone()).is_ok())
                        })
                    })
            } else {
                serde_json::from_value::<GuildConfig>((*entry).clone()).is_ok()
            };
            !valid
        })
        .map(|(guild, _)| guild.as_str())
}

fn read_backup(path: &Path) -> Result<(String, Store), CliError> {
    let invalid = |message: &str| CliError::new("BACKUP_INVALID", message);
    let info = std::fs::metadata(path)
        .map_err(|_| invalid("The backup must be an existing JSON file."))?;
    if !info.is_file() || info.len() > MAX_BACKUP_BYTES {
        return Err(invalid(
            "The backup must be a JSON file of up to 50000000 bytes.",
        ));
    }
    let value: Value = serde_json::from_str(&std::fs::read_to_string(path)?)
        .map_err(|_| invalid("The backup must be valid JSON."))?;
    let kind = value.get("kind").and_then(Value::as_str);
    match (
        value.get("version").and_then(Value::as_u64),
        kind,
        value.get("data"),
    ) {
        (Some(1), Some(kind @ ("guilds" | "cases")), Some(Value::Object(data))) => {
            if let Some(guild) = invalid_entry(kind, data) {
                return Err(CliError::new(
                    "BACKUP_INVALID",
                    format!("The backup entry for guild {guild} is not a valid {kind} record."),
                ));
            }
            Ok((kind.to_owned(), data.clone()))
        }
        _ => Err(invalid(
            "The backup must have version 1, a kind of guilds or cases, and a data object.",
        )),
    }
}

fn write_atomic(path: &Path, store: &Store) -> Result<(), CliError> {
    // A fresh project has no data directory until the bot first writes.
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(store)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Runs a data operation and returns the JSON report to print.
pub fn data_command(
    operation: DataOperation,
    input: &Path,
    file: Option<&Path>,
    apply: bool,
    policy: RestorePolicy,
) -> Result<Value, CliError> {
    let (store, expected) = match operation {
        DataOperation::Restore => read_restore_target(input, policy)?,
        _ => (read_store(input)?, None),
    };
    match operation {
        DataOperation::Stats => Ok(stats(&store, input, std::fs::metadata(input)?.len())),
        DataOperation::Backup => {
            let backup = json!({
                "version": 1,
                "kind": kind(&store, input),
                "created_at": unix_now(),
                "data": store,
            });
            let Some(file) = file else {
                return Ok(backup);
            };
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut out = options.open(file).map_err(|e| {
                CliError::new(
                    "BACKUP_WRITE",
                    format!(
                        "Cannot create {}: {e}. Backups never overwrite a file.",
                        file.display()
                    ),
                )
            })?;
            out.write_all(format!("{}\n", serde_json::to_string_pretty(&backup)?).as_bytes())?;
            Ok(json!({ "written": std::path::absolute(file)?, "version": 1 }))
        }
        DataOperation::Restore => {
            let file = file.ok_or_else(|| CliError::new("USAGE", "Restore needs --file."))?;
            let (backup_kind, data) = read_backup(file)?;
            check_kind(expected, backup_kind.as_str())?;
            let before = store.len();
            let restored = if apply {
                apply_restore(input, policy, &backup_kind, data)?
            } else {
                combine(policy, store, data)
            };
            let note = if apply {
                "Restored. Start the bot again to load it."
            } else {
                "Dry run: nothing was written. Stop the bot, then pass --apply to restore."
            };
            Ok(json!({
                "note": note,
                "dry_run": !apply,
                "policy": if policy == RestorePolicy::Merge { "merge" } else { "replace" },
                "kind": backup_kind,
                "guilds_before": before,
                "guilds_after": restored.len(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("flush-data-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn case(id: u64) -> String {
        format!(
            r#"{{"id":{id},"type":"ban","target_id":"7","moderator_id":"8","reason":"spam","timestamp":1}}"#
        )
    }

    fn cases() -> String {
        format!(
            r#"{{"1":{{"next_id":3,"cases":[{},{}]}},"2":{{"next_id":2,"cases":[{}]}}}}"#,
            case(1),
            case(2),
            case(1)
        )
    }

    #[test]
    fn stats_detect_the_store_kind() {
        let dir = temp("stats");
        let cases = dir.join("cases.json");
        std::fs::write(&cases, self::cases()).unwrap();
        let report = data_command(
            DataOperation::Stats,
            &cases,
            None,
            false,
            RestorePolicy::Replace,
        )
        .unwrap();
        assert_eq!(report["kind"], "cases");
        assert_eq!(report["guilds"], 2);
        assert_eq!(report["cases"], 3);

        let guilds = dir.join("guilds.json");
        std::fs::write(&guilds, r#"{"1":{"prefix":"?"}}"#).unwrap();
        let report = data_command(
            DataOperation::Stats,
            &guilds,
            None,
            false,
            RestorePolicy::Replace,
        )
        .unwrap();
        assert_eq!(report["kind"], "guilds");
        assert!(report.get("cases").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn backup_then_restore_round_trips() {
        let dir = temp("roundtrip");
        let store = dir.join("cases.json");
        let backup = dir.join("backup.json");
        std::fs::write(&store, cases()).unwrap();
        data_command(
            DataOperation::Backup,
            &store,
            Some(&backup),
            false,
            RestorePolicy::Replace,
        )
        .unwrap();
        let again = data_command(
            DataOperation::Backup,
            &store,
            Some(&backup),
            false,
            RestorePolicy::Replace,
        );
        assert_eq!(again.unwrap_err().code, "BACKUP_WRITE");

        std::fs::write(&store, r#"{"9":{"next_id":1,"cases":[]}}"#).unwrap();
        let dry = data_command(
            DataOperation::Restore,
            &store,
            Some(&backup),
            false,
            RestorePolicy::Merge,
        )
        .unwrap();
        assert_eq!(dry["dry_run"], true);
        assert_eq!(dry["guilds_after"], 3);
        assert!(std::fs::read_to_string(&store).unwrap().contains("\"9\""));

        let applied = data_command(
            DataOperation::Restore,
            &store,
            Some(&backup),
            true,
            RestorePolicy::Replace,
        )
        .unwrap();
        assert_eq!(applied["dry_run"], false);
        assert_eq!(applied["guilds_after"], 2);
        let restored: Value =
            serde_json::from_str(&std::fs::read_to_string(&store).unwrap()).unwrap();
        assert_eq!(restored, serde_json::from_str::<Value>(&cases()).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_stores_keep_their_kind() {
        let dir = temp("empty");
        let store = dir.join("cases.json");
        let backup = dir.join("backup.json");
        std::fs::write(&store, "{}").unwrap();
        let run = |operation, apply| {
            data_command(
                operation,
                &store,
                Some(&backup),
                apply,
                RestorePolicy::Replace,
            )
        };
        let stats = data_command(
            DataOperation::Stats,
            &store,
            None,
            false,
            RestorePolicy::Replace,
        )
        .unwrap();
        assert_eq!(stats["kind"], "cases");
        run(DataOperation::Backup, false).unwrap();
        std::fs::write(&store, cases()).unwrap();
        let restored = run(DataOperation::Restore, true).unwrap();
        assert_eq!(restored["kind"], "cases");
        assert_eq!(restored["guilds_after"], 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_guild_without_a_case_list_is_readable() {
        let store = |text: &str| serde_json::from_str::<Store>(text).unwrap();
        assert_eq!(
            invalid_entry("cases", &store(r#"{"1":{"next_id":1}}"#)),
            None
        );
        assert_eq!(
            invalid_entry("cases", &store(r#"{"1":{"next_id":1,"cases":null}}"#)),
            Some("1")
        );
        let kind = |text: &str| kind_from_entries(&store(text));
        assert_eq!(kind(r#"{"1":{"next_id":1}}"#), Some("cases"));
        assert_eq!(kind(r#"{"1":{"prefix":"!"}}"#), Some("guilds"));
    }

    #[test]
    fn concurrent_merges_keep_both_guilds() {
        let dir = temp("concurrent");
        let store = dir.join("cases.json");
        let backup = |guild: &str| {
            let file = dir.join(format!("backup-{guild}.json"));
            let data = format!(r#"{{"{guild}":{{"next_id":1,"cases":[]}}}}"#);
            let text = format!(r#"{{"version":1,"kind":"cases","data":{data}}}"#);
            std::fs::write(&file, text).unwrap();
            file
        };
        let (first, second) = (backup("1"), backup("2"));
        let merge = |file: &Path| loop {
            let merge = RestorePolicy::Merge;
            match data_command(DataOperation::Restore, &store, Some(file), true, merge) {
                Ok(_) => break,
                Err(error) => assert_eq!(error.code, "STORE_IN_USE"),
            }
        };
        for _ in 0..50 {
            std::fs::write(&store, "{}").unwrap();
            std::thread::scope(|scope| {
                scope.spawn(|| merge(&first));
                scope.spawn(|| merge(&second));
            });
            assert_eq!(read_store(&store).unwrap().len(), 2);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn restore_is_refused_while_a_bot_has_the_store_open() {
        let dir = temp("in-use");
        let store = dir.join("cases.json");
        let backup = dir.join("backup.json");
        std::fs::write(&store, cases()).unwrap();
        data_command(
            DataOperation::Backup,
            &store,
            Some(&backup),
            false,
            RestorePolicy::Replace,
        )
        .unwrap();
        std::fs::write(&store, "{}").unwrap();
        let restore = |apply, policy| {
            data_command(DataOperation::Restore, &store, Some(&backup), apply, policy)
                .map(|report| report["guilds_after"].clone())
                .map_err(|error| error.code)
        };

        let open = flush_framework::CaseStore::new(store.clone());
        let in_use = Err("STORE_IN_USE".to_owned());
        assert_eq!(restore(true, RestorePolicy::Replace), in_use);
        assert_eq!(restore(true, RestorePolicy::Merge), in_use);
        assert_eq!(std::fs::read_to_string(&store).unwrap(), "{}");
        // A dry run writes nothing, so it needs no lock.
        assert_eq!(restore(false, RestorePolicy::Replace), Ok(json!(2)));

        drop(open);
        assert_eq!(restore(true, RestorePolicy::Replace), Ok(json!(2)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn restore_creates_the_data_directory_of_a_fresh_project() {
        let dir = temp("fresh");
        let seed = dir.join("cases.json");
        let backup = dir.join("backup.json");
        std::fs::write(&seed, cases()).unwrap();
        data_command(
            DataOperation::Backup,
            &seed,
            Some(&backup),
            false,
            RestorePolicy::Replace,
        )
        .unwrap();
        let store = dir.join("project/data/cases.json");
        let restore = |apply| {
            data_command(
                DataOperation::Restore,
                &store,
                Some(&backup),
                apply,
                RestorePolicy::Replace,
            )
            .unwrap()
        };
        assert_eq!(restore(false)["dry_run"], true);
        assert!(!store.parent().unwrap().exists());
        assert_eq!(restore(true)["guilds_after"], 2);
        assert_eq!(
            std::fs::read_to_string(&store)
                .unwrap()
                .matches("spam")
                .count(),
            3
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_backup_of_the_other_kind_never_goes_in() {
        let dir = temp("kinds");
        let store = dir.join("cases.json");
        let backup = dir.join("backup.json");
        let restore = |policy| {
            data_command(DataOperation::Restore, &store, Some(&backup), true, policy)
                .map_err(|error| error.code)
        };
        let invalid = Err("BACKUP_INVALID".to_owned());

        // An empty guild-settings backup must not wipe a populated case store.
        std::fs::write(&backup, r#"{"version":1,"kind":"guilds","data":{}}"#).unwrap();
        std::fs::write(&store, cases()).unwrap();
        assert_eq!(restore(RestorePolicy::Replace), invalid);
        assert_eq!(restore(RestorePolicy::Merge), invalid);
        assert_eq!(std::fs::read_to_string(&store).unwrap(), cases());

        // Nor may guild settings land in an empty or missing case store.
        std::fs::write(
            &backup,
            r#"{"version":1,"kind":"guilds","data":{"1":{"prefix":"?"}}}"#,
        )
        .unwrap();
        std::fs::write(&store, "{}").unwrap();
        assert_eq!(restore(RestorePolicy::Replace), invalid);
        std::fs::remove_file(&store).unwrap();
        assert_eq!(restore(RestorePolicy::Replace), invalid);
        assert!(!store.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn replace_restore_ignores_whatever_is_unreadable_in_the_store() {
        let dir = temp("unreadable");
        let store = dir.join("cases.json");
        let backup = dir.join("backup.json");
        std::fs::write(&store, cases()).unwrap();
        data_command(
            DataOperation::Backup,
            &store,
            Some(&backup),
            false,
            RestorePolicy::Replace,
        )
        .unwrap();
        let restore = |policy| {
            data_command(DataOperation::Restore, &store, Some(&backup), true, policy)
                .map(|report| report["guilds_after"].clone())
                .map_err(|error| error.code)
        };
        let damaged: [&[u8]; 2] = [
            b"\xff\xfe not utf-8",
            br#"{"1":{"next_id":2,"cases":[{"id":"one"}]}}"#,
        ];
        for content in damaged {
            std::fs::write(&store, content).unwrap();
            assert_eq!(
                restore(RestorePolicy::Merge),
                Err("STORE_CORRUPT".to_owned())
            );
            assert_eq!(std::fs::read(&store).unwrap(), content);
            assert_eq!(restore(RestorePolicy::Replace), Ok(json!(2)));
            assert_eq!(
                std::fs::read_to_string(&store)
                    .map(|text| text.contains("spam"))
                    .ok(),
                Some(true)
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn replace_restore_recovers_a_missing_or_damaged_store() {
        let dir = temp("recover");
        let store = dir.join("cases.json");
        let backup = dir.join("backup.json");
        std::fs::write(&store, cases()).unwrap();
        data_command(
            DataOperation::Backup,
            &store,
            Some(&backup),
            false,
            RestorePolicy::Replace,
        )
        .unwrap();
        let restore =
            |policy| data_command(DataOperation::Restore, &store, Some(&backup), true, policy);

        std::fs::write(&store, "{ truncated").unwrap();
        assert_eq!(
            restore(RestorePolicy::Merge).unwrap_err().code,
            "STORE_CORRUPT"
        );
        assert_eq!(restore(RestorePolicy::Replace).unwrap()["guilds_before"], 0);
        std::fs::remove_file(&store).unwrap();
        assert_eq!(restore(RestorePolicy::Replace).unwrap()["guilds_after"], 2);
        let restored: Value =
            serde_json::from_str(&std::fs::read_to_string(&store).unwrap()).unwrap();
        assert_eq!(restored, serde_json::from_str::<Value>(&cases()).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_bad_stores_and_backups() {
        let dir = temp("invalid");
        let run = |path: &Path, file: Option<&Path>| {
            let operation = if file.is_some() {
                DataOperation::Restore
            } else {
                DataOperation::Stats
            };
            data_command(operation, path, file, false, RestorePolicy::Replace)
                .unwrap_err()
                .code
        };
        assert_eq!(run(&dir.join("store.txt"), None), "DATA_FORMAT");
        assert_eq!(run(&dir.join("store.sqlite"), None), "UNSUPPORTED");
        assert_eq!(run(&dir.join("missing.json"), None), "DATA_FORMAT");
        let store = dir.join("cases.json");
        std::fs::write(&store, "[]").unwrap();
        assert_eq!(run(&store, None), "STORE_CORRUPT");
        std::fs::write(&store, cases()).unwrap();
        let backup = dir.join("backup.json");
        std::fs::write(&backup, r#"{"version":2,"kind":"cases","data":{}}"#).unwrap();
        assert_eq!(run(&store, Some(&backup)), "BACKUP_INVALID");
        std::fs::write(
            &backup,
            r#"{"version":1,"kind":"guilds","data":{"1":{"prefix":"?"}}}"#,
        )
        .unwrap();
        assert_eq!(run(&store, Some(&backup)), "BACKUP_INVALID");
        let broken =
            r#"{"version":1,"kind":"cases","data":{"1":{"next_id":2,"cases":[{"id":"one"}]}}}"#;
        std::fs::write(&backup, broken).unwrap();
        assert_eq!(run(&store, Some(&backup)), "BACKUP_INVALID");
        assert_eq!(std::fs::read_to_string(&store).unwrap(), cases());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
