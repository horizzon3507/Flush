//! Advisory lock that says a JSON store is in use.
//!
//! [`GuildStore`](crate::GuildStore) and [`CaseStore`](crate::CaseStore) keep
//! their file in memory and write it back whole, so replacing the file under
//! a running bot is undone by the bot's next save. Each store holds a shared
//! lock on `<store>.lock` for as long as it lives; a tool that replaces the
//! file takes the exclusive lock first and is refused while a store is open.
//!
//! The lock sits on a file next to the store, not on the store itself: every
//! save renames a new file into place, which would drop a lock held on it.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use fs4::fs_std::FileExt;

/// A held lock; dropping it releases the store.
#[derive(Debug)]
pub struct StoreLock {
    file: File,
}

impl StoreLock {
    /// Lock file of the store at `store`: the same path with `.lock` appended.
    pub fn path_for(store: &Path) -> PathBuf {
        let mut name = store.as_os_str().to_owned();
        name.push(".lock");
        PathBuf::from(name)
    }

    /// Marks the store as in use. Any number of holders may share it; this
    /// waits while an exclusive holder is replacing the file.
    pub fn shared(store: &Path) -> io::Result<Self> {
        let file = open(store)?;
        FileExt::lock_shared(&file)?;
        Ok(Self { file })
    }

    /// Claims the store for a whole-file replacement. `Ok(None)` means a bot
    /// (or another tool) has the store open, so the file must not be
    /// replaced.
    pub fn try_exclusive(store: &Path) -> io::Result<Option<Self>> {
        let file = open(store)?;
        match FileExt::try_lock_exclusive(&file) {
            Ok(true) => Ok(Some(Self { file })),
            Ok(false) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn open(store: &Path) -> io::Result<File> {
    if let Some(parent) = store.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let path = StoreLock::path_for(store);
    let created = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path);
    // A lock file made by another account may be readable only; reading is
    // enough to hold the lock.
    created.or_else(|error| File::open(&path).map_err(|_| error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CaseStore, CaseType, GuildStore};

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("flush-lock-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn an_open_store_refuses_a_replacement() {
        let dir = dir("stores");
        let cases = dir.join("cases.json");
        let guilds = dir.join("guilds.json");

        let mut store = CaseStore::new(cases.clone());
        let second = CaseStore::new(cases.clone());
        store.add("g", CaseType::Warn, "u", "m", "spam").unwrap();
        assert!(StoreLock::try_exclusive(&cases).unwrap().is_none());
        drop(store);
        assert!(StoreLock::try_exclusive(&cases).unwrap().is_none());
        drop(second);
        assert!(StoreLock::try_exclusive(&cases).unwrap().is_some());

        let store = GuildStore::new(guilds.clone());
        assert!(StoreLock::try_exclusive(&guilds).unwrap().is_none());
        // Each store file has its own lock.
        assert!(StoreLock::try_exclusive(&cases).unwrap().is_some());
        drop(store);
        assert!(StoreLock::try_exclusive(&guilds).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_lock_file_still_locks() {
        use std::os::unix::fs::PermissionsExt;
        let dir = dir("readonly");
        let cases = dir.join("cases.json");
        drop(StoreLock::try_exclusive(&cases).unwrap());
        let lock = StoreLock::path_for(&cases);
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o444)).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let store = CaseStore::new(cases.clone());
        let held = StoreLock::try_exclusive(&cases);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(held.unwrap().is_none());
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_store_waits_for_a_replacement_to_finish() {
        let dir = dir("wait");
        let cases = dir.join("cases.json");
        let held = StoreLock::try_exclusive(&cases).unwrap().unwrap();
        assert!(StoreLock::try_exclusive(&cases).unwrap().is_none());

        let (opened, wait) = std::sync::mpsc::channel();
        let path = cases.clone();
        let reader = std::thread::spawn(move || {
            let mut store = CaseStore::new(path);
            opened.send(()).unwrap();
            store.active_warns("g", "u")
        });
        assert!(wait
            .recv_timeout(std::time::Duration::from_millis(200))
            .is_err());

        // What a restore writes while it holds the lock is what the store reads.
        std::fs::write(
            &cases,
            r#"{"g":{"next_id":2,"cases":[{"id":1,"type":"warn","target_id":"u","moderator_id":"m","reason":"r","timestamp":1,"reverted":false}]}}"#,
        )
        .unwrap();
        drop(held);
        assert_eq!(reader.join().unwrap(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
