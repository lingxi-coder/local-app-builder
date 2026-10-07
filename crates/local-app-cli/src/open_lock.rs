//! One process at a time opens a data root.
//!
//! Each client starts its own `local-app mcp`, so several processes can open the same data root together. The
//! service was written for one process: its startup recovery creates and locks a directory under the system temporary
//! directory, and two processes doing that at once fail each other (`File exists`; `scaffold-recovery.lock` not
//! found). The lock is held while the store opens and released before the first message is served, so it costs a
//! client nothing once it is running.
//!
//! This is *not* the writer lock. Which process may change the data root, and what the others do, is decided with
//! the local host (T1); until then no tool that changes anything is served (see `mcp_backend`).

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The file inside the data root that carries the lock.
pub const OPEN_LOCK_FILE: &str = "open.lock";

/// How long to wait for another process to finish opening before giving up.
pub const OPEN_LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// Held while the data root is being opened; released on drop.
#[derive(Debug)]
pub struct OpenLock {
    file: File,
    path: PathBuf,
}

impl OpenLock {
    /// Wait for the lock, creating the data root and the lock file if need be.
    ///
    /// # Errors
    /// The root or the lock file could not be created, or the lock was not free within `timeout`.
    pub fn acquire(root: &Path, timeout: Duration) -> Result<Self, String> {
        std::fs::create_dir_all(root).map_err(|error| format!("create {}: {error}", root.display()))?;
        let path = root.join(OPEN_LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|error| format!("open {}: {error}", path.display()))?;
        let deadline = Instant::now() + timeout;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { file, path }),
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(format!(
                        "another local-app process has held {} for {} s while opening the same data root",
                        path.display(),
                        timeout.as_secs()
                    ));
                }
                Err(TryLockError::Error(error)) => return Err(format!("lock {}: {error}", path.display())),
            }
        }
    }

    /// The lock file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for OpenLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_acquirer_waits_for_the_first_and_then_gets_it() {
        let root = tempfile::tempdir().unwrap();
        let first = OpenLock::acquire(root.path(), Duration::from_secs(5)).unwrap();
        assert_eq!(first.path(), root.path().join(OPEN_LOCK_FILE));
        let path = root.path().to_path_buf();
        let waiter = std::thread::spawn(move || {
            let started = Instant::now();
            let lock = OpenLock::acquire(&path, Duration::from_secs(5)).unwrap();
            drop(lock);
            started.elapsed()
        });
        std::thread::sleep(Duration::from_millis(150));
        drop(first);
        assert!(waiter.join().unwrap() >= Duration::from_millis(100), "the waiter was held back");
    }

    #[test]
    fn a_lock_that_never_frees_times_out_with_a_message_that_names_the_file() {
        let root = tempfile::tempdir().unwrap();
        let _held = OpenLock::acquire(root.path(), Duration::from_secs(1)).unwrap();
        let error = OpenLock::acquire(root.path(), Duration::from_millis(100)).unwrap_err();
        assert!(error.contains(OPEN_LOCK_FILE) && error.contains("another local-app process"), "{error}");
    }

    #[test]
    fn the_root_is_created_and_the_lock_is_reusable_after_release() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("nested/root");
        drop(OpenLock::acquire(&root, Duration::from_secs(1)).unwrap());
        assert!(root.is_dir());
        drop(OpenLock::acquire(&root, Duration::from_secs(1)).unwrap());
    }
}
