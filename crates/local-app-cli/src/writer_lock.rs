//! At most one process changes a data root.
//!
//! The service keeps the store in memory and writes it back, and it was written for one process; two writers would
//! each overwrite what the other saved. Each client starts its own `local-app mcp` against the same root, so the
//! rule has to be enforced between processes: the first one that needs to change anything takes this lock and keeps
//! it until it exits, and every other process can read but not change.
//!
//! The lock is an advisory `flock`, so it is released by the operating system when its holder dies, however it dies.
//! There is no stale lock to clean up and no pid to probe for liveness. While it holds the lock the writer records who
//! it is in the file, so that a process that cannot have the lock can say whom to wait for. That record is a courtesy
//! to the reader of an error message; the lock is the only authority.
//!
//! The data root is loaded only while this lock is held (see `lease`).

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The file inside the data root that carries the lock.
pub const WRITER_LOCK_FILE: &str = "writer.lock";

/// What the holder wrote about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    /// The holder's process id, if it recorded one that can be read.
    pub pid: Option<u32>,
    /// When it took the lock, in seconds since the Unix epoch.
    pub since_unix: Option<u64>,
}

impl Holder {
    /// How a message names the holder.
    #[must_use]
    pub fn describe(&self) -> String {
        match (self.pid, self.since_unix) {
            (Some(pid), Some(since)) => format!("process {pid}, since unix time {since}"),
            (Some(pid), None) => format!("process {pid}"),
            _ => "another process".to_string(),
        }
    }
}

/// The outcome of asking for the lock without waiting.
#[derive(Debug)]
pub enum Attempt {
    /// This process now holds it until the value is dropped.
    Acquired(WriterLock),
    /// Another process holds it.
    Held(Holder),
}

/// Held for as long as this process may change the data root; released on drop.
#[derive(Debug)]
pub struct WriterLock {
    file: File,
    path: PathBuf,
}

impl WriterLock {
    /// Take the lock if it is free, without waiting.
    ///
    /// # Errors
    /// The root or the lock file could not be created or locked for a reason other than another holder.
    pub fn try_acquire(root: &Path) -> Result<Attempt, String> {
        std::fs::create_dir_all(root).map_err(|error| format!("create {}: {error}", root.display()))?;
        let path = root.join(WRITER_LOCK_FILE);
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| format!("open {}: {error}", path.display()))?;
        match file.try_lock() {
            Ok(()) => {
                let since = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let record = format!("{{\"pid\":{},\"since_unix\":{since}}}\n", std::process::id());
                // A failure to write the courtesy record must not cost the lock.
                let _ = file
                    .set_len(0)
                    .and_then(|()| file.seek(SeekFrom::Start(0)))
                    .and_then(|_| file.write_all(record.as_bytes()))
                    .and_then(|()| file.flush());
                Ok(Attempt::Acquired(Self { file, path }))
            }
            Err(TryLockError::WouldBlock) => Ok(Attempt::Held(read_holder(&mut file))),
            Err(TryLockError::Error(error)) => Err(format!("lock {}: {error}", path.display())),
        }
    }

    /// The lock file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn read_holder(file: &mut File) -> Holder {
    let mut text = String::new();
    let read = file.seek(SeekFrom::Start(0)).and_then(|_| file.read_to_string(&mut text));
    let parsed: Option<serde_json::Value> = read.ok().and_then(|_| serde_json::from_str(&text).ok());
    let field = |name: &str| parsed.as_ref().and_then(|v| v.get(name)).and_then(serde_json::Value::as_u64);
    Holder { pid: field("pid").and_then(|pid| u32::try_from(pid).ok()), since_unix: field("since_unix") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acquired(root: &Path) -> WriterLock {
        match WriterLock::try_acquire(root).unwrap() {
            Attempt::Acquired(lock) => lock,
            Attempt::Held(holder) => panic!("expected the lock to be free, held by {holder:?}"),
        }
    }

    #[test]
    fn the_first_process_gets_the_lock_and_the_second_is_told_who_has_it() {
        let root = tempfile::tempdir().unwrap();
        let first = acquired(root.path());
        assert_eq!(first.path(), root.path().join(WRITER_LOCK_FILE));
        let Attempt::Held(holder) = WriterLock::try_acquire(root.path()).unwrap() else {
            panic!("the lock was taken twice");
        };
        assert_eq!(holder.pid, Some(std::process::id()));
        assert!(holder.since_unix.is_some_and(|t| t > 1_600_000_000), "{holder:?}");
        assert!(holder.describe().contains(&std::process::id().to_string()));
    }

    #[test]
    fn dropping_the_lock_frees_it_and_the_next_holder_replaces_the_record() {
        let root = tempfile::tempdir().unwrap();
        drop(acquired(root.path()));
        let second = acquired(root.path());
        let text = std::fs::read_to_string(second.path()).unwrap();
        assert_eq!(text.matches("pid").count(), 1, "the record was appended to, not replaced: {text:?}");
    }

    #[test]
    fn a_holder_whose_record_cannot_be_read_is_still_a_holder() {
        let root = tempfile::tempdir().unwrap();
        let held = acquired(root.path());
        std::fs::write(held.path(), b"not json").unwrap();
        let Attempt::Held(holder) = WriterLock::try_acquire(root.path()).unwrap() else {
            panic!("a garbled record must not release the lock");
        };
        assert_eq!(holder, Holder { pid: None, since_unix: None });
        assert_eq!(holder.describe(), "another process");
    }
}
