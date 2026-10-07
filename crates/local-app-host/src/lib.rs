//! What a Mac provides the Local App service.
//!
//! The service decides what must be isolated and how; a host provides the isolation and says what it did. This crate
//! is the Mac's half of the build seam: [`LocalExecutor`] runs the service's build and dependency-install commands in
//! a seatbelt sandbox, with the guest paths the service speaks translated to this machine's, and with network and
//! memory limits it actually enforces. [`Toolchains`] provides the exact Node and pnpm those commands run, from pinned
//! archives.
#![forbid(unsafe_code)]

mod executor;
mod path_map;
mod seatbelt;
mod toolchain;
mod watchdog;

pub use executor::{
    LocalExecutor, LocalExecutorConfig, KILLED_EXIT_CODE, OUTPUT_CAP_BYTES, TIMED_OUT_EXIT_CODE,
};
pub use path_map::{PathMap, Toolchain};
pub use seatbelt::{Policy, Profile, SANDBOX_EXEC};
pub use toolchain::{
    Artifact, Platform, ProgramRecord, Receipt, Source, Spec, Status, ToolchainError, Toolchains, INSTALL_LOCK_FILE,
    NODE_VERSION_OUTPUT, PNPM_VERSION_OUTPUT, RECEIPT_FILE, TOOLCHAIN_KEY,
};
pub use watchdog::{group_rss_kib, SAMPLE_INTERVAL};
