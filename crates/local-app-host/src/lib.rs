//! What a Mac provides the Local App service.
//!
//! The service decides what must be isolated and how; a host provides the isolation and says what it did. This crate
//! is the Mac's half of the build seam: [`LocalExecutor`] runs the service's build and dependency-install commands in
//! a seatbelt sandbox, with the guest paths the service speaks translated to this machine's, and with network and
//! memory limits it actually enforces.
#![forbid(unsafe_code)]

mod executor;
mod path_map;
mod seatbelt;
mod watchdog;

pub use executor::{
    LocalExecutor, LocalExecutorConfig, KILLED_EXIT_CODE, OUTPUT_CAP_BYTES, TIMED_OUT_EXIT_CODE,
};
pub use path_map::{PathMap, Toolchain};
pub use seatbelt::{Policy, Profile, SANDBOX_EXEC};
pub use watchdog::{group_rss_kib, SAMPLE_INTERVAL};
