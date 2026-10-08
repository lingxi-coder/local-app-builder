//! The Local App service, assembled the way a command line hosts it.
//!
//! [`LocalHost`] is one loaded data root: the app store, the service's broker (which does the work behind the build,
//! dependency and workspace operations), the transport that dispatches operations to it, and the record of which plans
//! the person has approved. It is built while the writer lock is held and dropped before the lock is released, so
//! nothing in it outlives the right to change the data root.
//!
//! **Nobody but the person approves anything.** The service asks its host's [`HostEventSink`] whenever an action needs a
//! decision, and waits. The sink here answers those it can put to the person through the current call's [`Approver`] (a
//! dependency change), and refuses the rest at once: anything that wants a capability, a UI action, a profile or an MCP
//! proposal is something this host has no surface for, and an unanswered request is a call that hangs. A request that
//! arrives while no call is running has nobody to ask and is refused too.

use crate::mcp_protocol::{Approval, ApprovalRequest, Approver, CallContext};
use async_trait::async_trait;
use local_app_builder_contracts::approvals::{
    AuthorizationDecision, DependencyChangeConfirmationRequest, DependencyChangeKind,
};
use local_app_builder_service::broker::LocalAppsHostBroker;
use local_app_builder_service::host::{BuildExecutor, HostEvent, HostEventSink};
use local_app_builder_service::mcp_server::LocalAppsMcpTransport;
use local_app_builder_service::plan_approval::PlanApprovalLog;
use local_app_builder_service::template_catalog::PluginBundle;
use local_apps::clock::SystemClock;
use local_apps::events::NoopAppEventObserver;
use local_apps::AppService;
use sha2::{Digest, Sha256};
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, OnceLock, Weak};

tokio::task_local! {
    /// The call being served, for events that arrive from inside it.
    static CURRENT_CALL: CallContext;
}

/// Run `work` as the body of a call: events the service raises from inside it can reach the person through `context`.
pub async fn within_call<F: Future>(context: CallContext, work: F) -> F::Output {
    CURRENT_CALL.scope(context, work).await
}

/// The template catalog this build ships. The templates themselves are compiled into the service; the catalog is what
/// says which exist and what each is for, and its digest stands for the bundle in every selection the service journals.
pub struct CatalogBundle {
    sha256: String,
}

const CATALOG: &[u8] =
    include_bytes!("../../plugins/lingxi-local-app/assets/templates/catalog.json");

impl CatalogBundle {
    /// The bundle this build carries.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sha256: Sha256::digest(CATALOG)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        }
    }
}

impl Default for CatalogBundle {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginBundle for CatalogBundle {
    fn catalog_bytes(&self) -> &[u8] {
        CATALOG
    }

    fn bundle_sha256(&self) -> &str {
        &self.sha256
    }
}

/// What the person is shown when a dependency change needs their confirmation. All of it is the host's own description
/// of the change: the packages and versions are what the assistant proposed, and nothing else the assistant wrote is
/// shown, so it cannot dress the question up.
fn dependency_question(request: &DependencyChangeConfirmationRequest) -> ApprovalRequest {
    let mut message = format!(
        "Approve changing the dependencies of the Local App `{}`?\n\nProposed changes:\n",
        request.app_id
    );
    for change in &request.changes {
        let verb = match change.kind {
            DependencyChangeKind::Add => "add",
            DependencyChangeKind::Update => "update",
            DependencyChangeKind::Remove => "remove",
        };
        let version = change
            .version
            .as_deref()
            .map_or(String::new(), |v| format!("@{v}"));
        message.push_str(&format!(
            "  - {verb} {}{version} (cache: {}; download: {})\n",
            change.package, change.cache_status, change.download_status
        ));
    }
    message.push_str(&format!(
        "\nNothing has been downloaded yet ({}). If you approve, the packages are resolved and installed from the npm \
         registry.\nLicense risk: {}\nSBOM risk: {}\nInstall scripts blocked: {}\nNative addons blocked: {}\nRollback: {}\n",
        request.reason,
        request.license_risk,
        request.sbom_risk,
        if request.lifecycle_scripts_blocked { "yes" } else { "NO" },
        if request.native_addons_blocked { "yes" } else { "NO" },
        request.rollback_policy
    ));
    ApprovalRequest {
        message,
        question: "Approve this dependency change".into(),
    }
}

/// Where the service reports what it needs decided.
#[derive(Default)]
pub struct ApprovalSink {
    broker: OnceLock<Weak<LocalAppsHostBroker>>,
}

impl ApprovalSink {
    fn bind(&self, broker: &Arc<LocalAppsHostBroker>) {
        let _ = self.broker.set(Arc::downgrade(broker));
    }
}

#[async_trait]
impl HostEventSink for ApprovalSink {
    async fn emit(&self, event: HostEvent) {
        let Some(broker) = self.broker.get().and_then(Weak::upgrade) else {
            return;
        };
        // The call this event came from, if it came from one. Captured here, while still inside it.
        let approver: Option<Arc<dyn Approver>> = CURRENT_CALL
            .try_with(|call| Arc::clone(&call.approver))
            .ok();
        match event {
            HostEvent::DependencyChangeConfirmationRequested(request) => {
                let id = request.request_id.clone();
                tokio::spawn(async move {
                    let approved = match approver {
                        Some(approver) => {
                            approver.ask(dependency_question(&request)).await == Approval::Approved
                        }
                        None => false,
                    };
                    broker
                        .resolve_dependency_change_confirmation(&id, approved)
                        .await;
                });
            }
            HostEvent::McpProposalApprovalRequested(request) => {
                tokio::spawn(async move {
                    broker
                        .resolve_mcp_proposal_approval(&request.request_id, false)
                        .await;
                });
            }
            HostEvent::CapabilityRequested(request) => {
                tokio::spawn(async move {
                    broker
                        .resolve_capability(&request.request_id, AuthorizationDecision::Deny)
                        .await;
                });
            }
            // Everything else is told to the host and needs no answer.
            _ => {}
        }
    }
}

/// How the host is built.
#[derive(Clone, Default)]
pub struct HostConfig {
    /// Where builds and dependency installs run. `None`: this host cannot build.
    pub executor: Option<Arc<dyn BuildExecutor>>,
}

/// One loaded data root.
pub struct LocalHost {
    /// Dispatches host operations by name.
    pub transport: Arc<LocalAppsMcpTransport>,
    /// The record of plans the person approved in this session.
    pub plans: Arc<PlanApprovalLog>,
    /// Names this session; an approval is only good in the session that obtained it.
    pub session: String,
    broker: Arc<LocalAppsHostBroker>,
}

/// Installed memory, which sizes the build budget. Zero (the smallest budget) when it cannot be read.
async fn physical_memory_bytes() -> u64 {
    let output = tokio::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .await;
    output
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

impl LocalHost {
    /// Load the data root at `root` and assemble the service around it. The caller holds the writer lock.
    ///
    /// # Errors
    /// The data root cannot be opened or its stored state is corrupt.
    pub async fn open(root: &Path, config: &HostConfig) -> Result<Self, String> {
        let sink = Arc::new(ApprovalSink::default());
        let broker = LocalAppsHostBroker::new_with_physical_memory(
            root.to_path_buf(),
            sink.clone(),
            config.executor.clone(),
            false,
            None,
            physical_memory_bytes().await,
        );
        sink.bind(&broker);
        let service = Arc::new(
            AppService::load(root, Arc::new(SystemClock), Arc::new(NoopAppEventObserver))
                .await
                .map_err(|error| {
                    format!("cannot open the data root {}: {error}", root.display())
                })?,
        );
        let attach = |what: &str| format!("{what} was attached twice");
        broker
            .attach_service(Arc::clone(&service))
            .map_err(|_| attach("the service"))?;
        broker
            .attach_plugin_bundle(Arc::new(CatalogBundle::new()))
            .map_err(|_| attach("the template catalog"))?;

        let plans = Arc::new(PlanApprovalLog::default());
        let session = format!(
            "cli-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        );
        let transport = LocalAppsMcpTransport::new(root.to_path_buf());
        transport
            .attach_service(service)
            .map_err(|_| attach("the service"))?;
        transport
            .attach_host(broker.clone())
            .map_err(|_| attach("the host"))?;
        transport
            .attach_plan_approval_log(Arc::clone(&plans))
            .map_err(|_| attach("the plan record"))?;
        let provided = session.clone();
        transport
            .attach_session_provider(Arc::new(move || Some(provided.clone())))
            .map_err(|_| attach("the session"))?;
        transport
            .attach_plugin_availability(Arc::new(|| Box::pin(async { true })))
            .map_err(|_| attach("the plugin probe"))?;
        Ok(Self {
            transport: Arc::new(transport),
            plans,
            session,
            broker,
        })
    }

    /// Whether anything started by this host is still running and needs the data root to stay held.
    pub async fn has_running_work(&self) -> bool {
        self.broker.has_active_runtimes().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use local_app_builder_contracts::approvals::DependencyChangeReview;

    fn request() -> DependencyChangeConfirmationRequest {
        DependencyChangeConfirmationRequest {
            request_id: "r1".into(),
            app_id: "abc123".into(),
            reason: "pre_resolution_no_network".into(),
            changes: vec![
                DependencyChangeReview {
                    kind: DependencyChangeKind::Add,
                    package: "dayjs".into(),
                    version: Some("1.11.13".into()),
                    cache_status: "unknown".into(),
                    download_status: "may_be_required".into(),
                },
                DependencyChangeReview {
                    kind: DependencyChangeKind::Remove,
                    package: "zod".into(),
                    version: None,
                    cache_status: "not_needed".into(),
                    download_status: "not_required".into(),
                },
            ],
            license_risk: "unknown_until_resolution".into(),
            sbom_risk: "unknown_until_resolution".into(),
            lifecycle_scripts_blocked: true,
            native_addons_blocked: false,
            rollback_policy: "rollback_on_validation_failure".into(),
        }
    }

    #[test]
    fn the_dependency_question_names_every_package_and_every_protection_that_is_off() {
        let question = dependency_question(&request());
        let message = &question.message;
        assert!(
            message.contains("`abc123`")
                && message.contains("add dayjs@1.11.13")
                && message.contains("remove zod"),
            "{message}"
        );
        assert!(
            message.contains("Install scripts blocked: yes"),
            "{message}"
        );
        assert!(
            message.contains("Native addons blocked: NO"),
            "a protection that is off must say so: {message}"
        );
        assert!(
            !message.contains("Reason given by the assistant"),
            "the reason is the host's policy code, not the assistant's words"
        );
    }

    #[test]
    fn the_bundle_digest_is_the_catalogs_own() {
        let bundle = CatalogBundle::new();
        assert_eq!(bundle.bundle_sha256().len(), 64);
        assert!(std::str::from_utf8(bundle.catalog_bytes())
            .unwrap()
            .contains("react-dom-r4"));
        assert_eq!(
            bundle.bundle_sha256(),
            CatalogBundle::new().bundle_sha256(),
            "stable across loads"
        );
    }
}
