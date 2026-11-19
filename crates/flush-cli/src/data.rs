//! `flush data`: inspect, back up and restore the JSON stores
//! (`guilds.json`, `cases.json`) without a config.

use std::io::Write;
use std::path::Path;

use serde_json::{json, Map, Value};

use crate::arguments::{DataOperation, RestorePolicy};
use crate::error::CliError;

const MAX_BACKUP_BYTES: u64 = 50_000_000;

type Store = Map<String, Value>;

fn read_store(path: &Path) -> Result<Store, CliError> {
    if path.extension().and_then(|e| e.to_str()) != Some("json") {
        let sqlite = matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("sqlite" | "db")
        );
        return Err(if sqlite {
            CliError::new(
                "UNSUPPORTED",
                "SQLite stores are not available in Flush yet (see docs/PARITY.md).",
            )
        } else {
            CliError::new("DATA_FORMAT", "The store must end in .json.")
        });
    }
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

/// `cases` when every guild entry holds a `cases` list, otherwise `guilds`.
fn kind(store: &Store) -> &'static str {
    let cases = !store.is_empty()
        && store
            .values()
            .all(|entry| entry.get("cases").is_some_and(Value::is_array));
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

fn stats(store: &Store, bytes: u64) -> Value {
    let kind = kind(store);
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
    let store = read_store(input)?;
    match operation {
        DataOperation::Stats => Ok(stats(&store, std::fs::metadata(input)?.len())),
        DataOperation::Backup => {
            let backup = json!({
                "version": 1,
                "kind": kind(&store),
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
            if !store.is_empty() && backup_kind != kind(&store) {
                return Err(CliError::new(
                    "BACKUP_INVALID",
                    format!(
                        "The backup holds {backup_kind} but the store holds {}.",
                        kind(&store)
                    ),
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

    const CASES: &str =
        r#"{"1":{"next_id":3,"cases":[{"id":1},{"id":2}]},"2":{"next_id":2,"cases":[{"id":1}]}}"#;

    #[test]
    fn stats_detect_the_store_kind() {
        let dir = temp("stats");
        let cases = dir.join("cases.json");
        std::fs::write(&cases, CASES).unwrap();
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
        std::fs::write(&store, CASES).unwrap();
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
        assert_eq!(restored, serde_json::from_str::<Value>(CASES).unwrap());
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
        std::fs::write(&store, CASES).unwrap();
        let backup = dir.join("backup.json");
        std::fs::write(&backup, r#"{"version":2,"kind":"cases","data":{}}"#).unwrap();
        assert_eq!(run(&store, Some(&backup)), "BACKUP_INVALID");
        std::fs::write(&backup, r#"{"version":1,"kind":"guilds","data":{}}"#).unwrap();
        assert_eq!(run(&store, Some(&backup)), "BACKUP_INVALID");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
