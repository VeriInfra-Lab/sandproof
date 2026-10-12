use sandproof_controller_service::application::fault::{
    FaultContext, FaultKind, FaultRecord, FaultScope, FaultStage,
};
use sandproof_controller_service::application::{
    ApplicationConfig as ManagedConfig, ControllerApplication, ControllerHost, DataRelay,
    HostCapacity, HostSlots, LaunchConfiguration, TemplateArtifacts, WorkEvidence, WorkExecution,
    WorkRecovery,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use sandbox_controller_core as core;
use sandproof_api::{
    ApiCredential, BuildPort, ConnectRequest, ControllerPort, CreateRequest, ListOrder, ListQuery,
    PortError, router,
};
#[path = "support/managed.rs"]
mod managed;
use managed::ManagedPort;
use sandproof_controller_service::{
    AdmissionLimits, ControllerJournal, CreateAdmission, HostEvidence, NeverCrash,
    WorkerTransportError, finish_admitted_create_v1, kill_v1,
};
use sandproof_protocol::{
    FailureCode, ResourceEvidence, ResourceRef, UnknownReason, WorkAction, WorkOutcome, WorkReceipt,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tower::ServiceExt;

#[path = "managed_port/admission.rs"]
mod admission;
#[path = "managed_port/allocators.rs"]
mod allocators;
#[path = "managed_port/bounded_slots.rs"]
mod bounded_slots;
#[path = "managed_port/build_vm.rs"]
mod build_vm;
#[path = "managed_port/committed_world.rs"]
mod committed_world;
#[path = "managed_port/complete_fixture.rs"]
mod complete_fixture;
#[path = "managed_port/credential_isolation.rs"]
mod credential_isolation;
#[path = "managed_port/history_capacity.rs"]
mod history_capacity;
#[cfg(unix)]
#[path = "managed_port/publication.rs"]
mod publication;

const NAMESPACE: &str = "0123456789abcdef";
const KEY_ONE: &str = "tenant-one-test-key-00000000000000001";
const KEY_TWO: &str = "tenant-two-test-key-00000000000000002";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Evidence {
    resource: ResourceRef,
    command: core::Command,
}

impl ResourceEvidence for Evidence {
    fn resource(&self) -> &ResourceRef {
        &self.resource
    }
}

impl HostEvidence for Evidence {
    fn validates(&self, command: &core::Command) -> bool {
        &self.command == command
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PrepareObservation {
    #[default]
    Missing,
    Successful,
    Incomplete,
    Reclaimed,
    LiveUnready,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ActivationObservation {
    #[default]
    Normal,
    Incomplete,
    Unavailable,
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum RecoveredAction {
    OpenFence,
    Network,
    TerminalFence,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum StopObservation {
    #[default]
    Normal,
    LoseReceipt,
    ObservedAfterRestart,
    LostThenObserved,
    Incomplete,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum AbortReceipt {
    #[default]
    Normal,
    LostOnce,
}

struct ActionGate {
    entered: Barrier,
    release: Barrier,
}

impl ActionGate {
    fn new() -> Self {
        Self {
            entered: Barrier::new(2),
            release: Barrier::new(2),
        }
    }

    fn block(&self) {
        self.entered.wait();
        self.release.wait();
    }
}

struct FakeHost {
    candidate_checks: Option<Arc<AtomicUsize>>,
    template_validation_error: Option<PortError>,
    secrets: BTreeMap<u64, (String, BTreeMap<String, String>)>,
    templates: BTreeMap<u64, sandproof_controller_service::templates::TemplateBinding>,
    launch_error_once: Option<PortError>,
    token_error_once: AtomicBool,
    token_error_for: Option<core::SandboxId>,
    relay_calls: Option<Arc<AtomicUsize>>,
    free_bytes: u64,
    public_ipv4_ready: bool,
    incomplete_open_fence: bool,
    prepare: PrepareObservation,
    reclaim_prepare_error_for: Option<core::SandboxId>,
    activation: ActivationObservation,
    activation_calls: Option<Arc<AtomicUsize>>,
    recovered_actions: BTreeSet<RecoveredAction>,
    stop: StopObservation,
    reclaim_stop_error_once: Option<PortError>,
    reclaim_stop_error_for: Option<core::SandboxId>,
    bad_stop_observation_for: Option<core::SandboxId>,
    bad_execute_for: Option<core::SandboxId>,
    unknown_stop_for: Option<core::SandboxId>,
    incomplete_cleanup: bool,
    unknown_cleanup_for: Option<core::SandboxId>,
    stop_calls: Option<Arc<AtomicUsize>>,
    prepare_calls: Option<Arc<AtomicUsize>>,
    abort_calls: Option<Arc<AtomicUsize>>,
    abort_receipt: AbortReceipt,
    stop_gate: Option<Arc<ActionGate>>,
    history_stop_gate: Option<(core::SandboxId, Arc<history_capacity::HostGate>)>,
    prepare_gate: Option<Arc<ActionGate>>,
}

impl Default for FakeHost {
    fn default() -> Self {
        Self {
            candidate_checks: None,
            template_validation_error: None,
            secrets: BTreeMap::new(),
            templates: BTreeMap::new(),
            launch_error_once: None,
            token_error_once: AtomicBool::new(false),
            token_error_for: None,
            relay_calls: None,
            free_bytes: 16 * 1024 * 1024 * 1024,
            public_ipv4_ready: false,
            incomplete_open_fence: false,
            prepare: PrepareObservation::Missing,
            reclaim_prepare_error_for: None,
            activation: ActivationObservation::Normal,
            activation_calls: None,
            recovered_actions: BTreeSet::new(),
            stop: StopObservation::Normal,
            reclaim_stop_error_once: None,
            reclaim_stop_error_for: None,
            bad_stop_observation_for: None,
            bad_execute_for: None,
            unknown_stop_for: None,
            incomplete_cleanup: false,
            unknown_cleanup_for: None,
            stop_calls: None,
            prepare_calls: None,
            abort_calls: None,
            abort_receipt: AbortReceipt::Normal,
            stop_gate: None,
            history_stop_gate: None,
            prepare_gate: None,
        }
    }
}

impl FakeHost {
    fn injected_execute_receipt(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> Option<WorkReceipt<Evidence>> {
        let sandbox = action.command.command().sandbox();
        if self.unknown_stop_for == Some(sandbox)
            && matches!(action.command.command(), core::Command::StopVm { .. })
        {
            return Some(WorkReceipt {
                identity: action.identity.clone(),
                outcome: WorkOutcome::Unknown {
                    reason: UnknownReason::ObservationUnavailable,
                    last_observation: None,
                },
            });
        }
        if self.bad_execute_for == Some(sandbox) {
            let mut identity = action.identity.clone();
            identity.owner = "forged-owner".try_into().unwrap();
            return Some(WorkReceipt {
                identity,
                outcome: WorkOutcome::ObservedSuccess(Evidence {
                    resource: action.identity.resource.clone(),
                    command: action.command.command().clone(),
                }),
            });
        }
        None
    }
}

fn unknown_receipt(action: &WorkAction<core::EffectId>) -> WorkReceipt<Evidence> {
    WorkReceipt {
        identity: action.identity.clone(),
        outcome: WorkOutcome::Unknown {
            reason: UnknownReason::ObservationUnavailable,
            last_observation: None,
        },
    }
}

impl WorkEvidence for FakeHost {
    type Evidence = Evidence;
}
impl WorkExecution for FakeHost {
    fn execute(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<Evidence>, WorkerTransportError> {
        if let Some(receipt) = self.injected_execute_receipt(action) {
            return Ok(receipt);
        }
        if self.incomplete_open_fence
            && matches!(
                action.command.command(),
                core::Command::InstallFence {
                    terminal: false,
                    ..
                }
            )
        {
            return Ok(unknown_receipt(action));
        }
        if matches!(action.command.command(), core::Command::PrepareVm { .. }) {
            if let Some(calls) = &self.prepare_calls {
                calls.fetch_add(1, Ordering::SeqCst);
            }
            if let Some(gate) = &self.prepare_gate {
                gate.block();
            }
            if self.prepare == PrepareObservation::LiveUnready {
                return Ok(unknown_receipt(action));
            }
        }
        if matches!(action.command.command(), core::Command::ActivateVm { .. })
            && matches!(
                self.activation,
                ActivationObservation::Incomplete | ActivationObservation::Unavailable
            )
        {
            return Ok(unknown_receipt(action));
        }
        if matches!(action.command.command(), core::Command::AbortPrepare { .. }) {
            if let Some(calls) = &self.abort_calls {
                calls.fetch_add(1, Ordering::SeqCst);
            }
            assert!(matches!(
                self.prepare,
                PrepareObservation::LiveUnready | PrepareObservation::Reclaimed
            ));
            self.prepare = PrepareObservation::Reclaimed;
            if std::mem::replace(&mut self.abort_receipt, AbortReceipt::Normal)
                == AbortReceipt::LostOnce
            {
                return Ok(unknown_receipt(action));
            }
        }
        if matches!(action.command.command(), core::Command::CleanupVm { sandbox, .. } if self.unknown_cleanup_for == Some(*sandbox))
        {
            return Ok(unknown_receipt(action));
        }
        if matches!(action.command.command(), core::Command::StopVm { .. }) {
            if let Some(calls) = &self.stop_calls {
                calls.fetch_add(1, Ordering::SeqCst);
            }
            if let Some(gate) = &self.stop_gate {
                gate.block();
            }
            if let Some((sandbox, gate)) = &self.history_stop_gate
                && *sandbox == action.command.command().sandbox()
            {
                gate.block();
            }
        }
        if matches!(
            self.stop,
            StopObservation::LoseReceipt
                | StopObservation::LostThenObserved
                | StopObservation::Incomplete
        ) && matches!(action.command.command(), core::Command::StopVm { .. })
        {
            return Ok(unknown_receipt(action));
        }
        Ok(WorkReceipt {
            identity: action.identity.clone(),
            outcome: WorkOutcome::ObservedSuccess(Evidence {
                resource: action.identity.resource.clone(),
                command: action.command.command().clone(),
            }),
        })
    }
}

impl WorkRecovery for FakeHost {
    fn observe(&self, action: &WorkAction<core::EffectId>) -> Option<WorkReceipt<Evidence>> {
        if self.bad_stop_observation_for == Some(action.command.command().sandbox())
            && matches!(action.command.command(), core::Command::StopVm { .. })
        {
            let mut identity = action.identity.clone();
            identity.owner = "forged-owner".try_into().unwrap();
            return Some(WorkReceipt {
                identity,
                outcome: WorkOutcome::ObservedSuccess(Evidence {
                    resource: action.identity.resource.clone(),
                    command: action.command.command().clone(),
                }),
            });
        }
        if self.prepare == PrepareObservation::Reclaimed
            && matches!(action.command.command(), core::Command::PrepareVm { .. })
        {
            return Some(WorkReceipt {
                identity: action.identity.clone(),
                outcome: WorkOutcome::DefinitiveFailure {
                    code: FailureCode::InvalidResource,
                    retryable: false,
                    partial: None,
                },
            });
        }
        if (self.recovered_actions.contains(&RecoveredAction::OpenFence)
            && matches!(
                action.command.command(),
                core::Command::InstallFence {
                    terminal: false,
                    ..
                }
            ))
            || (self.prepare == PrepareObservation::Successful
                && matches!(action.command.command(), core::Command::PrepareVm { .. }))
            || (self.activation == ActivationObservation::Complete
                && matches!(action.command.command(), core::Command::ActivateVm { .. }))
            || (self.recovered_actions.contains(&RecoveredAction::Network)
                && matches!(
                    action.command.command(),
                    core::Command::ConfigureNetwork { .. }
                ))
            || (self
                .recovered_actions
                .contains(&RecoveredAction::TerminalFence)
                && matches!(
                    action.command.command(),
                    core::Command::InstallFence { terminal: true, .. }
                ))
            || (matches!(
                self.stop,
                StopObservation::ObservedAfterRestart | StopObservation::LostThenObserved
            ) && matches!(action.command.command(), core::Command::StopVm { .. }))
            || (self.incomplete_cleanup
                && matches!(action.command.command(), core::Command::CleanupVm { .. }))
            || (self.prepare == PrepareObservation::Reclaimed
                && matches!(action.command.command(), core::Command::AbortPrepare { .. }))
        {
            Some(WorkReceipt {
                identity: action.identity.clone(),
                outcome: WorkOutcome::ObservedSuccess(Evidence {
                    resource: action.identity.resource.clone(),
                    command: action.command.command().clone(),
                }),
            })
        } else {
            None
        }
    }

    fn reclaim_incomplete_prepare(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        if self.reclaim_prepare_error_for == Some(action.command.command().sandbox()) {
            return Err(PortError::Busy);
        }
        if self.prepare == PrepareObservation::Incomplete {
            assert!(matches!(
                action.command.command(),
                core::Command::PrepareVm { .. }
            ));
            self.prepare = PrepareObservation::Reclaimed;
        }
        Ok(())
    }

    fn reclaim_incomplete_network(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        assert!(matches!(
            action.command.command(),
            core::Command::ConfigureNetwork { .. }
        ));
        self.recovered_actions.insert(RecoveredAction::Network);
        Ok(())
    }

    fn reclaim_incomplete_activation(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        assert!(matches!(
            action.command.command(),
            core::Command::ActivateVm { .. }
        ));
        if self.activation == ActivationObservation::Unavailable {
            return Err(PortError::Busy);
        }
        assert_eq!(self.activation, ActivationObservation::Incomplete);
        if let Some(calls) = &self.activation_calls {
            calls.fetch_add(1, Ordering::SeqCst);
        }
        self.activation = ActivationObservation::Complete;
        Ok(())
    }

    fn reclaim_incomplete_terminal_fence(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        assert!(matches!(
            action.command.command(),
            core::Command::InstallFence { terminal: true, .. }
        ));
        self.recovered_actions
            .insert(RecoveredAction::TerminalFence);
        Ok(())
    }

    fn reclaim_incomplete_open_fence(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        assert!(matches!(
            action.command.command(),
            core::Command::InstallFence {
                terminal: false,
                ..
            }
        ));
        self.recovered_actions.insert(RecoveredAction::OpenFence);
        Ok(())
    }

    fn reclaim_incomplete_stop(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        assert!(matches!(
            action.command.command(),
            core::Command::StopVm { .. }
        ));
        if self.reclaim_stop_error_for == Some(action.command.command().sandbox()) {
            self.reclaim_stop_error_for = None;
            return Err(PortError::Busy);
        }
        if let Some(error) = self.reclaim_stop_error_once.take() {
            return Err(error);
        }
        if self.stop == StopObservation::Incomplete {
            self.stop = StopObservation::ObservedAfterRestart;
        }
        Ok(())
    }

    fn reclaim_incomplete_abort_prepare(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        assert!(matches!(
            action.command.command(),
            core::Command::AbortPrepare { .. }
        ));
        assert_eq!(self.prepare, PrepareObservation::Reclaimed);
        Ok(())
    }

    fn reclaim_incomplete_cleanup(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        if matches!(action.command.command(), core::Command::CleanupVm { .. }) {
            self.incomplete_cleanup = true;
        }
        Ok(())
    }
}

impl HostSlots for FakeHost {
    fn bind_host_slot(
        &mut self,
        _: &sandproof_controller_service::HostSlotContext,
        _: bool,
    ) -> Result<(), PortError> {
        Ok(())
    }

    fn release_host_slot(
        &mut self,
        _: &sandproof_controller_service::HostSlotContext,
    ) -> Result<(), PortError> {
        Ok(())
    }
}

impl DataRelay for FakeHost {
    fn acquire_data_gate(
        &self,
        _: core::SandboxId,
        _: core::VmId,
        _: core::Generation,
    ) -> Result<Option<std::fs::File>, PortError> {
        let file = tempfile::tempfile().map_err(|_| PortError::Busy)?;
        file.try_lock_shared().map_err(|_| PortError::Busy)?;
        Ok(Some(file))
    }

    fn spawn_data_relay(
        &mut self,
        _: core::SandboxId,
        _: core::VmId,
        _: core::Generation,
        _: bool,
    ) -> Result<std::process::Child, PortError> {
        if let Some(calls) = &self.relay_calls {
            calls.fetch_add(1, Ordering::SeqCst);
        }
        Err(PortError::Busy)
    }

    fn data_gate_drained(
        &self,
        _: &sandproof_controller_service::HostSlotContext,
    ) -> Result<bool, PortError> {
        Ok(true)
    }
}

impl HostCapacity for FakeHost {
    fn available_disk_bytes(&self) -> Result<u64, PortError> {
        Ok(self.free_bytes)
    }

    fn validate_physical_storage(
        &self,
        profile: &sandproof_controller_service::physical_storage::PhysicalStorageProfile,
        pool: &sandproof_controller_service::physical_storage::PhysicalPool,
    ) -> Result<(), PortError> {
        profile.validate().map_err(|_| PortError::Busy)?;
        if profile.pools.contains(pool) {
            Ok(())
        } else {
            Err(PortError::Busy)
        }
    }

    fn public_ipv4_ready(&self) -> Result<bool, PortError> {
        Ok(self.public_ipv4_ready)
    }
}

impl LaunchConfiguration for FakeHost {
    fn put_secret(&mut self, sandbox: core::SandboxId, token: &str) -> Result<(), PortError> {
        self.put_launch_config(sandbox, token, &BTreeMap::new())
    }

    fn has_secret(&self, sandbox: core::SandboxId) -> Result<bool, PortError> {
        Ok(self.secrets.contains_key(&sandbox.0))
    }

    fn token_for(&self, sandbox: core::SandboxId) -> Result<String, PortError> {
        if self.token_error_for == Some(sandbox)
            || self.token_error_once.swap(false, Ordering::SeqCst)
        {
            return Err(PortError::Busy);
        }
        self.secrets
            .get(&sandbox.0)
            .map(|(token, _)| token.clone())
            .ok_or(PortError::Busy)
    }

    fn put_launch_config(
        &mut self,
        sandbox: core::SandboxId,
        token: &str,
        env_vars: &BTreeMap<String, String>,
    ) -> Result<(), PortError> {
        if let Some(error) = self.launch_error_once.take() {
            return Err(error);
        }
        if self
            .secrets
            .insert(sandbox.0, (token.to_owned(), env_vars.clone()))
            .is_some()
        {
            return Err(PortError::Conflict);
        }
        Ok(())
    }

    fn has_launch_config(
        &self,
        sandbox: core::SandboxId,
        env_vars: &BTreeMap<String, String>,
    ) -> Result<bool, PortError> {
        match self.secrets.get(&sandbox.0) {
            Some((_, stored)) if stored == env_vars => Ok(true),
            Some(_) => Err(PortError::Busy),
            None => Ok(false),
        }
    }

    fn put_bound_launch_config(
        &mut self,
        sandbox: core::SandboxId,
        token: &str,
        options: &sandproof_controller_service::CreateOptions,
    ) -> Result<(), PortError> {
        self.put_launch_config(sandbox, token, &options.env_vars)?;
        if let Some(template) = &options.template {
            self.templates.insert(sandbox.0, template.clone());
        }
        Ok(())
    }

    fn has_bound_launch_config(
        &self,
        sandbox: core::SandboxId,
        options: &sandproof_controller_service::CreateOptions,
    ) -> Result<bool, PortError> {
        if self.templates.get(&sandbox.0) != options.template.as_ref() {
            return Err(PortError::Busy);
        }
        self.has_launch_config(sandbox, &options.env_vars)
    }
}

impl TemplateArtifacts for FakeHost {
    fn validate_template(
        &self,
        template: &sandproof_controller_service::templates::TemplateBinding,
    ) -> Result<(), PortError> {
        if let Some(error) = &self.template_validation_error {
            return Err(*error);
        }
        if template.is_valid() {
            Ok(())
        } else {
            Err(PortError::Invalid)
        }
    }

    fn cache_candidate_verification(
        &self,
        template: &sandproof_controller_service::templates::TemplateBinding,
        verification: &sandproof_controller_service::template_registry::CandidateVerification,
    ) {
        assert!(verification.matches(&template.kernel));
        assert!(verification.matches(&template.rootfs));
        if let Some(count) = &self.candidate_checks {
            count.fetch_add(1, Ordering::SeqCst);
        }
    }
}

fn config() -> ManagedConfig {
    ManagedConfig {
        public_ipv4_enabled: false,
        limits: AdmissionLimits {
            max_active_global: 2,
            max_active_per_tenant: 1,
            max_disk_bytes_global: complete_fixture::CLEANUP + 3_000,
            max_disk_bytes_per_tenant: 1_000,
            cleanup_reserve_bytes: complete_fixture::CLEANUP,
            max_timeout_secs: 86_400,
            max_lifetime_secs: 86_400,
            startup_timeout_secs: 300,
        },
        disk_bytes: 1_000,
        file_policy_digest: core::PolicyDigest(1),
        cpu_count: 1,
        memory_mb: 512,
        disk_size_mb: 1_024,
        envd_version: "0.9.0".into(),
        domain: "example.invalid".into(),
    }
}

fn new_port<H: ControllerHost>(
    journal: ControllerJournal,
    host: H,
    config: ManagedConfig,
) -> ManagedPort<H> {
    ManagedPort::new(Arc::new(ControllerApplication::new(journal, host, config)))
}
fn with_templates<H: ControllerHost>(
    journal: ControllerJournal,
    host: H,
    config: ManagedConfig,
    catalog: sandproof_controller_service::templates::TemplateCatalog,
) -> Result<ManagedPort<H>, PortError> {
    ControllerApplication::with_templates(journal, host, config, catalog)
        .map(|app| ManagedPort::new(Arc::new(app)))
}

fn setup_with_host(host: FakeHost) -> (TempDir, Arc<ManagedPort<FakeHost>>) {
    let directory = TempDir::new().unwrap();
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    (directory, Arc::new(new_port(journal, host, config())))
}

fn setup() -> (TempDir, Arc<ManagedPort<FakeHost>>) {
    setup_with_host(FakeHost::default())
}

fn admit_public_create(journal: &mut ControllerJournal) {
    journal
        .admit_create(
            core::CreateRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                owner: core::TenantId(1),
                sandbox: core::SandboxId(1),
                workspace: core::FsId(1),
                file_policy: core::FilePolicyRef {
                    digest: core::PolicyDigest(1),
                },
                policy: core::NetworkPolicy::from_sdk_ipv4(None, &[], &[]).unwrap(),
                operation: core::OperationId(1),
            },
            i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
            .unwrap(),
            300,
            1_000,
            config().limits,
            &mut NeverCrash,
        )
        .unwrap();
}

// Manual worker-fault fixtures must honor the same launch ordering as the
// production API: admit, register the original credential digest, then claim work.
fn register_test_credential(
    journal: &mut ControllerJournal,
    sandbox: core::SandboxId,
    token: &str,
) {
    let generation = journal
        .world()
        .unwrap()
        .sandbox_record(sandbox)
        .unwrap()
        .generation;
    journal
        .register_data_credential(
            sandbox,
            generation,
            Sha256::digest(token.as_bytes()).into(),
            &mut NeverCrash,
        )
        .unwrap();
}

fn due_unknown_prepare_fixture(
    lose_abort_receipt_once: bool,
) -> (
    TempDir,
    ControllerJournal,
    FakeHost,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
) {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let sandbox = core::SandboxId(1);
    let mut limits = config().limits;
    limits.startup_timeout_secs = 1;
    journal
        .admit_create(
            core::CreateRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                owner: core::TenantId(1),
                sandbox,
                workspace: core::FsId(1),
                file_policy: core::FilePolicyRef {
                    digest: core::PolicyDigest(1),
                },
                policy: core::NetworkPolicy::DenyAll,
                operation: core::OperationId(1),
            },
            0,
            1,
            1_000,
            limits,
            &mut NeverCrash,
        )
        .unwrap();
    let prepare_calls = Arc::new(AtomicUsize::new(0));
    let abort_calls = Arc::new(AtomicUsize::new(0));
    let mut host = FakeHost {
        prepare: PrepareObservation::LiveUnready,
        prepare_calls: Some(Arc::clone(&prepare_calls)),
        abort_calls: Some(Arc::clone(&abort_calls)),
        abort_receipt: if lose_abort_receipt_once {
            AbortReceipt::LostOnce
        } else {
            AbortReceipt::Normal
        },
        ..FakeHost::default()
    };
    host.put_secret(sandbox, &"a".repeat(64)).unwrap();
    register_test_credential(&mut journal, sandbox, &"a".repeat(64));
    assert!(matches!(
        finish_admitted_create_v1(&mut journal, sandbox, core::OperationId(1), &mut |action| {
            host.execute(action)
        }),
        Err(sandproof_controller_service::JournalError::UnresolvedAction)
    ));
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);
    assert_eq!(journal.uncertain_actions().unwrap().len(), 1);
    (directory, journal, host, prepare_calls, abort_calls)
}

#[test]
fn expired_unready_prepare_is_fenced_exactly_aborted_and_never_relaunched() {
    let (directory, journal, host, prepare_calls, abort_calls) = due_unknown_prepare_fixture(false);
    drop(journal);
    let port = new_port(
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap(),
        host,
        config(),
    );
    let first = port.application().recover_on_startup().unwrap();
    assert_eq!(first.workflows_completed, 1);
    assert_eq!(first.unresolved_sandboxes, 1);
    assert_eq!(abort_calls.load(Ordering::SeqCst), 1);
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        port.get("1", &format!("sb-{NAMESPACE}-1")),
        Err(PortError::NotFound)
    );
    let second = port.application().recover_on_startup().unwrap();
    assert_eq!(second.receipts_recovered, 1);
    assert_eq!(second.unresolved_sandboxes, 0);
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);
    assert_eq!(abort_calls.load(Ordering::SeqCst), 1);
    drop(port);
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert!(journal.uncertain_actions().unwrap().is_empty());
    let world = journal.world().unwrap();
    let record = world.sandbox_record(core::SandboxId(1)).unwrap();
    assert_eq!(record.phase, core::Phase::Destroyed);
    assert!(record.terminal);
    assert_eq!(
        world.vm_record(core::VmId(0)).unwrap().status,
        core::VmStatus::Reclaimed
    );
    assert!(world.slot_record(core::NetSlotId(0)).unwrap().reclaimed);
}

#[test]
fn abort_intent_alone_cannot_schedule_unfenced_or_foreign_resource() {
    let (_directory, mut journal, _host, _prepare_calls, _abort_calls) =
        due_unknown_prepare_fixture(false);
    assert_eq!(journal.abort_due_startups(999, &mut NeverCrash).unwrap(), 0);
    assert_eq!(
        journal.abort_due_startups(1_000, &mut NeverCrash).unwrap(),
        1
    );
    assert_eq!(
        journal.abort_due_startups(1_001, &mut NeverCrash).unwrap(),
        0
    );
    let world = journal.world().unwrap();
    let sandbox = core::SandboxId(1);
    assert!(core::scheduler::only_uncertain_prepare_for_abort(
        &world, sandbox
    ));
    assert_eq!(
        core::scheduler::handle_schedule_abort_prepare(
            &world,
            sandbox,
            1,
            core::VmId(0),
            core::NetSlotId(99),
        ),
        Err(core::scheduler::SchedulerReject::PlanMismatch)
    );
    assert_eq!(
        core::scheduler::handle_schedule_abort_prepare(
            &world,
            sandbox,
            1,
            core::VmId(0),
            core::NetSlotId(0),
        ),
        Err(core::scheduler::SchedulerReject::FenceNotInstalled)
    );
    assert!(
        core::publication::handle_publish_failure(
            &world,
            sandbox,
            core::FailureKind::DeadlineExceeded,
        )
        .is_err()
    );
}

#[test]
fn restart_after_abort_host_success_without_receipt_reconciles_once() {
    let (directory, journal, host, prepare_calls, abort_calls) = due_unknown_prepare_fixture(true);
    drop(journal);
    let port = new_port(
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap(),
        host,
        config(),
    );
    let first = port.application().recover_on_startup().unwrap();
    assert_eq!(first.unresolved_sandboxes, 1);
    assert_eq!(abort_calls.load(Ordering::SeqCst), 1);
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        port.get("1", &format!("sb-{NAMESPACE}-1")),
        Err(PortError::NotFound)
    );
    drop(port);
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let abort_action = journal
        .uncertain_actions()
        .unwrap()
        .into_iter()
        .find(|action| matches!(action.command.command(), core::Command::AbortPrepare { .. }))
        .unwrap();
    let world = journal.world().unwrap();
    assert!(
        journal
            .matches_current_abort_prepare(&world, &abort_action)
            .unwrap()
    );
    let mut stale = abort_action.clone();
    stale.identity.generation += 1;
    assert!(
        !journal
            .matches_current_abort_prepare(&world, &stale)
            .unwrap()
    );
    drop(journal);
    // A new controller process observes physical reclamation, records both
    // missing receipts, and publishes failure without repeating either action.
    let mut restarted_host = FakeHost {
        prepare: PrepareObservation::Reclaimed,
        prepare_calls: Some(Arc::clone(&prepare_calls)),
        abort_calls: Some(Arc::clone(&abort_calls)),
        ..FakeHost::default()
    };
    restarted_host
        .put_secret(core::SandboxId(1), &"a".repeat(64))
        .unwrap();
    let restarted = new_port(
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap(),
        restarted_host,
        config(),
    );
    let report = restarted.application().recover_on_startup().unwrap();
    assert_eq!(report.receipts_recovered, 2);
    assert_eq!(report.workflows_completed, 1);
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);
    assert_eq!(abort_calls.load(Ordering::SeqCst), 1);
    drop(restarted);
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert!(journal.uncertain_actions().unwrap().is_empty());
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(core::SandboxId(1))
            .unwrap()
            .phase,
        core::Phase::Destroyed
    );
}

#[test]
fn opening_fence_interrupted_before_host_write_is_completed_only_for_current_create() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    admit_public_create(&mut journal);
    let sandbox = core::SandboxId(1);
    register_test_credential(&mut journal, sandbox, &"a".repeat(64));
    let pending =
        finish_admitted_create_v1(&mut journal, sandbox, core::OperationId(1), &mut |action| {
            FakeHost {
                incomplete_open_fence: true,
                ..FakeHost::default()
            }
            .execute(action)
        });
    assert!(matches!(
        pending,
        Err(sandproof_controller_service::JournalError::UnresolvedAction)
    ));
    let uncertain = journal.uncertain_actions().unwrap();
    assert_eq!(uncertain.len(), 1);
    assert!(matches!(
        uncertain[0].command.command(),
        core::Command::InstallFence {
            terminal: false,
            ..
        }
    ));
    let world = journal.world().unwrap();
    assert!(
        journal
            .matches_current_open_fence(&world, &uncertain[0])
            .unwrap()
    );
    let mut stale = uncertain[0].clone();
    stale.identity.epoch += 1;
    assert!(!journal.matches_current_open_fence(&world, &stale).unwrap());
    assert_ne!(
        world.sandbox_record(sandbox).unwrap().phase,
        core::Phase::Running
    );
    drop(journal);

    let mut host = FakeHost {
        public_ipv4_ready: true,
        ..FakeHost::default()
    };
    host.put_secret(sandbox, &"a".repeat(64)).unwrap();
    let mut settings = config();
    settings.public_ipv4_enabled = true;
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        host,
        settings,
    );
    let report = port.application().recover_on_startup().unwrap();
    assert_eq!(report.receipts_recovered, 1);
    assert_eq!(report.workflows_completed, 1);
    assert_eq!(report.unresolved_sandboxes, 0);
    assert_eq!(
        port.get("1", &format!("sb-{NAMESPACE}-1"))
            .unwrap()
            .info
            .state,
        "running"
    );
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .receipts_recovered,
        0
    );
}

#[test]
fn lost_public_network_receipt_is_reconciled_before_running() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    admit_public_create(&mut journal);
    let sandbox = core::SandboxId(1);
    register_test_credential(&mut journal, sandbox, &"a".repeat(64));
    let pending =
        finish_admitted_create_v1(&mut journal, sandbox, core::OperationId(1), &mut |action| {
            if matches!(
                action.command.command(),
                core::Command::ConfigureNetwork { .. }
            ) {
                Ok(WorkReceipt::<Evidence> {
                    identity: action.identity.clone(),
                    outcome: WorkOutcome::Unknown {
                        reason: UnknownReason::ObservationUnavailable,
                        last_observation: None,
                    },
                })
            } else {
                FakeHost::default().execute(action)
            }
        });
    assert!(matches!(
        pending,
        Err(sandproof_controller_service::JournalError::UnresolvedAction)
    ));
    let uncertain = journal.uncertain_actions().unwrap();
    assert_eq!(uncertain.len(), 1);
    assert!(matches!(
        uncertain[0].command.command(),
        core::Command::ConfigureNetwork { .. }
    ));
    assert!(
        journal
            .matches_current_network(&journal.world().unwrap(), &uncertain[0])
            .unwrap()
    );
    let mut stale = uncertain[0].clone();
    stale.identity.epoch += 1;
    assert!(
        !journal
            .matches_current_network(&journal.world().unwrap(), &stale)
            .unwrap()
    );
    assert_ne!(
        journal
            .world()
            .unwrap()
            .sandbox_record(sandbox)
            .unwrap()
            .phase,
        core::Phase::Running
    );
    drop(journal);

    let mut host = FakeHost {
        public_ipv4_ready: true,
        ..FakeHost::default()
    };
    host.put_secret(sandbox, &"a".repeat(64)).unwrap();
    let mut settings = config();
    settings.public_ipv4_enabled = true;
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        host,
        settings,
    );
    let report = port.application().recover_on_startup().unwrap();
    assert_eq!(report.receipts_recovered, 1);
    assert_eq!(report.workflows_completed, 1);
    assert_eq!(report.unresolved_sandboxes, 0);
    assert_eq!(
        port.get("1", &format!("sb-{NAMESPACE}-1"))
            .unwrap()
            .info
            .state,
        "running"
    );
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .receipts_recovered,
        0
    );
}

fn assert_activation_identity(journal: &ControllerJournal, action: &WorkAction<core::EffectId>) {
    let world = journal.world().unwrap();
    assert!(journal.matches_current_activation(&world, action).unwrap());
    let mut wrong_epoch = action.clone();
    wrong_epoch.identity.epoch += 1;
    assert!(
        !journal
            .matches_current_activation(&world, &wrong_epoch)
            .unwrap()
    );
    let mut wrong_vm = action.clone();
    if let core::Command::ActivateVm { vm, .. } = &mut wrong_vm.command.logical.command {
        *vm = core::VmId(vm.0 + 1);
    }
    assert!(
        !journal
            .matches_current_activation(&world, &wrong_vm)
            .unwrap()
    );
}

#[test]
fn unknown_activation_reuses_original_attempt_and_vm_then_publishes_running() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let sandbox = core::SandboxId(1);
    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    admit_public_create(&mut journal);
    let calls = Arc::new(AtomicUsize::new(0));
    let mut host = FakeHost {
        activation: ActivationObservation::Incomplete,
        activation_calls: Some(Arc::clone(&calls)),
        public_ipv4_ready: true,
        ..FakeHost::default()
    };
    host.put_secret(sandbox, &"a".repeat(64)).unwrap();
    register_test_credential(&mut journal, sandbox, &"a".repeat(64));
    let pending =
        finish_admitted_create_v1(&mut journal, sandbox, core::OperationId(1), &mut |action| {
            host.execute(action)
        });
    assert!(matches!(
        pending,
        Err(sandproof_controller_service::JournalError::UnresolvedAction)
    ));
    let uncertain = journal.uncertain_actions().unwrap();
    assert_eq!(uncertain.len(), 1);
    let action = &uncertain[0];
    assert!(matches!(
        action.command.command(),
        core::Command::ActivateVm { .. }
    ));
    assert_activation_identity(&journal, action);
    let world = journal.world().unwrap();
    let original_vm = world.sandbox_record(sandbox).unwrap().current_vm;
    drop(journal);

    let mut wrong_host = FakeHost {
        activation: ActivationObservation::Incomplete,
        activation_calls: Some(Arc::clone(&calls)),
        ..FakeHost::default()
    };
    wrong_host
        .put_launch_config(
            sandbox,
            &"b".repeat(64),
            &BTreeMap::from([("SP_RECOVERY".into(), "different".into())]),
        )
        .unwrap();
    let blocked = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        wrong_host,
        config(),
    );
    assert_eq!(
        blocked
            .application()
            .recover_on_startup()
            .unwrap()
            .unresolved_sandboxes,
        1
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(blocked);

    let mut settings = config();
    settings.public_ipv4_enabled = true;
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        host,
        settings,
    );
    let report = port.application().recover_on_startup().unwrap();
    assert_eq!(report.receipts_recovered, 1);
    assert_eq!(report.workflows_completed, 1);
    assert_eq!(report.unresolved_sandboxes, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        port.get("1", &format!("sb-{NAMESPACE}-1"))
            .unwrap()
            .info
            .state,
        "running"
    );
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .receipts_recovered,
        0
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(port);
    let journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let world = journal.world().unwrap();
    assert_eq!(
        world.sandbox_record(sandbox).unwrap().current_vm,
        original_vm
    );
    assert!(journal.uncertain_actions().unwrap().is_empty());
}

#[test]
fn expired_unknown_activation_is_stopped_without_reinitializing_or_publishing_running() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let sandbox = core::SandboxId(1);
    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let mut limits = config().limits;
    limits.startup_timeout_secs = 1;
    journal
        .admit_create(
            core::CreateRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                owner: core::TenantId(1),
                sandbox,
                workspace: core::FsId(1),
                file_policy: core::FilePolicyRef {
                    digest: core::PolicyDigest(1),
                },
                policy: core::NetworkPolicy::DenyAll,
                operation: core::OperationId(1),
            },
            0,
            300,
            1_000,
            limits,
            &mut NeverCrash,
        )
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut host = FakeHost {
        activation: ActivationObservation::Incomplete,
        activation_calls: Some(Arc::clone(&calls)),
        ..FakeHost::default()
    };
    host.put_secret(sandbox, &"a".repeat(64)).unwrap();
    register_test_credential(&mut journal, sandbox, &"a".repeat(64));
    assert!(matches!(
        finish_admitted_create_v1(&mut journal, sandbox, core::OperationId(1), &mut |action| {
            host.execute(action)
        }),
        Err(sandproof_controller_service::JournalError::UnresolvedAction)
    ));
    drop(journal);
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        host,
        config(),
    );
    let report = port.application().recover_on_startup().unwrap();
    assert_eq!(report.receipts_recovered, 0);
    assert_eq!(report.workflows_completed, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        port.get("1", &format!("sb-{NAMESPACE}-1")),
        Err(PortError::NotFound)
    );
    drop(port);
    let journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let world = journal.world().unwrap();
    assert_eq!(
        world.sandbox_record(sandbox).unwrap().phase,
        core::Phase::Destroyed
    );
    assert_eq!(
        world.vm_record(core::VmId(0)).unwrap().status,
        core::VmStatus::Reclaimed
    );
}

#[test]
fn unavailable_activation_is_retired_after_exact_stop_after_startup_deadline() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let sandbox = core::SandboxId(1);
    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let mut limits = config().limits;
    limits.startup_timeout_secs = 1;
    journal
        .admit_create(
            core::CreateRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                owner: core::TenantId(1),
                sandbox,
                workspace: core::FsId(1),
                file_policy: core::FilePolicyRef {
                    digest: core::PolicyDigest(1),
                },
                policy: core::NetworkPolicy::DenyAll,
                operation: core::OperationId(1),
            },
            0,
            300,
            1_000,
            limits,
            &mut NeverCrash,
        )
        .unwrap();
    let mut host = FakeHost {
        activation: ActivationObservation::Unavailable,
        ..FakeHost::default()
    };
    host.put_secret(sandbox, &"a".repeat(64)).unwrap();
    register_test_credential(&mut journal, sandbox, &"a".repeat(64));
    assert!(matches!(
        finish_admitted_create_v1(&mut journal, sandbox, core::OperationId(1), &mut |action| {
            host.execute(action)
        }),
        Err(sandproof_controller_service::JournalError::UnresolvedAction)
    ));
    drop(journal);
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        host,
        config(),
    );
    let report = port.application().recover_on_startup().unwrap();
    assert_eq!(
        port.get("1", &format!("sb-{NAMESPACE}-1")),
        Err(PortError::NotFound)
    );
    assert_eq!(report.workflows_completed, 1);
    assert_eq!(report.unresolved_sandboxes, 0);
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .workflows_completed,
        0
    );
    drop(port);
    let journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    assert!(journal.uncertain_actions().unwrap().is_empty());
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(sandbox)
            .unwrap()
            .phase,
        core::Phase::Destroyed
    );
}

#[test]
fn lost_terminal_fence_receipt_is_reapplied_before_failed_publication() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    admit_public_create(&mut journal);
    let sandbox = core::SandboxId(1);
    register_test_credential(&mut journal, sandbox, &"a".repeat(64));
    let pending =
        finish_admitted_create_v1(&mut journal, sandbox, core::OperationId(1), &mut |action| {
            let outcome = match action.command.command() {
                core::Command::ConfigureNetwork { .. } => WorkOutcome::DefinitiveFailure {
                    code: FailureCode::InvalidResource,
                    retryable: false,
                    partial: None,
                },
                core::Command::InstallFence { terminal: true, .. } => WorkOutcome::Unknown {
                    reason: UnknownReason::ObservationUnavailable,
                    last_observation: None,
                },
                _ => return FakeHost::default().execute(action),
            };
            Ok(WorkReceipt::<Evidence> {
                identity: action.identity.clone(),
                outcome,
            })
        });
    assert!(matches!(
        pending,
        Err(sandproof_controller_service::JournalError::UnresolvedAction)
    ));
    let uncertain = journal.uncertain_actions().unwrap();
    assert_eq!(uncertain.len(), 1);
    assert!(matches!(
        uncertain[0].command.command(),
        core::Command::InstallFence { terminal: true, .. }
    ));
    assert!(
        journal
            .matches_current_terminal_fence(&journal.world().unwrap(), &uncertain[0])
            .unwrap()
    );
    let mut stale = uncertain[0].clone();
    stale.identity.generation += 1;
    assert!(
        !journal
            .matches_current_terminal_fence(&journal.world().unwrap(), &stale)
            .unwrap()
    );
    assert_ne!(
        journal
            .world()
            .unwrap()
            .sandbox_record(sandbox)
            .unwrap()
            .phase,
        core::Phase::Destroyed
    );
    drop(journal);

    let mut host = FakeHost::default();
    host.put_secret(sandbox, &"a".repeat(64)).unwrap();
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        host,
        config(),
    );
    let report = port.application().recover_on_startup().unwrap();
    assert_eq!(report.receipts_recovered, 1);
    assert_eq!(report.workflows_completed, 1);
    assert_eq!(report.unresolved_sandboxes, 0);
    assert_eq!(
        port.get("1", &format!("sb-{NAMESPACE}-1")),
        Err(PortError::NotFound)
    );
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .receipts_recovered,
        0
    );
}

#[test]
fn startup_recovers_admitted_create_before_credential_write() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let sandbox = core::SandboxId(1);
    journal
        .admit_create(
            core::CreateRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                owner: core::TenantId(1),
                sandbox,
                workspace: core::FsId(1),
                file_policy: core::FilePolicyRef {
                    digest: core::PolicyDigest(1),
                },
                policy: core::NetworkPolicy::DenyAll,
                operation: core::OperationId(1),
            },
            i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
            .unwrap(),
            300,
            1_000,
            config().limits,
            &mut NeverCrash,
        )
        .unwrap();
    drop(journal); // crash after admission, before the first credential file exists
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    let recovery = port.application().recover_on_startup().unwrap();
    assert_eq!(recovery.receipts_recovered, 0);
    assert_eq!(recovery.workflows_completed, 1);
    assert_eq!(recovery.unresolved_sandboxes, 0);
    assert_eq!(
        port.connect(
            "1",
            &format!("sb-{NAMESPACE}-1"),
            ConnectRequest { timeout: None }
        )
        .unwrap()
        .envd_access_token
        .len(),
        64
    );
}

#[test]
fn startup_does_not_replace_a_missing_credential_after_vm_reservation() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let sandbox = core::SandboxId(1);
    journal
        .admit_create(
            core::CreateRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                owner: core::TenantId(1),
                sandbox,
                workspace: core::FsId(1),
                file_policy: core::FilePolicyRef {
                    digest: core::PolicyDigest(1),
                },
                policy: core::NetworkPolicy::DenyAll,
                operation: core::OperationId(1),
            },
            i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
            .unwrap(),
            300,
            1_000,
            config().limits,
            &mut NeverCrash,
        )
        .unwrap();
    // Register the original token before the first claim. The restarted host
    // loses its descriptor; recovery must not replace the recorded identity.
    register_test_credential(&mut journal, sandbox, &"a".repeat(64));
    assert!(matches!(
        finish_admitted_create_v1::<Evidence, _>(
            &mut journal,
            sandbox,
            core::OperationId(1),
            &mut |_| Err(WorkerTransportError::ConnectionLost),
        ),
        Err(sandproof_controller_service::JournalError::UnresolvedAction)
    ));
    assert_eq!(journal.uncertain_actions().unwrap().len(), 1);
    drop(journal);
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    let recovery = port.application().recover_on_startup().unwrap();
    assert_eq!(recovery.workflows_completed, 0);
    assert_eq!(recovery.unresolved_sandboxes, 1);
    assert!(matches!(
        port.get("1", &format!("sb-{NAMESPACE}-1")),
        Err(PortError::NotFound)
    ));
}

#[test]
fn startup_reconciles_lost_prepare_receipt_before_resuming_create() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let sandbox = core::SandboxId(1);
    journal
        .admit_create(
            core::CreateRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                owner: core::TenantId(1),
                sandbox,
                workspace: core::FsId(1),
                file_policy: core::FilePolicyRef {
                    digest: core::PolicyDigest(1),
                },
                policy: core::NetworkPolicy::DenyAll,
                operation: core::OperationId(1),
            },
            i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
            .unwrap(),
            300,
            1_000,
            config().limits,
            &mut NeverCrash,
        )
        .unwrap();
    register_test_credential(&mut journal, sandbox, &"a".repeat(64));
    let attempted =
        finish_admitted_create_v1(&mut journal, sandbox, core::OperationId(1), &mut |action| {
            if matches!(action.command.command(), core::Command::PrepareVm { .. }) {
                Ok(WorkReceipt::<Evidence> {
                    identity: action.identity.clone(),
                    outcome: WorkOutcome::Unknown {
                        reason: UnknownReason::ObservationUnavailable,
                        last_observation: None,
                    },
                })
            } else {
                FakeHost::default().execute(action)
            }
        });
    assert!(attempted.is_err());
    drop(journal);
    let mut host = FakeHost {
        prepare: PrepareObservation::Successful,
        ..FakeHost::default()
    };
    host.put_secret(sandbox, &"a".repeat(64)).unwrap();
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        host,
        config(),
    );
    let recovery = port.application().recover_on_startup().unwrap();
    assert_eq!(recovery.receipts_recovered, 1);
    assert_eq!(recovery.workflows_completed, 1);
    assert_eq!(recovery.unresolved_sandboxes, 0);
    assert_eq!(
        port.get("1", &format!("sb-{NAMESPACE}-1"))
            .unwrap()
            .info
            .state,
        "running"
    );
    let again = port.application().recover_on_startup().unwrap();
    assert_eq!(again.receipts_recovered, 0);
    assert_eq!(again.workflows_completed, 0);
    assert_eq!(again.unresolved_sandboxes, 0);
}

fn two_unknown_prepares() -> (TempDir, ControllerJournal) {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let now_ms = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    for id in [1, 2] {
        let sandbox = core::SandboxId(id);
        journal
            .admit_create(
                core::CreateRequest {
                    actor: core::Actor::Tenant(core::TenantId(id)),
                    owner: core::TenantId(id),
                    sandbox,
                    workspace: core::FsId(id),
                    file_policy: core::FilePolicyRef {
                        digest: core::PolicyDigest(1),
                    },
                    policy: core::NetworkPolicy::DenyAll,
                    operation: core::OperationId(1),
                },
                now_ms,
                300,
                1_000,
                config().limits,
                &mut NeverCrash,
            )
            .unwrap();
        register_test_credential(&mut journal, sandbox, &format!("{id:064x}"));
        let attempted =
            finish_admitted_create_v1(&mut journal, sandbox, core::OperationId(1), &mut |action| {
                if matches!(action.command.command(), core::Command::PrepareVm { .. }) {
                    Ok(WorkReceipt::<Evidence> {
                        identity: action.identity.clone(),
                        outcome: WorkOutcome::Unknown {
                            reason: UnknownReason::ObservationUnavailable,
                            last_observation: None,
                        },
                    })
                } else {
                    FakeHost::default().execute(action)
                }
            });
        assert!(attempted.is_err());
    }
    assert_eq!(journal.uncertain_actions().unwrap().len(), 2);
    (directory, journal)
}

#[test]
fn failed_prepare_reclaim_parks_only_its_sandbox_without_relaunching_either_vm() {
    let (directory, journal) = two_unknown_prepares();
    let path = directory.path().join("controller.db");
    drop(journal);
    let prepare_calls = Arc::new(AtomicUsize::new(0));
    let mut host = FakeHost {
        prepare: PrepareObservation::Successful,
        reclaim_prepare_error_for: Some(core::SandboxId(1)),
        prepare_calls: Some(Arc::clone(&prepare_calls)),
        ..FakeHost::default()
    };
    for id in [1, 2] {
        host.put_secret(core::SandboxId(id), &format!("{id:064x}"))
            .unwrap();
    }
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        host,
        config(),
    );
    let first = port.application().recover_on_startup().unwrap();
    assert_eq!(first.receipts_recovered, 1);
    assert_eq!(first.workflows_completed, 1);
    assert_eq!(first.unresolved_sandboxes, 1);
    assert_eq!(first.failed_sandboxes, 1);
    let fault = first.first_fault.unwrap();
    assert_eq!(fault.scope, FaultScope::Instance);
    assert_eq!(fault.stage, FaultStage::Reclaim);
    assert_eq!(fault.context.sandbox, Some(1));
    assert_eq!(fault.context.operation, Some(1));
    assert_eq!(fault.context.attempt, Some(0));
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 0);
    assert_eq!(port.create("1", request()), Err(PortError::QuotaExceeded));
    assert_eq!(
        port.get("2", &format!("sb-{NAMESPACE}-2"))
            .unwrap()
            .info
            .state,
        "running"
    );
    assert!(matches!(
        port.get("1", &format!("sb-{NAMESPACE}-1")),
        Err(PortError::NotFound)
    ));
    drop(port);
    let journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    assert_eq!(journal.uncertain_actions().unwrap().len(), 1);
    let mut host = FakeHost {
        prepare: PrepareObservation::Successful,
        prepare_calls: Some(Arc::clone(&prepare_calls)),
        ..FakeHost::default()
    };
    host.put_secret(core::SandboxId(1), &format!("{:064x}", 1))
        .unwrap();
    let port = new_port(journal, host, config());
    let resumed = port.application().recover_on_startup().unwrap();
    assert_eq!(resumed.receipts_recovered, 1);
    assert_eq!(resumed.workflows_completed, 1);
    assert_eq!(resumed.unresolved_sandboxes, 0);
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        port.get("1", &format!("sb-{NAMESPACE}-1"))
            .unwrap()
            .info
            .state,
        "running"
    );
}

#[test]
fn periodic_recovery_reclaims_incomplete_prepare_without_replaying_launch() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let sandbox = core::SandboxId(1);
    journal
        .admit_create(
            core::CreateRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                owner: core::TenantId(1),
                sandbox,
                workspace: core::FsId(1),
                file_policy: core::FilePolicyRef {
                    digest: core::PolicyDigest(1),
                },
                policy: core::NetworkPolicy::DenyAll,
                operation: core::OperationId(1),
            },
            i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
            .unwrap(),
            300,
            1_000,
            config().limits,
            &mut NeverCrash,
        )
        .unwrap();
    register_test_credential(&mut journal, sandbox, &"a".repeat(64));
    let pending =
        finish_admitted_create_v1(&mut journal, sandbox, core::OperationId(1), &mut |action| {
            if matches!(action.command.command(), core::Command::PrepareVm { .. }) {
                Ok(WorkReceipt::<Evidence> {
                    identity: action.identity.clone(),
                    outcome: WorkOutcome::Unknown {
                        reason: UnknownReason::ObservationUnavailable,
                        last_observation: None,
                    },
                })
            } else {
                FakeHost::default().execute(action)
            }
        });
    assert!(matches!(
        pending,
        Err(sandproof_controller_service::JournalError::UnresolvedAction)
    ));
    drop(journal);
    let mut host = FakeHost {
        prepare: PrepareObservation::Incomplete,
        ..FakeHost::default()
    };
    host.put_secret(sandbox, &"a".repeat(64)).unwrap();
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        host,
        config(),
    );
    let recovered = port.application().recover_on_startup().unwrap();
    assert_eq!(recovered.receipts_recovered, 1);
    assert_eq!(recovered.unresolved_sandboxes, 0);
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .receipts_recovered,
        0
    );
    drop(port);
    let journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    assert!(journal.uncertain_actions().unwrap().is_empty());
    assert_ne!(
        journal
            .world()
            .unwrap()
            .sandbox_record(sandbox)
            .unwrap()
            .phase,
        core::Phase::Running
    );
}

#[test]
fn startup_reconciles_lost_stop_receipt_before_publishing_destroyed() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    let sandbox_id = port.create("1", request()).unwrap().sandbox_id;
    drop(port);

    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let sandbox = core::SandboxId(1);
    let lost = kill_v1(
        &mut journal,
        core::KillRequest {
            actor: core::Actor::Tenant(core::TenantId(1)),
            sandbox,
            operation: core::OperationId(2),
        },
        &mut |action| {
            if matches!(action.command.command(), core::Command::StopVm { .. }) {
                Ok(WorkReceipt::<Evidence> {
                    identity: action.identity.clone(),
                    outcome: WorkOutcome::Unknown {
                        reason: UnknownReason::ObservationUnavailable,
                        last_observation: None,
                    },
                })
            } else {
                FakeHost::default().execute(action)
            }
        },
    );
    assert!(matches!(
        lost,
        Err(sandproof_controller_service::JournalError::UnresolvedAction)
    ));
    assert_ne!(
        journal
            .world()
            .unwrap()
            .sandbox_record(sandbox)
            .unwrap()
            .phase,
        core::Phase::Destroyed
    );
    let uncertain = journal.uncertain_actions().unwrap();
    assert_eq!(uncertain.len(), 1);
    assert!(matches!(
        uncertain[0].command.command(),
        core::Command::StopVm { .. }
    ));
    let late_stop = FakeHost::default().execute(&uncertain[0]).unwrap();
    drop(journal);

    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        FakeHost {
            stop: StopObservation::ObservedAfterRestart,
            ..FakeHost::default()
        },
        config(),
    );
    let recovery = port.application().recover_on_startup().unwrap();
    assert_eq!(recovery.receipts_recovered, 1);
    assert_eq!(recovery.workflows_completed, 1);
    assert_eq!(recovery.unresolved_sandboxes, 0);
    assert_eq!(port.get("1", &sandbox_id), Err(PortError::NotFound));
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .receipts_recovered,
        0
    );
    drop(port);

    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let destroyed = journal.world().unwrap();
    assert_eq!(
        destroyed.sandbox_record(sandbox).unwrap().phase,
        core::Phase::Destroyed
    );
    assert_eq!(
        journal.complete(&late_stop, &mut NeverCrash).unwrap(),
        destroyed
    );
    assert!(journal.uncertain_actions().unwrap().is_empty());
}

#[test]
fn interrupted_stop_is_finished_by_current_terminal_operation_without_new_kill() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    let sandbox_id = port.create("1", request()).unwrap().sandbox_id;
    drop(port);

    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let sandbox = core::SandboxId(1);
    let pending = kill_v1(
        &mut journal,
        core::KillRequest {
            actor: core::Actor::Tenant(core::TenantId(1)),
            sandbox,
            operation: core::OperationId(2),
        },
        &mut |action| {
            FakeHost {
                stop: StopObservation::Incomplete,
                ..FakeHost::default()
            }
            .execute(action)
        },
    );
    assert!(matches!(
        pending,
        Err(sandproof_controller_service::JournalError::UnresolvedAction)
    ));
    let uncertain = journal.uncertain_actions().unwrap();
    assert_eq!(uncertain.len(), 1);
    assert!(matches!(
        uncertain[0].command.command(),
        core::Command::StopVm { .. }
    ));
    let world = journal.world().unwrap();
    assert!(journal.matches_current_stop(&world, &uncertain[0]).unwrap());
    let mut stale = uncertain[0].clone();
    stale.identity.generation += 1;
    assert!(!journal.matches_current_stop(&world, &stale).unwrap());
    assert_ne!(
        world.sandbox_record(sandbox).unwrap().phase,
        core::Phase::Destroyed
    );
    drop(journal);

    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        FakeHost {
            stop: StopObservation::Incomplete,
            ..FakeHost::default()
        },
        config(),
    );
    let report = port.application().recover_on_startup().unwrap();
    assert_eq!(report.receipts_recovered, 1);
    assert_eq!(report.workflows_completed, 1);
    assert_eq!(report.unresolved_sandboxes, 0);
    assert_eq!(port.get("1", &sandbox_id), Err(PortError::NotFound));
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .receipts_recovered,
        0
    );
    drop(port);

    let journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(sandbox)
            .unwrap()
            .phase,
        core::Phase::Destroyed
    );
    assert_eq!(
        journal
            .world()
            .unwrap()
            .operation_records()
            .iter()
            .filter(|(key, operation)| {
                key.sandbox == sandbox && operation.operation.kind == core::OperationKind::Kill
            })
            .count(),
        1
    );
}

#[test]
fn periodic_recovery_completes_partial_cleanup_before_destroyed_publication() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    let sandbox_id = port.create("1", request()).unwrap().sandbox_id;
    drop(port);

    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let sandbox = core::SandboxId(1);
    let result = kill_v1(
        &mut journal,
        core::KillRequest {
            actor: core::Actor::Tenant(core::TenantId(1)),
            sandbox,
            operation: core::OperationId(2),
        },
        &mut |action| {
            if matches!(action.command.command(), core::Command::CleanupVm { .. }) {
                Ok(WorkReceipt::<Evidence> {
                    identity: action.identity.clone(),
                    outcome: WorkOutcome::Unknown {
                        reason: UnknownReason::ObservationUnavailable,
                        last_observation: None,
                    },
                })
            } else {
                FakeHost::default().execute(action)
            }
        },
    );
    assert!(matches!(
        result,
        Err(sandproof_controller_service::JournalError::UnresolvedAction)
    ));
    let actions = journal.uncertain_actions().unwrap();
    assert_eq!(actions.len(), 1);
    assert!(matches!(
        actions[0].command.command(),
        core::Command::CleanupVm { .. }
    ));
    assert!(
        journal
            .matches_current_cleanup(&journal.world().unwrap(), &actions[0])
            .unwrap()
    );
    let mut foreign = actions[0].clone();
    foreign.identity.operation = sandproof_protocol::OperationId::try_from("op-wrong").unwrap();
    assert!(
        !journal
            .matches_current_cleanup(&journal.world().unwrap(), &foreign)
            .unwrap()
    );
    let late = FakeHost::default().execute(&actions[0]).unwrap();
    drop(journal);

    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    let recovered = port.application().recover_on_startup().unwrap();
    assert_eq!(recovered.receipts_recovered, 1);
    assert_eq!(recovered.workflows_completed, 1);
    assert_eq!(recovered.unresolved_sandboxes, 0);
    assert_eq!(port.get("1", &sandbox_id), Err(PortError::NotFound));
    let idle = port.application().recover_on_startup().unwrap();
    assert_eq!(idle.receipts_recovered, 0);
    assert_eq!(idle.workflows_completed, 0);
    assert_eq!(idle.unresolved_sandboxes, 0);
    drop(port);

    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let destroyed = journal.world().unwrap();
    assert_eq!(
        destroyed.sandbox_record(sandbox).unwrap().phase,
        core::Phase::Destroyed
    );
    assert_eq!(journal.complete(&late, &mut NeverCrash).unwrap(), destroyed);
}

#[test]
fn failed_create_partial_cleanup_recovers_without_restarting_the_vm() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let sandbox = core::SandboxId(1);
    admit_public_create(&mut journal);
    register_test_credential(&mut journal, sandbox, &"a".repeat(64));
    let result =
        finish_admitted_create_v1(&mut journal, sandbox, core::OperationId(1), &mut |action| {
            match action.command.command() {
                core::Command::ConfigureNetwork { .. } => Ok(WorkReceipt::<Evidence> {
                    identity: action.identity.clone(),
                    outcome: WorkOutcome::DefinitiveFailure {
                        code: FailureCode::PolicyRejected,
                        retryable: false,
                        partial: None,
                    },
                }),
                core::Command::CleanupVm { .. } => Ok(WorkReceipt::<Evidence> {
                    identity: action.identity.clone(),
                    outcome: WorkOutcome::Unknown {
                        reason: UnknownReason::ObservationUnavailable,
                        last_observation: None,
                    },
                }),
                _ => FakeHost::default().execute(action),
            }
        });
    assert!(result.is_err());
    let uncertain = journal.uncertain_actions().unwrap();
    assert_eq!(uncertain.len(), 1);
    assert!(matches!(
        uncertain[0].command.command(),
        core::Command::CleanupVm { .. }
    ));
    assert!(
        journal
            .matches_current_cleanup(&journal.world().unwrap(), &uncertain[0])
            .unwrap()
    );
    drop(journal);
    let mut host = FakeHost::default();
    host.put_secret(sandbox, &"a".repeat(64)).unwrap();
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        host,
        config(),
    );
    let recovered = port.application().recover_on_startup().unwrap();
    assert_eq!(recovered.receipts_recovered, 1);
    assert_eq!(recovered.unresolved_sandboxes, 0);
    let idle = port.application().recover_on_startup().unwrap();
    assert_eq!(idle.receipts_recovered, 0);
    assert_eq!(idle.workflows_completed, 0);
    assert_eq!(idle.unresolved_sandboxes, 0);
}

fn request() -> CreateRequest {
    CreateRequest {
        template_id: "base".into(),
        timeout: Some(300),
        allow_internet_access: Some(false),
        network: None,
        metadata: None,
        env_vars: None,
    }
}

fn template_catalog() -> sandproof_controller_service::templates::TemplateCatalog {
    use sandproof_controller_service::templates::{
        ComputeLimits, ResourceShape, TemplateArtifact, TemplateBinding, TemplateCatalog,
    };
    let first = TemplateBinding {
        startup: None,
        template_id: "ubuntu-r1".into(),
        revision: "r1".into(),
        owner: Some(1),
        kernel: TemplateArtifact {
            path: "/opt/images/vmlinux".into(),
            sha256: "a".repeat(64),
            size_bytes: 1024,
        },
        rootfs: TemplateArtifact {
            path: "/opt/images/ubuntu-r1.ext4".into(),
            sha256: "b".repeat(64),
            size_bytes: 1000,
        },
        resources: ResourceShape {
            cpu_count: 1,
            memory_mib: 128,
            disk_bytes: 1000,
            host_memory_bytes: 256 * 1024 * 1024,
        },
        envd_version: "0.9.0-r1".into(),
    };
    let mut second = first.clone();
    second.template_id = "ubuntu-r2".into();
    second.revision = "r2".into();
    second.owner = None;
    second.rootfs.path = "/opt/images/ubuntu-r2.ext4".into();
    second.rootfs.size_bytes = 2000;
    second.resources.disk_bytes = 2000;
    second.resources.cpu_count = 2;
    second.resources.memory_mib = 256;
    second.resources.host_memory_bytes = 512 * 1024 * 1024;
    second.envd_version = "0.9.0-r2".into();
    TemplateCatalog {
        templates: vec![first, second],
        aliases: BTreeMap::from([("ubuntu".into(), "ubuntu-r1".into())]),
        compute_limits: ComputeLimits {
            max_vcpus_global: 3,
            max_vcpus_per_tenant: 3,
            max_memory_bytes_global: 768 * 1024 * 1024,
            max_memory_bytes_per_tenant: 768 * 1024 * 1024,
        },
    }
}

fn template_config() -> ManagedConfig {
    let mut settings = config();
    settings.limits.max_active_per_tenant = 2;
    settings.limits.max_active_global = 10;
    settings.limits.max_disk_bytes_per_tenant = 10_000;
    settings.limits.max_disk_bytes_global = complete_fixture::CLEANUP + 19_000;
    settings
}

fn template_request(name: &str) -> CreateRequest {
    CreateRequest {
        template_id: name.into(),
        ..request()
    }
}

#[tokio::test]
async fn template_http_selection_projection_and_compute_quota_release() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let port = Arc::new(
        with_templates(
            complete_fixture::open_with_config(&path, NAMESPACE, &template_config()).unwrap(),
            FakeHost::default(),
            template_config(),
            template_catalog(),
        )
        .unwrap(),
    );
    assert_eq!(
        port.create("2", template_request("ubuntu")),
        Err(PortError::NotFound)
    );
    assert_eq!(
        port.create("1", template_request("missing")),
        Err(PortError::NotFound)
    );
    let app = router(
        port.clone(),
        vec![ApiCredential {
            tenant: "1".into(),
            secret: KEY_ONE.into(),
        }],
    )
    .unwrap();
    let reply = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v2/sandboxes")
                .header("X-API-Key", KEY_ONE)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    r#"{"templateID":"ubuntu","allow_internet_access":false}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::CREATED);
    let first: serde_json::Value =
        serde_json::from_slice(&to_bytes(reply.into_body(), 1_000_000).await.unwrap()).unwrap();
    assert_eq!(first["templateID"], "ubuntu-r1");
    assert_eq!(first["envdVersion"], "0.9.0-r1");
    let first_id = first["sandboxID"].as_str().unwrap();
    let second = port.create("1", template_request("ubuntu-r2")).unwrap();
    let detail = port.get("1", &second.sandbox_id).unwrap();
    assert_eq!(
        (
            detail.info.cpu_count,
            detail.info.memory_mb,
            detail.info.disk_size_mb
        ),
        (2, 256, 1)
    );
    assert_eq!(
        port.list(
            "1",
            ListQuery {
                template: Some("ubuntu-r2".into()),
                ..ListQuery::default()
            }
        )
        .unwrap()
        .items
        .len(),
        1
    );
    // Count and disk still fit; the compute budget alone refuses a third VM.
    assert_eq!(
        port.create("1", template_request("ubuntu")),
        Err(PortError::QuotaExceeded)
    );
    port.kill("1", first_id).unwrap();
    let replacement = port.create("1", template_request("ubuntu")).unwrap();
    port.kill("1", &replacement.sandbox_id).unwrap();
    port.kill("1", &second.sandbox_id).unwrap();
}

#[test]
fn alias_change_after_restart_does_not_rebind_old_instance() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let old = template_catalog();
    let token = "a".repeat(64);
    let mut host = FakeHost::default();
    host.put_bound_launch_config(
        core::SandboxId(1),
        &token,
        &sandproof_controller_service::CreateOptions {
            template: Some(old.templates[0].clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let mut journal =
        complete_fixture::open_with_config(&path, NAMESPACE, &template_config()).unwrap();
    journal
        .admit_template_create(
            core::CreateRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                owner: core::TenantId(1),
                sandbox: core::SandboxId(1),
                workspace: core::FsId(1),
                file_policy: core::FilePolicyRef {
                    digest: core::PolicyDigest(1),
                },
                policy: core::NetworkPolicy::DenyAll,
                operation: core::OperationId(1),
            },
            CreateAdmission {
                now_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis()
                    .try_into()
                    .unwrap(),
                timeout_secs: 300,
                disk_bytes: 1000,
                limits: template_config().limits,
                options: sandproof_controller_service::CreateOptions {
                    template: Some(old.templates[0].clone()),
                    ..Default::default()
                },
            },
            old.compute_limits,
            &mut NeverCrash,
        )
        .unwrap();
    register_test_credential(&mut journal, core::SandboxId(1), &token);
    finish_admitted_create_v1(
        &mut journal,
        core::SandboxId(1),
        core::OperationId(1),
        &mut |action| host.execute(action),
    )
    .unwrap();
    drop(journal);
    let mut changed = old;
    changed.aliases.insert("ubuntu".into(), "ubuntu-r2".into());
    let port = with_templates(
        complete_fixture::open_with_config(&path, NAMESPACE, &template_config()).unwrap(),
        host,
        template_config(),
        changed,
    )
    .unwrap();
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .unresolved_sandboxes,
        0
    );
    let detail = port.get("1", &format!("sb-{NAMESPACE}-1")).unwrap();
    assert_eq!(
        (
            detail.info.template_id.as_str(),
            detail.info.cpu_count,
            detail.envd_access_token.as_str()
        ),
        ("ubuntu-r1", 1, token.as_str())
    );
    assert_eq!(
        port.connect(
            "1",
            &format!("sb-{NAMESPACE}-1"),
            ConnectRequest { timeout: None }
        )
        .unwrap()
        .envd_version,
        "0.9.0-r1"
    );
    let second = port.create("1", template_request("ubuntu")).unwrap();
    assert_eq!(second.template_id, "ubuntu-r2");
    port.kill("1", &second.sandbox_id).unwrap();
    port.kill("1", &detail.info.sandbox_id).unwrap();
}

#[test]
fn idle_recovery_does_not_revisit_reclaimed_history_or_change_live_authority() {
    let prepare_calls = Arc::new(AtomicUsize::new(0));
    let stop_calls = Arc::new(AtomicUsize::new(0));
    let (_directory, port) = setup_with_host(FakeHost {
        prepare_calls: Some(Arc::clone(&prepare_calls)),
        stop_calls: Some(Arc::clone(&stop_calls)),
        ..FakeHost::default()
    });
    let mut retired = Vec::new();
    for _ in 0..3 {
        let session = port.create("1", request()).unwrap();
        port.kill("1", &session.sandbox_id).unwrap();
        retired.push(session);
    }
    let live = port.create("1", request()).unwrap();
    for _ in 0..2 {
        let report = port.application().recover_on_startup().unwrap();
        assert_eq!(
            report.workflows_completed, 0,
            "completed history was resumed"
        );
        assert_eq!(report.receipts_recovered, 0);
        assert_eq!(report.unresolved_sandboxes, 0);
        assert_eq!(prepare_calls.load(Ordering::SeqCst), 4);
        assert_eq!(stop_calls.load(Ordering::SeqCst), 3);
        assert_eq!(port.list("1", ListQuery::default()).unwrap().items.len(), 1);
        assert!(
            port.application()
                .authorize_data(
                    &live.envd_access_token,
                    sandproof_controller_service::application::DataKind::Exec
                )
                .is_ok()
        );
        for session in &retired {
            assert_eq!(
                port.application().authorize_data(
                    &session.envd_access_token,
                    sandproof_controller_service::application::DataKind::Exec
                ),
                Err(PortError::NotFound)
            );
        }
    }
    port.kill("1", &live.sandbox_id).unwrap();
}

#[test]
fn public_calls_use_the_durable_reducer_and_owner_lifetime_checks() {
    let (_directory, port) = setup();
    let created = port.create("1", request()).unwrap();
    assert_eq!(created.sandbox_id, format!("sb-{NAMESPACE}-1"));
    assert_eq!(created.envd_access_token.len(), 64);
    for kind in [
        sandproof_controller_service::application::DataKind::Exec,
        sandproof_controller_service::application::DataKind::FileRead,
        sandproof_controller_service::application::DataKind::FileWrite,
    ] {
        assert_eq!(
            port.application()
                .authorize_data(&created.envd_access_token, kind)
                .unwrap()
                .generation,
            1
        );
    }
    assert_eq!(
        port.application().authorize_data(
            &"0".repeat(64),
            sandproof_controller_service::application::DataKind::Exec
        ),
        Err(PortError::NotFound)
    );
    assert!(matches!(
        port.create("1", request()),
        Err(PortError::QuotaExceeded)
    ));
    assert_eq!(port.get("2", &created.sandbox_id), Err(PortError::NotFound));
    assert_eq!(
        port.connect("2", &created.sandbox_id, ConnectRequest::default()),
        Err(PortError::NotFound)
    );
    assert_eq!(
        port.kill("2", &created.sandbox_id),
        Err(PortError::NotFound)
    );
    assert!(
        port.list("2", ListQuery::default())
            .unwrap()
            .items
            .is_empty()
    );

    let info = port.get("1", &created.sandbox_id).unwrap();
    assert_eq!(info.info.state, "running");
    assert!(info.info.started_at.ends_with('Z'));
    assert!(info.info.end_at.ends_with('Z'));
    assert_eq!(
        port.list("1", ListQuery::default()).unwrap().items,
        vec![info.info]
    );
    assert_eq!(
        port.connect("1", &created.sandbox_id, ConnectRequest { timeout: None })
            .unwrap()
            .envd_access_token,
        created.envd_access_token
    );
    assert_eq!(port.set_timeout("1", &created.sandbox_id, 60), Ok(()));
    assert_eq!(port.kill("1", &created.sandbox_id), Ok(()));
    assert_eq!(
        port.application().authorize_data(
            &created.envd_access_token,
            sandproof_controller_service::application::DataKind::Exec
        ),
        Err(PortError::NotFound)
    );
    assert_eq!(port.get("1", &created.sandbox_id), Err(PortError::NotFound));
    assert!(
        port.list("1", ListQuery::default())
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn create_environment_and_metadata_follow_one_durable_identity() {
    let (directory, port) = setup();
    let mut input = request();
    input.env_vars = Some(BTreeMap::from([("TASK_ID".into(), "job-7".into())]));
    input.metadata = Some(BTreeMap::from([("job".into(), "job-7".into())]));
    let session = port.create("1", input).unwrap();
    assert_eq!(
        port.get("1", &session.sandbox_id)
            .unwrap()
            .info
            .metadata
            .get("job"),
        Some(&"job-7".to_owned())
    );
    drop(port);
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let options = journal.create_options(core::SandboxId(1)).unwrap();
    assert_eq!(options.env_vars.get("TASK_ID"), Some(&"job-7".to_owned()));
    assert_eq!(options.metadata.get("job"), Some(&"job-7".to_owned()));
}

#[test]
fn details_preserve_original_networking_after_reopen_without_list_credentials() {
    use sandproof_api::CreateNetwork;
    for internet in [None, Some(false), Some(true)] {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("controller.db");
        let mut configured = config();
        configured.public_ipv4_enabled = true;
        let mut input = request();
        input.allow_internet_access = internet;
        input.network = Some(CreateNetwork {
            allow_public_traffic: Some(true),
            allow_out: Some(vec!["1.2.3.4/24".into(), "1.2.3.0/24".into()]),
            deny_out: Some(vec![]),
        });
        let expected_network = input.network.clone();
        let port = new_port(
            complete_fixture::open(&path, NAMESPACE).unwrap(),
            FakeHost {
                public_ipv4_ready: true,
                ..FakeHost::default()
            },
            configured.clone(),
        );
        let created = port.create("1", input).unwrap();
        let before = port.get("1", &created.sandbox_id).unwrap();
        assert_eq!(before.allow_internet_access, Some(internet));
        assert_eq!(before.network, expected_network);
        assert_eq!(before.envd_access_token, created.envd_access_token);
        assert_eq!(before.domain, configured.domain);
        drop(port);
        let host = FakeHost {
            secrets: BTreeMap::from([(1, (created.envd_access_token.clone(), BTreeMap::new()))]),
            ..FakeHost::default()
        };
        let port = new_port(
            complete_fixture::open(&path, NAMESPACE).unwrap(),
            host,
            configured,
        );
        assert_eq!(port.get("1", &created.sandbox_id).unwrap(), before);
        assert_eq!(port.get("2", &created.sandbox_id), Err(PortError::NotFound));
        let listed =
            serde_json::to_value(port.list("1", ListQuery::default()).unwrap().items).unwrap();
        assert!(listed[0].get("envdAccessToken").is_none());
        assert!(listed[0].get("network").is_none());
        port.kill("1", &created.sandbox_id).unwrap();
        assert_eq!(port.get("1", &created.sandbox_id), Err(PortError::NotFound));
    }
}

#[test]
fn foreign_detail_and_list_do_not_read_the_host_token() {
    let (directory, port) = setup();
    let created = port.create("1", request()).unwrap();
    drop(port);
    let host = FakeHost {
        token_error_once: AtomicBool::new(true),
        secrets: BTreeMap::from([(1, (created.envd_access_token, BTreeMap::new()))]),
        ..FakeHost::default()
    };
    let port = new_port(
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap(),
        host,
        config(),
    );
    assert_eq!(port.get("2", &created.sandbox_id), Err(PortError::NotFound));
    assert_eq!(port.list("1", ListQuery::default()).unwrap().items.len(), 1);
    assert_eq!(port.get("1", &created.sandbox_id), Err(PortError::Busy));
    assert!(port.get("1", &created.sandbox_id).is_ok());
    port.kill("1", &created.sandbox_id).unwrap();
}

#[test]
fn restart_uses_admitted_environment_and_rejects_a_mismatched_host_record() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let options = sandproof_controller_service::CreateOptions {
        env_vars: BTreeMap::from([("TASK_ID".into(), "persisted".into())]),
        metadata: BTreeMap::from([("job".into(), "persisted".into())]),
        ..Default::default()
    };
    journal
        .admit_create_with_options(
            core::CreateRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                owner: core::TenantId(1),
                sandbox: core::SandboxId(1),
                workspace: core::FsId(1),
                file_policy: core::FilePolicyRef {
                    digest: core::PolicyDigest(1),
                },
                policy: core::NetworkPolicy::DenyAll,
                operation: core::OperationId(1),
            },
            CreateAdmission {
                now_ms: i64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis(),
                )
                .unwrap(),
                timeout_secs: 300,
                disk_bytes: 1_000,
                limits: config().limits,
                options: options.clone(),
            },
            &mut NeverCrash,
        )
        .unwrap();
    drop(journal);

    let mut wrong_host = FakeHost::default();
    wrong_host
        .put_secret(core::SandboxId(1), &"a".repeat(64))
        .unwrap();
    let wrong = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        wrong_host,
        config(),
    );
    assert_eq!(
        wrong
            .application()
            .recover_on_startup()
            .unwrap()
            .unresolved_sandboxes,
        1
    );
    assert_eq!(
        wrong.get("1", &format!("sb-{NAMESPACE}-1")),
        Err(PortError::NotFound)
    );
    drop(wrong);

    let good = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    assert_eq!(
        good.application()
            .recover_on_startup()
            .unwrap()
            .workflows_completed,
        1
    );
    assert_eq!(
        good.get("1", &format!("sb-{NAMESPACE}-1"))
            .unwrap()
            .info
            .metadata,
        options.metadata
    );
    let legacy =
        serde_json::to_value(good.get("1", &format!("sb-{NAMESPACE}-1")).unwrap()).unwrap();
    assert!(legacy.get("allowInternetAccess").is_none());
    assert!(legacy.get("network").is_none());
    assert_eq!(
        good.application()
            .recover_on_startup()
            .unwrap()
            .unresolved_sandboxes,
        0
    );
}

#[test]
fn data_revocation_is_scoped_and_cannot_be_undone_by_a_new_instance() {
    use sandproof_controller_service::application::DataKind;
    let (_directory, port) = setup();
    let first = port.create("1", request()).unwrap();
    let second = port.create("2", request()).unwrap();
    let first_routes = [DataKind::Exec, DataKind::FileRead, DataKind::FileWrite].map(|kind| {
        port.application()
            .data_permit(&first.envd_access_token, kind)
            .unwrap()
    });
    let other_route = port
        .application()
        .data_permit(&second.envd_access_token, DataKind::Exec)
        .unwrap();
    assert!(
        port.application()
            .data_permit(&"0".repeat(64), DataKind::Exec)
            .is_err()
    );
    assert_eq!(port.kill("2", &first.sandbox_id), Err(PortError::NotFound));
    port.set_timeout("1", &first.sandbox_id, 60).unwrap();
    assert_eq!(port.application().expire_due().unwrap(), 0);
    assert!(first_routes.iter().all(|route| !*route.revocation.borrow()));
    port.kill("1", &first.sandbox_id).unwrap();
    assert!(first_routes.iter().all(|route| *route.revocation.borrow()));
    assert!(!*other_route.revocation.borrow());
    assert!(
        port.application()
            .data_permit(&first.envd_access_token, DataKind::Exec)
            .is_err()
    );
    let replacement = port.create("1", request()).unwrap();
    let new_route = port
        .application()
        .data_permit(&replacement.envd_access_token, DataKind::Exec)
        .unwrap();
    assert_ne!(new_route.target.vm, first_routes[0].target.vm);
    assert!(!*new_route.revocation.borrow());
    assert!(first_routes.iter().all(|route| *route.revocation.borrow()));
    port.kill("1", &replacement.sandbox_id).unwrap();
    assert!(!*other_route.revocation.borrow());
    port.kill("2", &second.sandbox_id).unwrap();
}

#[test]
fn nonpositive_timeout_revokes_existing_transfers_before_the_expiry_worker_runs() {
    use sandproof_controller_service::application::DataKind;
    for seconds in [0, -1, i64::MIN] {
        let (directory, port) = setup();
        let first = port.create("1", request()).unwrap();
        let other = port.create("2", request()).unwrap();
        let routes = [DataKind::Exec, DataKind::FileRead, DataKind::FileWrite].map(|kind| {
            port.application()
                .data_permit(&first.envd_access_token, kind)
                .unwrap()
        });
        let other_route = port
            .application()
            .data_permit(&other.envd_access_token, DataKind::Exec)
            .unwrap();
        assert_eq!(
            port.set_timeout("2", &first.sandbox_id, seconds),
            Err(PortError::NotFound)
        );
        assert!(routes.iter().all(|route| !*route.revocation.borrow()));
        port.set_timeout("1", &first.sandbox_id, seconds).unwrap();
        assert!(routes.iter().all(|route| *route.revocation.borrow()));
        assert!(!*other_route.revocation.borrow());
        assert_eq!(port.get("1", &first.sandbox_id), Err(PortError::NotFound));
        assert_eq!(
            port.connect("1", &first.sandbox_id, ConnectRequest::default()),
            Err(PortError::NotFound)
        );
        assert_eq!(
            port.set_timeout("1", &first.sandbox_id, 300),
            Err(PortError::NotFound)
        );
        assert!(
            port.application()
                .data_permit(&first.envd_access_token, DataKind::Exec)
                .is_err()
        );
        assert_eq!(port.application().expire_due().unwrap(), 1);
        assert_eq!(port.application().expire_due().unwrap(), 0);
        port.kill("2", &other.sandbox_id).unwrap();
        drop(port);
        let journal =
            complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
        assert!(
            journal
                .world()
                .unwrap()
                .sandbox_records()
                .iter()
                .all(|(_, record)| record.phase == core::Phase::Destroyed)
        );
    }
}

#[test]
fn fixed_harbor_day_request_is_preserved_when_the_trusted_budget_allows_it() {
    let (_directory, port) = setup();
    let mut input = request();
    input.timeout = Some(86_400);
    let created = port.create("1", input).unwrap();
    let before = port.get("1", &created.sandbox_id).unwrap().info;
    let start = time::OffsetDateTime::parse(
        &before.started_at,
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    let end = time::OffsetDateTime::parse(
        &before.end_at,
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    assert_eq!((end - start).whole_seconds(), 86_400);
    port.connect("1", &created.sandbox_id, ConnectRequest::default())
        .unwrap();
    assert_eq!(
        port.get("1", &created.sandbox_id).unwrap().info.end_at,
        before.end_at
    );
    port.kill("1", &created.sandbox_id).unwrap();
}

#[test]
fn renewal_contending_with_expiry_cleanup_cannot_resurrect_the_instance() {
    for connect in [false, true] {
        let gate = Arc::new(ActionGate::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let (directory, port) = setup_with_host(FakeHost {
            stop_gate: Some(Arc::clone(&gate)),
            stop_calls: Some(Arc::clone(&calls)),
            ..FakeHost::default()
        });
        let created = port.create("1", request()).unwrap();
        port.set_timeout("1", &created.sandbox_id, 0).unwrap();
        let expiring = Arc::clone(&port);
        let worker = std::thread::spawn(move || expiring.application().expire_due());
        gate.entered.wait(); // expiry is durable and its exact host stop is in flight
        let renewing = Arc::clone(&port);
        let ready = Arc::new(Barrier::new(2));
        let caller_ready = Arc::clone(&ready);
        let caller = std::thread::spawn(move || {
            caller_ready.wait();
            if connect {
                renewing
                    .connect("1", &created.sandbox_id, ConnectRequest::default())
                    .map(|_| ())
            } else {
                renewing.set_timeout("1", &created.sandbox_id, 300)
            }
        });
        ready.wait();
        gate.release.wait();
        assert_eq!(worker.join().unwrap(), Ok(1));
        assert_eq!(caller.join().unwrap(), Err(PortError::NotFound));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        drop(port);
        let journal =
            complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
        let world = journal.world().unwrap();
        assert_eq!(
            world.sandbox_record(core::SandboxId(1)).unwrap().phase,
            core::Phase::Destroyed
        );
        assert_eq!(
            world
                .operation_records()
                .iter()
                .filter(|(_, op)| op.operation.kind == core::OperationKind::Kill)
                .count(),
            1
        );
    }
}

#[test]
fn accepted_kill_revokes_routes_even_when_stop_receipt_is_unknown() {
    use sandproof_controller_service::application::DataKind;
    let (_directory, port) = setup_with_host(FakeHost {
        stop: StopObservation::LoseReceipt,
        ..FakeHost::default()
    });
    let sandbox = port.create("1", request()).unwrap();
    let route = port
        .application()
        .data_permit(&sandbox.envd_access_token, DataKind::Exec)
        .unwrap();
    assert_eq!(
        port.kill("1", &sandbox.sandbox_id),
        Err(PortError::OutcomeUnknown)
    );
    assert!(*route.revocation.borrow());
    assert!(
        port.application()
            .data_permit(&sandbox.envd_access_token, DataKind::Exec)
            .is_err()
    );
}

#[test]
fn repeated_kill_observes_one_persisted_stop_without_a_new_operation() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (directory, port) = setup_with_host(FakeHost {
        stop: StopObservation::LostThenObserved,
        stop_calls: Some(Arc::clone(&calls)),
        ..FakeHost::default()
    });
    let created = port.create("1", request()).unwrap();
    let route = port
        .application()
        .data_permit(
            &created.envd_access_token,
            sandproof_controller_service::application::DataKind::Exec,
        )
        .unwrap();
    assert_eq!(
        port.kill("1", &created.sandbox_id),
        Err(PortError::OutcomeUnknown)
    );
    assert!(*route.revocation.borrow());
    assert_eq!(port.get("1", &created.sandbox_id), Err(PortError::NotFound));
    assert_eq!(port.kill("1", &created.sandbox_id), Ok(()));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        port.kill("1", &created.sandbox_id),
        Err(PortError::NotFound)
    );
    drop(port);
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let world = journal.world().unwrap();
    assert_eq!(
        world.sandbox_record(core::SandboxId(1)).unwrap().phase,
        core::Phase::Destroyed
    );
    assert_eq!(
        world
            .operation_records()
            .iter()
            .filter(|(key, operation)| {
                key.sandbox == core::SandboxId(1)
                    && operation.operation.kind == core::OperationKind::Kill
            })
            .count(),
        1
    );
    assert!(journal.uncertain_actions().unwrap().is_empty());
}

#[tokio::test]
async fn accepted_kill_recovery_error_is_unknown_and_reuses_the_original_stop() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (directory, port) = setup_with_host(FakeHost {
        stop: StopObservation::Incomplete,
        reclaim_stop_error_once: Some(PortError::Busy),
        stop_calls: Some(Arc::clone(&calls)),
        ..FakeHost::default()
    });
    let created = port.create("1", request()).unwrap();
    assert_eq!(
        port.kill("1", &created.sandbox_id),
        Err(PortError::OutcomeUnknown)
    );
    assert_eq!(port.get("1", &created.sandbox_id), Err(PortError::NotFound));
    assert!(
        port.list("1", ListQuery::default())
            .unwrap()
            .items
            .is_empty()
    );

    let app = router(
        Arc::clone(&port),
        vec![ApiCredential {
            tenant: "1".into(),
            secret: KEY_ONE.into(),
        }],
    )
    .unwrap();
    let request = Request::builder()
        .method("DELETE")
        .uri(format!("/sandboxes/{}", created.sandbox_id))
        .header("x-api-key", KEY_ONE)
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let error: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(error["code"], 500);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    assert_eq!(port.kill("1", &created.sandbox_id), Ok(()));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        port.kill("1", &created.sandbox_id),
        Err(PortError::NotFound)
    );
    drop(port);
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert!(journal.uncertain_actions().unwrap().is_empty());
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(core::SandboxId(1))
            .unwrap()
            .phase,
        core::Phase::Destroyed
    );
}

#[test]
fn simultaneous_kills_serialize_without_a_second_stop_or_operation() {
    let gate = Arc::new(ActionGate::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let (directory, port) = setup_with_host(FakeHost {
        stop_gate: Some(Arc::clone(&gate)),
        stop_calls: Some(Arc::clone(&calls)),
        ..FakeHost::default()
    });
    let created = port.create("1", request()).unwrap();
    let first = Arc::clone(&port);
    let id = created.sandbox_id.clone();
    let first = std::thread::spawn(move || first.kill("1", &id));
    gate.entered.wait(); // first kill holds the reducer/host serialization lock
    let second = Arc::clone(&port);
    let id = created.sandbox_id.clone();
    let ready = Arc::new(Barrier::new(2));
    let second_ready = Arc::clone(&ready);
    let second = std::thread::spawn(move || {
        second_ready.wait();
        second.kill("1", &id)
    });
    ready.wait();
    gate.release.wait();
    assert_eq!(first.join().unwrap(), Ok(()));
    assert_eq!(second.join().unwrap(), Err(PortError::NotFound));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(port);
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let world = journal.world().unwrap();
    assert_eq!(
        world
            .operation_records()
            .iter()
            .filter(|(key, operation)| {
                key.sandbox == core::SandboxId(1)
                    && operation.operation.kind == core::OperationKind::Kill
            })
            .count(),
        1
    );
    assert_eq!(
        world.sandbox_record(core::SandboxId(1)).unwrap().phase,
        core::Phase::Destroyed
    );
}

#[test]
fn create_and_kill_race_has_one_prepare_one_stop_and_no_live_access() {
    let gate = Arc::new(ActionGate::new());
    let prepare_calls = Arc::new(AtomicUsize::new(0));
    let stop_calls = Arc::new(AtomicUsize::new(0));
    let (directory, port) = setup_with_host(FakeHost {
        prepare_gate: Some(Arc::clone(&gate)),
        prepare_calls: Some(Arc::clone(&prepare_calls)),
        stop_calls: Some(Arc::clone(&stop_calls)),
        ..FakeHost::default()
    });
    let creator = Arc::clone(&port);
    let creator = std::thread::spawn(move || creator.create("1", request()));
    gate.entered.wait(); // create is admitted and physical preparation is in flight
    let killer = Arc::clone(&port);
    let ready = Arc::new(Barrier::new(2));
    let killer_ready = Arc::clone(&ready);
    let killer = std::thread::spawn(move || {
        killer_ready.wait();
        killer.kill("1", &format!("sb-{NAMESPACE}-1"))
    });
    ready.wait();
    gate.release.wait();
    let session = creator.join().unwrap().unwrap();
    assert_eq!(killer.join().unwrap(), Ok(()));
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);
    assert_eq!(stop_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        port.application().authorize_data(
            &session.envd_access_token,
            sandproof_controller_service::application::DataKind::Exec
        ),
        Err(PortError::NotFound)
    );
    drop(port);
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let world = journal.world().unwrap();
    assert_eq!(
        world.sandbox_record(core::SandboxId(1)).unwrap().phase,
        core::Phase::Destroyed
    );
    assert!(journal.uncertain_actions().unwrap().is_empty());
}

#[test]
fn physical_disk_shortage_rejects_before_any_admission() {
    let (_directory, port) = setup_with_host(FakeHost {
        free_bytes: 1_999,
        ..FakeHost::default()
    });
    assert_eq!(port.create("1", request()), Err(PortError::QuotaExceeded));
    assert!(
        port.list("1", ListQuery::default())
            .unwrap()
            .items
            .is_empty()
    );
}

fn seed_checked_vm_frontier(path: &std::path::Path, next_vm_id: u64) {
    // Seed a checked current checkpoint near the real permanent-ID boundary.
    // No host resource/UID range is made into a logical identity limit.
    let database = rusqlite::Connection::open(path).unwrap();
    let payload: Vec<u8> = database
        .query_row(
            "SELECT payload FROM controller_checkpoint WHERE id=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    // Preserve the checkpoint's canonical field order: re-encoding a
    // generic JSON map would correctly be rejected as noncanonical.
    let payload = String::from_utf8(payload).unwrap();
    assert_eq!(payload.matches("\"next_vm_id\":0").count(), 1);
    let payload = payload
        .replace("\"next_vm_id\":0", &format!("\"next_vm_id\":{next_vm_id}"))
        .into_bytes();
    database
        .execute(
            "UPDATE controller_checkpoint SET payload=?1,sha256=?2 WHERE id=1",
            rusqlite::params![payload, Sha256::digest(&payload).to_vec()],
        )
        .unwrap();
}

#[test]
fn reclaimed_vms_do_not_reset_the_host_id_capacity() {
    use sandproof_controller_service::application::DataKind;

    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let prepare_calls = Arc::new(AtomicUsize::new(0));
    drop(complete_fixture::open(&path, NAMESPACE).unwrap());
    seed_checked_vm_frontier(&path, u64::MAX - 2);
    let host = FakeHost {
        prepare_calls: Some(Arc::clone(&prepare_calls)),
        ..FakeHost::default()
    };
    let mut policy = config();
    policy.limits.max_active_per_tenant = 2;
    policy.limits.max_disk_bytes_per_tenant = 4_000;
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        host,
        policy.clone(),
    );

    let created = port.create("1", request()).unwrap();
    port.kill("1", &created.sandbox_id).unwrap();
    let survivor = port.create("1", request()).unwrap();
    let before = port.get("1", &survivor.sandbox_id).unwrap();
    let permit = port
        .application()
        .data_permit(&survivor.envd_access_token, DataKind::Exec)
        .unwrap();
    assert_eq!(permit.target.vm, core::VmId(u64::MAX - 1));
    assert_eq!(port.create("1", request()), Err(PortError::QuotaExceeded));
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 2);
    assert!(!*permit.revocation.borrow());
    assert_eq!(port.get("1", &survivor.sandbox_id).unwrap(), before);
    for kind in [DataKind::Exec, DataKind::FileRead, DataKind::FileWrite] {
        assert_eq!(
            port.application()
                .authorize_data(&survivor.envd_access_token, kind)
                .unwrap(),
            permit.target
        );
    }
    drop(port);

    let reopened = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        FakeHost {
            prepare_calls: Some(Arc::clone(&prepare_calls)),
            secrets: BTreeMap::from([(2, (survivor.envd_access_token.clone(), BTreeMap::new()))]),
            ..FakeHost::default()
        },
        policy,
    );
    reopened.application().recover_on_startup().unwrap();
    assert_eq!(
        reopened.create("1", request()),
        Err(PortError::QuotaExceeded)
    );
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 2);
    assert_eq!(reopened.get("1", &survivor.sandbox_id).unwrap(), before);
    for kind in [DataKind::Exec, DataKind::FileRead, DataKind::FileWrite] {
        assert_eq!(
            reopened
                .application()
                .authorize_data(&survivor.envd_access_token, kind)
                .unwrap(),
            permit.target
        );
    }
    reopened.kill("1", &survivor.sandbox_id).unwrap();
    assert_eq!(
        reopened.create("1", request()),
        Err(PortError::QuotaExceeded)
    );
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 2);
    drop(reopened);
    let journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let world = journal.world().unwrap();
    assert_eq!(world.sandbox_records().len(), 2);
    assert!(
        world
            .sandbox_records()
            .iter()
            .all(|(_, record)| record.phase == core::Phase::Destroyed)
    );
    assert!(journal.uncertain_actions().unwrap().is_empty());
}

#[tokio::test]
async fn credential_write_failure_after_admission_is_unknown_and_recovers_without_recreate() {
    let prepare_calls = Arc::new(AtomicUsize::new(0));
    let (directory, port) = setup_with_host(FakeHost {
        launch_error_once: Some(PortError::Busy),
        prepare_calls: Some(Arc::clone(&prepare_calls)),
        ..FakeHost::default()
    });
    let app = router(
        Arc::clone(&port),
        vec![ApiCredential {
            tenant: "1".into(),
            secret: KEY_ONE.into(),
        }],
    )
    .unwrap();
    let response = app
        .oneshot(
            Request::post("/v2/sandboxes")
                .header("x-api-key", KEY_ONE)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"templateID":"base","allow_internet_access":false,"metadata":{"job":"original"}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    drop(port);
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert_eq!(journal.world().unwrap().sandbox_records().len(), 1);
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(core::SandboxId(1))
            .unwrap()
            .phase,
        core::Phase::Starting
    );
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 0);
    // HTTP 503 means a refused call to the pinned SDK. An already durable
    // create must instead report unknown, even when no host VM exists yet.
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let port = new_port(
        journal,
        FakeHost {
            prepare_calls: Some(Arc::clone(&prepare_calls)),
            ..FakeHost::default()
        },
        config(),
    );
    let recovery = port.application().recover_on_startup().unwrap();
    assert_eq!(recovery.workflows_completed, 1);
    assert_eq!(recovery.unresolved_sandboxes, 0);
    let items = port.list("1", ListQuery::default()).unwrap().items;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].sandbox_id, format!("sb-{NAMESPACE}-1"));
    assert_eq!(items[0].metadata.get("job"), Some(&"original".into()));
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);
    port.kill("1", &items[0].sandbox_id).unwrap();
    drop(port);
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert_eq!(journal.world().unwrap().sandbox_records().len(), 1);
}

#[test]
fn direct_port_cannot_bypass_public_subset_guards() {
    let (_directory, port) = setup();
    let mut unsupported = request();
    unsupported.env_vars = Some(BTreeMap::from([("A=B".into(), "x".into())]));
    assert_eq!(port.create("1", unsupported), Err(PortError::Invalid));
    let mut unsupported = request();
    unsupported.allow_internet_access = Some(true);
    assert_eq!(port.create("1", unsupported), Err(PortError::Invalid));
    assert!(
        port.list("1", ListQuery::default())
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn create_reply_failure_is_unknown_after_the_vm_has_started() {
    let prepare_calls = Arc::new(AtomicUsize::new(0));
    let (directory, port) = setup_with_host(FakeHost {
        token_error_once: AtomicBool::new(true),
        prepare_calls: Some(Arc::clone(&prepare_calls)),
        ..FakeHost::default()
    });
    assert_eq!(port.create("1", request()), Err(PortError::OutcomeUnknown));
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);
    let items = port.list("1", ListQuery::default()).unwrap().items;
    assert_eq!(items.len(), 1);
    let recovered = port
        .connect("1", &items[0].sandbox_id, ConnectRequest { timeout: None })
        .unwrap();
    assert_eq!(recovered.sandbox_id, format!("sb-{NAMESPACE}-1"));
    assert_eq!(recovered.envd_access_token.len(), 64);
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);
    port.kill("1", &recovered.sandbox_id).unwrap();
    drop(port);
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert_eq!(journal.world().unwrap().sandbox_records().len(), 1);
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(core::SandboxId(1))
            .unwrap()
            .phase,
        core::Phase::Destroyed
    );
}

#[test]
fn public_ipv4_requires_explicit_deployment_opt_in_and_ready_host() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut settings = config();
    settings.public_ipv4_enabled = true;
    let mut create = request();
    create.allow_internet_access = Some(true);
    let blocked = new_port(
        complete_fixture::open_with_config(&path, NAMESPACE, &settings).unwrap(),
        FakeHost::default(),
        settings.clone(),
    );
    assert_eq!(blocked.create("1", create.clone()), Err(PortError::Busy));
    assert!(
        blocked
            .list("1", ListQuery::default())
            .unwrap()
            .items
            .is_empty()
    );
    drop(blocked);
    let ready = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        FakeHost {
            public_ipv4_ready: true,
            ..FakeHost::default()
        },
        settings,
    );
    assert!(ready.create("1", create).is_ok());
    assert_eq!(
        ready.list("1", ListQuery::default()).unwrap().items.len(),
        1
    );
}

#[test]
fn two_active_sandboxes_page_without_duplicate_or_cross_tenant_leak() {
    let directory = TempDir::new().unwrap();
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let mut settings = config();
    settings.limits.max_active_per_tenant = 2;
    settings.limits.max_disk_bytes_per_tenant = 2_000;
    let port = new_port(journal, FakeHost::default(), settings);
    let first = port.create("1", request()).unwrap();
    let second = port.create("1", request()).unwrap();
    let page = port
        .list(
            "1",
            ListQuery {
                limit: Some(1),
                ..ListQuery::default()
            },
        )
        .unwrap();
    assert_eq!(page.items.len(), 1);
    let first_info = port.get("1", &first.sandbox_id).unwrap();
    let second_info = port.get("1", &second.sandbox_id).unwrap();
    let (newest, oldest) = if first_info.info.started_at == second_info.info.started_at {
        (&first.sandbox_id, &second.sandbox_id)
    } else {
        (&second.sandbox_id, &first.sandbox_id)
    };
    assert_eq!(&page.items[0].sandbox_id, newest);
    assert_eq!(page.total_running, Some(2));
    let following = port
        .list(
            "1",
            ListQuery {
                limit: Some(1),
                next_token: page.next_token,
                ..ListQuery::default()
            },
        )
        .unwrap();
    assert_eq!(following.items.len(), 1);
    assert_eq!(&following.items[0].sandbox_id, oldest);
    assert_eq!(following.total_running, Some(2));
    assert!(following.next_token.is_none());
    assert!(
        port.list("2", ListQuery::default())
            .unwrap()
            .items
            .is_empty()
    );
    port.kill("1", &first.sandbox_id).unwrap();
    port.kill("1", &second.sandbox_id).unwrap();
}

#[tokio::test]
async fn router_reaches_the_same_port_without_request_selected_identity() {
    let (_directory, port) = setup();
    let app = router(
        port,
        vec![
            ApiCredential {
                tenant: "1".into(),
                secret: KEY_ONE.into(),
            },
            ApiCredential {
                tenant: "2".into(),
                secret: KEY_TWO.into(),
            },
        ],
    )
    .unwrap();
    let create = Request::builder()
        .method("POST")
        .uri("/v2/sandboxes")
        .header("x-api-key", KEY_ONE)
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"templateID":"base","allow_internet_access":false}"#,
        ))
        .unwrap();
    let response = app.clone().oneshot(create).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    let sandbox = body["sandboxID"].as_str().unwrap();
    let foreign = Request::builder()
        .method("GET")
        .uri(format!("/sandboxes/{sandbox}"))
        .header("x-api-key", KEY_TWO)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(foreign).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    let kill = Request::builder()
        .method("DELETE")
        .uri(format!("/sandboxes/{sandbox}"))
        .header("x-api-key", KEY_ONE)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(kill).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
}

#[test]
fn filtered_listing_survives_restart_and_never_counts_or_returns_foreign_instances() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut settings = config();
    settings.limits.max_active_global = 4;
    settings.limits.max_active_per_tenant = 3;
    settings.limits.max_disk_bytes_global = complete_fixture::CLEANUP + 7_000;
    settings.limits.max_disk_bytes_per_tenant = 3_000;
    let port = new_port(
        complete_fixture::open_with_config(&path, NAMESPACE, &settings).unwrap(),
        FakeHost::default(),
        settings.clone(),
    );
    let mut red = request();
    red.metadata = Some(BTreeMap::from([("group".into(), "red".into())]));
    let first = port.create("1", red.clone()).unwrap();
    let second = port.create("1", red.clone()).unwrap();
    let foreign = port.create("2", red).unwrap();
    let mut blue = request();
    blue.metadata = Some(BTreeMap::from([("group".into(), "blue".into())]));
    let blue = port.create("1", blue).unwrap();
    drop(port);
    let port = new_port(
        complete_fixture::open_with_config(&path, NAMESPACE, &settings).unwrap(),
        FakeHost::default(),
        settings,
    );
    let mut query = ListQuery {
        metadata: Some("group=red".into()),
        state: Some("running,paused".into()),
        template: Some("base".into()),
        order: Some(ListOrder::Asc),
        limit: Some(1),
        ..ListQuery::default()
    };
    let page = port.list("1", query.clone()).unwrap();
    assert_eq!(page.total_running, Some(2));
    assert_eq!(page.items.len(), 1);
    query.next_token = page.next_token;
    assert!(query.next_token.is_some());
    let following = port.list("1", query.clone()).unwrap();
    assert_eq!(following.total_running, Some(2));
    assert!(following.next_token.is_none());
    let ids: BTreeSet<_> = page
        .items
        .iter()
        .chain(&following.items)
        .map(|item| item.sandbox_id.as_str())
        .collect();
    assert_eq!(
        ids,
        BTreeSet::from([first.sandbox_id.as_str(), second.sandbox_id.as_str()])
    );
    // Reusing a cursor under another tenant is only a position in that
    // tenant's authorized result set. Even its count cannot reveal the owner.
    let foreign_page = port.list("2", query.clone()).unwrap();
    assert_eq!(foreign_page.total_running, Some(1));
    assert!(
        foreign_page
            .items
            .iter()
            .all(|item| item.sandbox_id == foreign.sandbox_id)
    );
    query.state = Some("paused".into());
    let paused = port.list("1", query).unwrap();
    assert!(paused.items.is_empty());
    assert_eq!(paused.total_running, None);
    for session in [first, second, blue] {
        port.kill("1", &session.sandbox_id).unwrap();
    }
    port.kill("2", &foreign.sandbox_id).unwrap();
}

fn two_unknown_stops() -> (TempDir, ControllerJournal) {
    let (directory, port) = setup();
    let first = port.create("1", request()).unwrap();
    let second = port.create("2", request()).unwrap();
    assert_ne!(first.sandbox_id, second.sandbox_id);
    drop(port);
    let mut journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    for (sandbox, tenant) in [(1, 1), (2, 2)] {
        let result = kill_v1(
            &mut journal,
            core::KillRequest {
                actor: core::Actor::Tenant(core::TenantId(tenant)),
                sandbox: core::SandboxId(sandbox),
                operation: core::OperationId(2),
            },
            &mut |action| {
                if matches!(action.command.command(), core::Command::StopVm { .. }) {
                    return Ok(WorkReceipt::<Evidence> {
                        identity: action.identity.clone(),
                        outcome: WorkOutcome::Unknown {
                            reason: UnknownReason::ObservationUnavailable,
                            last_observation: None,
                        },
                    });
                }
                FakeHost::default().execute(action)
            },
        );
        assert!(matches!(
            result,
            Err(sandproof_controller_service::JournalError::UnresolvedAction)
        ));
    }
    assert_eq!(journal.uncertain_actions().unwrap().len(), 2);
    (directory, journal)
}

#[test]
fn failed_stop_reclaim_parks_only_its_sandbox_then_recovers_same_attempt() {
    let (directory, journal) = two_unknown_stops();
    let port = new_port(
        journal,
        FakeHost {
            stop: StopObservation::ObservedAfterRestart,
            reclaim_stop_error_for: Some(core::SandboxId(1)),
            ..FakeHost::default()
        },
        config(),
    );
    let first = port.application().recover_on_startup().unwrap();
    assert_eq!(first.receipts_recovered, 1);
    assert_eq!(first.workflows_completed, 1);
    assert_eq!(first.unresolved_sandboxes, 1);
    assert_eq!(first.failed_sandboxes, 1);
    let fault = first.first_fault.unwrap();
    assert_eq!(fault.scope, FaultScope::Instance);
    assert_eq!(fault.stage, FaultStage::Reclaim);
    assert_eq!(fault.kind, FaultKind::Application(PortError::Busy));
    assert_eq!(fault.context.sandbox, Some(1));
    assert!(fault.context.operation.is_some());
    assert_eq!(fault.context.attempt, Some(0));
    // The stopped but unconfirmed first VM still consumes its tenant slot;
    // the other tenant can reuse only its own reclaimed slot.
    assert_eq!(port.create("1", request()), Err(PortError::QuotaExceeded));
    port.create("2", request()).unwrap();
    drop(port);
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let world = journal.world().unwrap();
    assert_eq!(
        world.sandbox_record(core::SandboxId(1)).unwrap().phase,
        core::Phase::Revoking
    );
    assert_eq!(
        world.sandbox_record(core::SandboxId(2)).unwrap().phase,
        core::Phase::Destroyed
    );
    let unknown = journal.uncertain_actions().unwrap();
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].command.command().sandbox(), core::SandboxId(1));
    let port = new_port(
        journal,
        FakeHost {
            stop: StopObservation::ObservedAfterRestart,
            ..FakeHost::default()
        },
        config(),
    );
    let resumed = port.application().recover_on_startup().unwrap();
    assert_eq!(resumed.receipts_recovered, 1);
    assert_eq!(resumed.workflows_completed, 1);
    assert_eq!(resumed.unresolved_sandboxes, 0);
    assert_eq!(resumed.failed_sandboxes, 0);
    assert_eq!(resumed.first_fault, None);
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .workflows_completed,
        0
    );
    // Quota becomes reusable only after exact recovery finishes.
    port.create("1", request()).unwrap();
}

#[test]
fn invalid_receipt_is_rejected_without_delaying_the_other_sandbox() {
    let (directory, journal) = two_unknown_stops();
    let port = new_port(
        journal,
        FakeHost {
            stop: StopObservation::ObservedAfterRestart,
            bad_stop_observation_for: Some(core::SandboxId(1)),
            ..FakeHost::default()
        },
        config(),
    );
    let report = port.application().recover_on_startup().unwrap();
    assert_eq!(report.receipts_recovered, 1);
    assert_eq!(report.workflows_completed, 1);
    assert_eq!(report.failed_sandboxes, 1);
    let fault = report.first_fault.unwrap();
    assert_eq!(fault.scope, FaultScope::Instance);
    assert_eq!(fault.stage, FaultStage::RecoverReceipt);
    assert_eq!(fault.kind, FaultKind::MismatchedIdentity);
    assert_eq!(fault.context.sandbox, Some(1));
    assert_eq!(fault.context.attempt, Some(0));
    drop(port);
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert_eq!(journal.uncertain_actions().unwrap().len(), 1);
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(core::SandboxId(1))
            .unwrap()
            .phase,
        core::Phase::Revoking
    );
    let port = new_port(
        journal,
        FakeHost {
            stop: StopObservation::ObservedAfterRestart,
            ..FakeHost::default()
        },
        config(),
    );
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .unresolved_sandboxes,
        0
    );
}

#[test]
fn one_unknown_expiry_stop_does_not_starve_another_due_sandbox() {
    let (directory, port) = setup();
    let first = port.create("1", request()).unwrap();
    let second = port.create("2", request()).unwrap();
    port.set_timeout("1", &first.sandbox_id, 0).unwrap();
    port.set_timeout("2", &second.sandbox_id, 0).unwrap();
    drop(port);

    let port = new_port(
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap(),
        FakeHost {
            unknown_stop_for: Some(core::SandboxId(1)),
            ..FakeHost::default()
        },
        config(),
    );
    let report = port.application().expire_due_report().unwrap();
    assert_eq!(report.completed, 1);
    assert_eq!(report.unresolved_sandboxes, 1);
    drop(port);

    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let world = journal.world().unwrap();
    assert_eq!(
        world.sandbox_record(core::SandboxId(1)).unwrap().phase,
        core::Phase::Revoking
    );
    assert_eq!(
        world.sandbox_record(core::SandboxId(2)).unwrap().phase,
        core::Phase::Destroyed
    );
    assert_eq!(journal.uncertain_actions().unwrap().len(), 1);
    let port = new_port(
        journal,
        FakeHost {
            stop: StopObservation::ObservedAfterRestart,
            ..FakeHost::default()
        },
        config(),
    );
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .workflows_completed,
        1
    );
    assert_eq!(port.application().expire_due_report().unwrap().completed, 0);
}

#[test]
fn rejected_worker_receipt_parks_only_its_create() {
    let directory = TempDir::new().unwrap();
    let mut journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    for sandbox in [1, 2] {
        journal
            .admit_create(
                core::CreateRequest {
                    actor: core::Actor::Tenant(core::TenantId(sandbox)),
                    owner: core::TenantId(sandbox),
                    sandbox: core::SandboxId(sandbox),
                    workspace: core::FsId(sandbox),
                    file_policy: core::FilePolicyRef {
                        digest: core::PolicyDigest(1),
                    },
                    policy: core::NetworkPolicy::DenyAll,
                    operation: core::OperationId(1),
                },
                now,
                300,
                1_000,
                config().limits,
                &mut NeverCrash,
            )
            .unwrap();
    }
    let port = new_port(
        journal,
        FakeHost {
            bad_execute_for: Some(core::SandboxId(1)),
            ..FakeHost::default()
        },
        config(),
    );
    let report = port.application().recover_on_startup().unwrap();
    assert_eq!(report.workflows_completed, 1);
    assert_eq!(report.unresolved_sandboxes, 1);
    assert_eq!(report.failed_sandboxes, 1);
    drop(port);
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let world = journal.world().unwrap();
    assert_eq!(
        world.sandbox_record(core::SandboxId(1)).unwrap().phase,
        core::Phase::Starting
    );
    assert_eq!(
        world.sandbox_record(core::SandboxId(2)).unwrap().phase,
        core::Phase::Running
    );
    assert_eq!(journal.uncertain_actions().unwrap().len(), 1);
}

#[test]
fn corrupt_shared_journal_still_aborts_recovery() {
    let (directory, journal) = two_unknown_stops();
    let connection = rusqlite::Connection::open(directory.path().join("controller.db")).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE controller_checkpoint SET payload=X'00' WHERE id=1",
                [],
            )
            .unwrap(),
        1
    );
    let error = journal.world().unwrap_err();
    let fault = FaultRecord::journal(FaultStage::Recovery, &error, FaultContext::default());
    assert_eq!(fault.scope, FaultScope::SharedStorage);
    assert!(matches!(
        fault.kind,
        FaultKind::CorruptStore | FaultKind::JsonSyntax | FaultKind::JsonEof | FaultKind::JsonData
    ));
    let port = new_port(
        journal,
        FakeHost {
            stop: StopObservation::ObservedAfterRestart,
            ..FakeHost::default()
        },
        config(),
    );
    assert_eq!(
        port.application().recover_on_startup(),
        Err(PortError::Busy)
    );
}
