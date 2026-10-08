//! Taking turns with the other `local-app-builder` processes on one data root.
//!
//! Each client starts its own `local-app-builder mcp`, and the service keeps the store in memory and was written for one
//! process, so only one may have the data root loaded at a time. A process takes the writer lock and loads the root when
//! a call needs it, and gives both up shortly after its last call finishes: the next client's call then gets its turn.
//! The loaded root is never read by a process that does not hold the lock. Loading is not read-only (it sweeps what a
//! crashed create left behind), so a process that loaded the root while another was creating an app could remove that
//! app's half-made directory.
//!
//! A call that cannot get its turn waits up to [`LEASE_WAIT`], and then fails with a message that says who holds the
//! root. Work in progress (a build, a running app) keeps the lease for as long as it lasts, so the other client's
//! calls wait for it rather than corrupting it.

use crate::local_host::{HostConfig, LocalHost};
use crate::writer_lock::{Attempt, Holder, WriterLock};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// How long a call waits for its turn.
pub const LEASE_WAIT: Duration = Duration::from_secs(30);
/// How long the root stays loaded after the last call finishes.
pub const LEASE_IDLE: Duration = Duration::from_millis(500);

/// Why a turn was not had.
#[derive(Debug)]
pub enum LeaseError {
    /// Another process holds the data root.
    Busy(Holder),
    /// The data root could not be opened.
    Open(String),
}

impl LeaseError {
    /// The message a model and a person read.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Busy(holder) => format!(
                "data_root_busy: another local-app-builder process ({}) is using this data root and did not finish within {} s. \
                 Nothing was changed. Try again in a moment; if it keeps happening, another client's Local App session is \
                 doing long work (a build, for example) and has to finish or be closed first.",
                holder.describe(),
                LEASE_WAIT.as_secs()
            ),
            Self::Open(message) => message.clone(),
        }
    }
}

struct Held {
    host: Arc<LocalHost>,
    _lock: WriterLock,
}

struct State {
    held: Option<Held>,
    users: usize,
    epoch: u64,
}

/// A process's claim on the data root, taken lazily and given back when idle.
pub struct Lease {
    root: PathBuf,
    config: HostConfig,
    wait: Duration,
    idle: Duration,
    state: Mutex<State>,
}

/// One use of the loaded root. The lease is released shortly after the last use is dropped.
pub struct Use {
    lease: Arc<Lease>,
    host: Arc<LocalHost>,
}

impl Use {
    /// The loaded data root.
    #[must_use]
    pub fn host(&self) -> &Arc<LocalHost> {
        &self.host
    }
}

impl Drop for Use {
    fn drop(&mut self) {
        let lease = Arc::clone(&self.lease);
        // A runtime is the only place this can happen outside tests; without one there is nothing to release into.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move { lease.finish_one().await });
        }
    }
}

impl Lease {
    /// A lease on `root` with the default timings.
    #[must_use]
    pub fn new(root: PathBuf, config: HostConfig) -> Arc<Self> {
        Self::with_timings(root, config, LEASE_WAIT, LEASE_IDLE)
    }

    /// A lease with its own timings (tests).
    #[must_use]
    pub fn with_timings(
        root: PathBuf,
        config: HostConfig,
        wait: Duration,
        idle: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            root,
            config,
            wait,
            idle,
            state: Mutex::new(State {
                held: None,
                users: 0,
                epoch: 0,
            }),
        })
    }

    /// Get a turn: the loaded data root, held by this process until the returned [`Use`] and every other are dropped
    /// and the idle time has passed.
    ///
    /// # Errors
    /// Another process holds the root past the wait, or the root cannot be opened.
    pub async fn acquire(self: &Arc<Self>) -> Result<Use, LeaseError> {
        let mut state = self.state.lock().await;
        if let Some(host) = state.held.as_ref().map(|held| Arc::clone(&held.host)) {
            state.users += 1;
            state.epoch += 1;
            return Ok(Use {
                lease: Arc::clone(self),
                host,
            });
        }
        let deadline = Instant::now() + self.wait;
        loop {
            match WriterLock::try_acquire(&self.root).map_err(LeaseError::Open)? {
                Attempt::Acquired(lock) => {
                    let host = Arc::new(
                        LocalHost::open(&self.root, &self.config)
                            .await
                            .map_err(LeaseError::Open)?,
                    );
                    state.held = Some(Held {
                        host: Arc::clone(&host),
                        _lock: lock,
                    });
                    state.users = 1;
                    state.epoch += 1;
                    return Ok(Use {
                        lease: Arc::clone(self),
                        host,
                    });
                }
                Attempt::Held(holder) if Instant::now() >= deadline => {
                    return Err(LeaseError::Busy(holder))
                }
                Attempt::Held(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    }

    /// Whether this process currently holds the data root.
    pub async fn is_held(&self) -> bool {
        self.state.lock().await.held.is_some()
    }

    async fn finish_one(self: Arc<Self>) {
        let epoch = {
            let mut state = self.state.lock().await;
            state.users = state.users.saturating_sub(1);
            if state.users > 0 {
                return;
            }
            state.epoch += 1;
            state.epoch
        };
        tokio::time::sleep(self.idle).await;
        let mut state = self.state.lock().await;
        // Nothing used the root since, and nothing it started is still running.
        if state.users == 0 && state.epoch == epoch {
            if let Some(held) = &state.held {
                if held.host.has_running_work().await {
                    // Check again later rather than keep the root forever or drop work on the floor.
                    drop(state);
                    let lease = Arc::clone(&self);
                    tokio::spawn(async move { lease.finish_one_again(epoch).await });
                    return;
                }
            }
            state.held = None;
        }
    }

    async fn finish_one_again(self: Arc<Self>, epoch: u64) {
        loop {
            tokio::time::sleep(Duration::from_secs(2).max(self.idle)).await;
            let mut state = self.state.lock().await;
            if state.users > 0 || state.epoch != epoch {
                return;
            }
            let running = match &state.held {
                Some(held) => held.host.has_running_work().await,
                None => return,
            };
            if !running {
                state.held = None;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease(root: &std::path::Path, wait: Duration, idle: Duration) -> Arc<Lease> {
        Lease::with_timings(root.to_path_buf(), HostConfig::default(), wait, idle)
    }

    #[tokio::test]
    async fn the_root_is_not_loaded_until_a_call_needs_it_and_is_given_back_when_idle() {
        let root = tempfile::tempdir().unwrap();
        let lease = lease(
            root.path(),
            Duration::from_secs(1),
            Duration::from_millis(100),
        );
        assert!(!lease.is_held().await);
        let turn = lease.acquire().await.unwrap();
        assert!(lease.is_held().await);
        // Another process cannot take it while this one holds it.
        assert!(matches!(
            WriterLock::try_acquire(root.path()).unwrap(),
            Attempt::Held(_)
        ));
        drop(turn);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            !lease.is_held().await,
            "idle for longer than the idle time: the root is given back"
        );
        assert!(matches!(
            WriterLock::try_acquire(root.path()).unwrap(),
            Attempt::Acquired(_)
        ));
    }

    #[tokio::test]
    async fn calls_close_together_share_one_load_and_the_idle_clock_restarts() {
        let root = tempfile::tempdir().unwrap();
        let lease = lease(
            root.path(),
            Duration::from_secs(1),
            Duration::from_millis(300),
        );
        let first = lease.acquire().await.unwrap();
        let host = Arc::as_ptr(first.host());
        drop(first);
        tokio::time::sleep(Duration::from_millis(150)).await;
        let second = lease.acquire().await.unwrap();
        assert_eq!(
            Arc::as_ptr(second.host()),
            host,
            "within the idle time the loaded root is reused"
        );
        drop(second);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            lease.is_held().await,
            "the idle time counts from the last call, not the first"
        );
    }

    #[tokio::test]
    async fn two_processes_take_turns_and_the_second_waits_for_the_first_to_go_idle() {
        let root = tempfile::tempdir().unwrap();
        let (a, b) = (
            lease(
                root.path(),
                Duration::from_secs(5),
                Duration::from_millis(100),
            ),
            lease(
                root.path(),
                Duration::from_secs(5),
                Duration::from_millis(100),
            ),
        );
        let first = a.acquire().await.unwrap();
        drop(first);
        let started = Instant::now();
        let second = b
            .acquire()
            .await
            .expect("the second gets its turn once the first is idle");
        assert!(
            started.elapsed() >= Duration::from_millis(50),
            "it had to wait for the first to let go"
        );
        assert!(b.is_held().await && !a.is_held().await);
        drop(second);
    }

    #[tokio::test]
    async fn a_process_that_never_gets_its_turn_is_told_who_has_the_root_and_never_loads_it() {
        let root = tempfile::tempdir().unwrap();
        let a = lease(
            root.path(),
            Duration::from_secs(5),
            Duration::from_millis(100),
        );
        let b = lease(
            root.path(),
            Duration::from_millis(300),
            Duration::from_millis(100),
        );
        let busy = a.acquire().await.unwrap(); // held for the whole test
        let error = match b.acquire().await {
            Err(error) => error,
            Ok(_) => panic!("the second must not get a turn while the first holds the root"),
        };
        let message = error.message();
        assert!(
            message.starts_with("data_root_busy:")
                && message.contains(&std::process::id().to_string()),
            "{message}"
        );
        assert!(message.contains("Nothing was changed"));
        assert!(!b.is_held().await);
        drop(busy);
    }

    #[tokio::test]
    async fn a_root_that_cannot_be_opened_is_an_error_and_leaves_the_lock_free() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-directory");
        std::fs::write(&file, b"x").unwrap();
        let lease = lease(&file, Duration::from_millis(200), Duration::from_millis(50));
        let error = lease
            .acquire()
            .await
            .err()
            .expect("a file where the root should be cannot be opened");
        assert!(matches!(error, LeaseError::Open(_)), "{error:?}");
        assert!(
            error.message().contains("not-a-directory"),
            "{}",
            error.message()
        );
        assert!(!lease.is_held().await);
    }
}
