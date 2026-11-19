//! `flush data`: inspect, back up and restore the JSON stores
//! (`guilds.json`, `cases.json`) without a config.

use std::io::Write;
use std::path::Path;

use flush_framework::{GuildConfig, ModCase};
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
    match serde_json::from_str(&std::fs::read_to_string(path)?) {
        Ok(Value::Object(store)) => Ok(store),
        _ => Err(CliError::new(
            "STORE_CORRUPT",
            "The store is not a JSON object keyed by guild id.",
        )),
    }
}

/// Store to restore over. A `replace` restore is how a deleted or damaged
/// store is recovered, so there it counts as empty; `merge` needs it readable.
fn read_restore_target(path: &Path, policy: RestorePolicy) -> Result<Store, CliError> {
    check_extension(path)?;
    match read_store(path) {
        Err(error)
            if policy == RestorePolicy::Replace
                && matches!(error.code.as_str(), "DATA_FORMAT" | "STORE_CORRUPT")
                && !path.is_dir() =>
        {
            Ok(Store::new())
        }
        other => other,
    }
}

/// `cases` when every guild entry holds a `cases` list, otherwise `guilds`.
/// An empty store has no entries to tell, so its file name decides.
fn kind(store: &Store, path: &Path) -> &'static str {
    let cases = if store.is_empty() {
        path.file_stem().is_some_and(|stem| stem == "cases")
    } else {
        store
            .values()
            .all(|entry| entry.get("cases").is_some_and(Value::is_array))
    };
    if cases {
        "cases"
    } else {
        "guilds"
    }
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

/// Checks every entry the way the stores read them: one entry the bot cannot
/// parse makes it discard the whole file on its next load.
fn validate_entries(kind: &str, data: &Store) -> Result<(), CliError> {
    for (guild, entry) in data {
        let valid = if kind == "cases" {
            entry.get("next_id").is_some_and(Value::is_u64)
                && entry
                    .get("cases")
                    .and_then(Value::as_array)
                    .is_some_and(|cases| {
                        cases
                            .iter()
                            .all(|case| serde_json::from_value::<ModCase>(case.clone()).is_ok())
                    })
        } else {
            serde_json::from_value::<GuildConfig>(entry.clone()).is_ok()
        };
        if !valid {
            return Err(CliError::new(
                "BACKUP_INVALID",
                format!("The backup entry for guild {guild} is not a valid {kind} record."),
            ));
        }
    }
    Ok(())
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
            validate_entries(kind, data)?;
            Ok((kind.to_owned(), data.clone()))
        }
        _ => Err(invalid(
            "The backup must have version 1, a kind of guilds or cases, and a data object.",
        )),
    }
}

fn write_atomic(path: &Path, store: &Store) -> Result<(), CliError> {
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
    let store = match operation {
        DataOperation::Restore => read_restore_target(input, policy)?,
        _ => read_store(input)?,
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
            // Only entries can clash: an empty side fits either kind.
            let store_kind = kind(&store, input);
            if !store.is_empty() && !data.is_empty() && backup_kind != store_kind {
                return Err(CliError::new(
                    "BACKUP_INVALID",
                    format!("The backup holds {backup_kind} but the store holds {store_kind}."),
                ));
            }
            let before = store.len();
            let restored = match policy {
                RestorePolicy::Replace => data,
                RestorePolicy::Merge => {
                    let mut merged = store;
                    merged.extend(data);
                    merged
                }
            };
            if apply {
                write_atomic(input, &restored)?;
            }
            Ok(json!({
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
