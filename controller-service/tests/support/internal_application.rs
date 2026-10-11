//! Original application fixture: controlled host effects, not a second controller.
#[path = "../../../sandbox-protocol/tests/support/lifecycle.rs"]
pub mod wire;
#[allow(
    dead_code,
    reason = "original worker helpers serve several component tests"
)]
#[path = "mod.rs"]
mod workflow;
use sandbox_controller_core as core;
use sandproof_controller_service::{
    CreateOptions, HostSlotContext, InternalInboxConfig, InternalTemplate, NeverCrash,
    WorkerTransportError,
    application::{
        ApplicationConfig, ApplicationError, Clock, ControllerApplication, DataRelay, HostCapacity,
        HostSlots, LaunchConfiguration, TemplateArtifacts, WorkEvidence, WorkExecution,
        WorkRecovery,
    },
    physical_storage::{PhysicalPool, PhysicalStorageProfile},
    templates::{ComputeLimits, ResourceShape, TemplateArtifact, TemplateBinding},
};
use sandproof_protocol::{
    WorkAction, WorkReceipt,
    internal::lifecycle::{Intent, LifecycleRequest},
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
        mpsc,
    },
};
pub use workflow::complete_fixture;

pub struct TestClock(pub AtomicI64);
impl Clock for TestClock {
    fn now_ms(&self) -> Result<i64, ApplicationError> {
        Ok(self.0.load(Ordering::SeqCst))
    }
}
#[derive(Default)]
pub struct State {
    pub actions: Vec<WorkAction<core::EffectId>>,
    pub configurations: BTreeMap<u64, (String, CreateOptions)>,
    pub lose_activation: bool,
    pub block_prepare: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    pub release_unknown: bool,
}
pub struct TestHost(pub Arc<Mutex<State>>);
impl WorkEvidence for TestHost {
    type Evidence = workflow::FakeHostEvidence;
}
impl WorkExecution for TestHost {
    fn execute(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<Self::Evidence>, WorkerTransportError> {
        let mut state = self.0.lock().unwrap();
        state.actions.push(action.clone());
        if matches!(action.command.command(), core::Command::PrepareVm { .. })
            && let Some((entered, release)) = state.block_prepare.take()
        {
            entered.send(()).unwrap();
            release
                .recv_timeout(std::time::Duration::from_secs(15))
                .unwrap();
        }
        if state.lose_activation
            && matches!(action.command.command(), core::Command::ActivateVm { .. })
        {
            return Err(WorkerTransportError::ReplyLost);
        }
        workflow::worker(action)
    }
}
impl WorkRecovery for TestHost {
    fn observe(&self, action: &WorkAction<core::EffectId>) -> Option<WorkReceipt<Self::Evidence>> {
        self.0
            .lock()
            .unwrap()
            .actions
            .contains(action)
            .then(|| workflow::success(action))
    }
    fn reclaim_incomplete_prepare(
        &mut self,
        _: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Busy)
    }
    fn reclaim_incomplete_activation(
        &mut self,
        _: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        Ok(())
    }
    fn reclaim_incomplete_cleanup(
        &mut self,
        _: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        Ok(())
    }
}
impl HostSlots for TestHost {
    fn bind_host_slot(&mut self, _: &HostSlotContext, _: bool) -> Result<(), ApplicationError> {
        Ok(())
    }
    fn release_host_slot(&mut self, _: &HostSlotContext) -> Result<(), ApplicationError> {
        if self.0.lock().unwrap().release_unknown {
            Err(ApplicationError::Busy)
        } else {
            Ok(())
        }
    }
}
impl DataRelay for TestHost {
    fn acquire_data_gate(
        &self,
        _: core::SandboxId,
        _: core::VmId,
        _: core::Generation,
    ) -> Result<Option<std::fs::File>, ApplicationError> {
        let gate = tempfile::tempfile().map_err(|_| ApplicationError::Busy)?;
        gate.try_lock_shared().map_err(|_| ApplicationError::Busy)?;
        Ok(Some(gate))
    }
    fn data_gate_drained(&self, _: &HostSlotContext) -> Result<bool, ApplicationError> {
        Ok(true)
    }
}
impl HostCapacity for TestHost {
    fn available_disk_bytes(&self) -> Result<u64, ApplicationError> {
        Ok(u64::MAX)
    }
    fn validate_physical_storage(
        &self,
        profile: &PhysicalStorageProfile,
        _: &PhysicalPool,
    ) -> Result<(), ApplicationError> {
        profile.validate().map_err(|_| ApplicationError::Busy)
    }
}
impl LaunchConfiguration for TestHost {
    fn put_secret(&mut self, _: core::SandboxId, _: &str) -> Result<(), ApplicationError> {
        Err(ApplicationError::Invalid)
    }
    fn has_secret(&self, sandbox: core::SandboxId) -> Result<bool, ApplicationError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .configurations
            .contains_key(&sandbox.0))
    }
    fn token_for(&self, sandbox: core::SandboxId) -> Result<String, ApplicationError> {
        self.0
            .lock()
            .unwrap()
            .configurations
            .get(&sandbox.0)
            .map(|v| v.0.clone())
            .ok_or(ApplicationError::Busy)
    }
    fn put_bound_launch_config(
        &mut self,
        sandbox: core::SandboxId,
        token: &str,
        options: &CreateOptions,
    ) -> Result<(), ApplicationError> {
        let mut state = self.0.lock().unwrap();
        if state.configurations.contains_key(&sandbox.0) {
            return Err(ApplicationError::Conflict);
        }
        state
            .configurations
            .insert(sandbox.0, (token.into(), options.clone()));
        Ok(())
    }
    fn has_bound_launch_config(
        &self,
        sandbox: core::SandboxId,
        options: &CreateOptions,
    ) -> Result<bool, ApplicationError> {
        let state = self.0.lock().unwrap();
        match state.configurations.get(&sandbox.0) {
            None => Ok(false),
            Some((_, original)) if original == options => Ok(true),
            _ => Err(ApplicationError::Conflict),
        }
    }
}
impl TemplateArtifacts for TestHost {
    fn validate_template(&self, template: &TemplateBinding) -> Result<(), ApplicationError> {
        if template.is_valid() {
            Ok(())
        } else {
            Err(ApplicationError::Invalid)
        }
    }
}
pub fn template() -> InternalTemplate {
    InternalTemplate {
        revision: "template-revision".try_into().unwrap(),
        digest: [1; 32],
        local: TemplateBinding {
            template_id: "base-fixed".into(),
            revision: "r1".into(),
            owner: None,
            kernel: TemplateArtifact {
                path: "/protected-cache/kernel".into(),
                sha256: "a".repeat(64),
                size_bytes: 40,
            },
            rootfs: TemplateArtifact {
                path: "/protected-cache/rootfs".into(),
                sha256: "b".repeat(64),
                size_bytes: 4096,
            },
            resources: ResourceShape {
                cpu_count: 1,
                memory_mib: 128,
                host_memory_bytes: 129 * 1024 * 1024,
                disk_bytes: 4096,
            },
            envd_version: "fixed-envd".into(),
            startup: None,
        },
    }
}
pub fn compute() -> ComputeLimits {
    ComputeLimits {
        max_vcpus_global: 2,
        max_vcpus_per_tenant: 2,
        max_memory_bytes_global: 512 * 1024 * 1024,
        max_memory_bytes_per_tenant: 512 * 1024 * 1024,
    }
}
pub fn request() -> LifecycleRequest {
    let mut r = wire::request(1, 1, Intent::Create);
    r.create.timeout_ms = 60_000;
    r.create.max_lifetime_ms = 300_000;
    r.create.resources.memory_bytes = template().local.resources.host_memory_bytes;
    r.create.options = serde_json::to_vec(&CreateOptions {
        networking: Some(sandproof_controller_service::CreateNetworking {
            allow_internet_access: Some(false),
            network: None,
        }),
        ..CreateOptions::default()
    })
    .unwrap();
    r
}
pub fn application(
    path: &Path,
    state: Arc<Mutex<State>>,
    clock: Arc<TestClock>,
    provision: bool,
) -> Arc<ControllerApplication<TestHost>> {
    application_with_inbox_config(
        path,
        state,
        clock,
        provision,
        InternalInboxConfig {
            node: wire::node("host-a"),
            max_sandboxes: 2,
            operations_per_sandbox: 3,
            max_options_bytes: 128,
        },
    )
}

/// Selected component suites supply their explicit operation/options budget;
/// the original application and all local admission/recovery logic stay shared.
pub fn application_with_inbox_config(
    path: &Path,
    state: Arc<Mutex<State>>,
    clock: Arc<TestClock>,
    provision: bool,
    inbox: InternalInboxConfig,
) -> Arc<ControllerApplication<TestHost>> {
    let reserve = workflow::CLEANUP_BYTES + 1000;
    let mut journal = sandproof_controller_service::ControllerJournal::open_complete(
        path,
        workflow::NAMESPACE,
        complete_fixture::host(path, workflow::NAMESPACE, 64, 2 * 1024 * 1024, reserve),
        complete_fixture::maintenance(),
        0,
    )
    .unwrap();
    if provision {
        journal
            .initialize_internal_inbox(inbox, &mut NeverCrash)
            .unwrap();
    }
    let mut limits = workflow::admission_limits();
    limits.cleanup_reserve_bytes = reserve;
    limits.max_disk_bytes_global = reserve + 2 * 1024 * 1024;
    Arc::new(
        ControllerApplication::new(
            journal,
            TestHost(state),
            ApplicationConfig {
                public_ipv4_enabled: false,
                limits,
                disk_bytes: 4096,
                file_policy_digest: core::PolicyDigest(1),
                cpu_count: 1,
                memory_mb: 128,
                disk_size_mb: 1,
                envd_version: "fixed-envd".into(),
                domain: "example.invalid".into(),
            },
        )
        .with_clock(clock),
    )
}
