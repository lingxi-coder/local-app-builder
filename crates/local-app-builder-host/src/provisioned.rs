//! The executor a command line hands the service: the pinned toolchain, checked once, then [`LocalExecutor`].
//!
//! A build must run the toolchain its contract names, so before the first command of a process the installed
//! toolchain is verified against the pins ([`Toolchains::verify`]: both programs hash to their receipt and report the
//! pinned versions). Verifying costs a second or two, so it happens once per process and its outcome is kept; a
//! toolchain that is missing or damaged fails every command with the same instruction, and the instruction is the
//! command that fixes it.

use crate::executor::{LocalExecutor, LocalExecutorConfig};
use crate::toolchain::{Spec, Status, Toolchains};
use async_trait::async_trait;
use local_app_builder_contracts::execution::{CommandOutcome, IsolatedCommand};
use local_app_builder_service::host::BuildExecutor;
use std::path::PathBuf;
use tokio::sync::OnceCell;

/// The service's [`BuildExecutor`] for a data root, backed by the toolchain installed under it.
pub struct ProvisionedExecutor {
    toolchains: Toolchains,
    spec: Spec,
    private_roots: Vec<PathBuf>,
    checked: OnceCell<Result<LocalExecutor, String>>,
}

impl ProvisionedExecutor {
    /// An executor for `spec`, installed under `toolchains`. `private_roots` are the directories a command may not
    /// read except under its mounts: the data root, and the home directory.
    #[must_use]
    pub fn new(toolchains: Toolchains, spec: Spec, private_roots: Vec<PathBuf>) -> Self {
        Self { toolchains, spec, private_roots, checked: OnceCell::new() }
    }

    async fn executor(&self) -> Result<&LocalExecutor, String> {
        let checked = self
            .checked
            .get_or_init(|| async {
                match self.toolchains.status(&self.spec).await {
                    Status::Ready(_) => Ok(LocalExecutor::new(LocalExecutorConfig::new(
                        self.toolchains.dir(&self.spec),
                        self.private_roots.clone(),
                    ))),
                    Status::NotInstalled => Err(format!(
                        "toolchain_not_installed: {} is not installed, and a build runs only that toolchain. Ask the person \
                         to run `local-app-builder toolchain install` (it downloads about 80 MB), then try again.",
                        self.spec.key
                    )),
                    Status::Damaged(why) => Err(format!(
                        "toolchain_damaged: {} is installed but does not match what was pinned ({why}). Ask the person to run \
                         `local-app-builder toolchain install` to replace it, then try again.",
                        self.spec.key
                    )),
                }
            })
            .await;
        checked.as_ref().map_err(Clone::clone)
    }
}

#[async_trait]
impl BuildExecutor for ProvisionedExecutor {
    async fn run(&self, command: IsolatedCommand) -> Result<CommandOutcome, String> {
        self.executor().await?.run(command).await
    }
}
