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
    ApiCredential, ConnectRequest, ControllerPort, CreateRequest, ListOrder, ListQuery, PortError,
    managed::{HostAdapter, ManagedConfig, ManagedPort},
    router,
};
use sandproof_controller_service::{
    AdmissionLimits, ControllerJournal, CreateAdmission, HostEvidence, NeverCrash,
    WorkerTransportError, finish_admitted_create_v1, kill_v1,
};
use sandproof_protocol::{
    FailureCode, ResourceEvidence, ResourceRef, UnknownReason, WorkAction, WorkOutcome, WorkReceipt,
};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use tower::ServiceExt;

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
    secrets: BTreeMap<u64, (String, BTreeMap<String, String>)>,
    launch_error_once: Option<PortError>,
    token_error_once: AtomicBool,
    free_bytes: u64,
    public_ipv4_ready: bool,
    incomplete_open_fence: bool,
    prepare: PrepareObservation,
    recovered_actions: BTreeSet<RecoveredAction>,
    stop: StopObservation,
    incomplete_cleanup: bool,
    stop_calls: Option<Arc<AtomicUsize>>,
    prepare_calls: Option<Arc<AtomicUsize>>,
    stop_gate: Option<Arc<ActionGate>>,
    prepare_gate: Option<Arc<ActionGate>>,
}

impl Default for FakeHost {
    fn default() -> Self {
        Self {
            secrets: BTreeMap::new(),
            launch_error_once: None,
            token_error_once: AtomicBool::new(false),
            free_bytes: 10_000,
            public_ipv4_ready: false,
            incomplete_open_fence: false,
            prepare: PrepareObservation::Missing,
            recovered_actions: BTreeSet::new(),
            stop: StopObservation::Normal,
            incomplete_cleanup: false,
            stop_calls: None,
            prepare_calls: None,
            stop_gate: None,
            prepare_gate: None,
        }
    }
}

impl HostAdapter for FakeHost {
    type Evidence = Evidence;

    fn public_ipv4_ready(&self) -> Result<bool, PortError> {
        Ok(self.public_ipv4_ready)
    }

    fn available_disk_bytes(&self) -> Result<u64, PortError> {
        Ok(self.free_bytes)
    }

    fn put_secret(&mut self, sandbox: core::SandboxId, token: &str) -> Result<(), PortError> {
        self.put_launch_config(sandbox, token, &BTreeMap::new())
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

    fn has_secret(&self, sandbox: core::SandboxId) -> Result<bool, PortError> {
        Ok(self.secrets.contains_key(&sandbox.0))
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

    fn token_for(&self, sandbox: core::SandboxId) -> Result<String, PortError> {
        if self.token_error_once.swap(false, Ordering::SeqCst) {
            return Err(PortError::Busy);
        }
        self.secrets
            .get(&sandbox.0)
            .map(|(token, _)| token.clone())
            .ok_or(PortError::Busy)
    }

    fn execute(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<Evidence>, WorkerTransportError> {
        if self.incomplete_open_fence
            && matches!(
                action.command.command(),
                core::Command::InstallFence {
                    terminal: false,
                    ..
                }
            )
        {
            return Ok(WorkReceipt {
                identity: action.identity.clone(),
                outcome: WorkOutcome::Unknown {
                    reason: UnknownReason::ObservationUnavailable,
                    last_observation: None,
                },
            });
        }
        if matches!(action.command.command(), core::Command::PrepareVm { .. }) {
            if let Some(calls) = &self.prepare_calls {
                calls.fetch_add(1, Ordering::SeqCst);
            }
            if let Some(gate) = &self.prepare_gate {
                gate.block();
            }
        }
        if matches!(action.command.command(), core::Command::StopVm { .. }) {
            if let Some(calls) = &self.stop_calls {
                calls.fetch_add(1, Ordering::SeqCst);
            }
            if let Some(gate) = &self.stop_gate {
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
            return Ok(WorkReceipt {
                identity: action.identity.clone(),
                outcome: WorkOutcome::Unknown {
                    reason: UnknownReason::ObservationUnavailable,
                    last_observation: None,
                },
            });
        }
        Ok(WorkReceipt {
            identity: action.identity.clone(),
            outcome: WorkOutcome::ObservedSuccess(Evidence {
                resource: action.identity.resource.clone(),
                command: action.command.command().clone(),
            }),
        })
    }

    fn observe(&self, action: &WorkAction<core::EffectId>) -> Option<WorkReceipt<Evidence>> {
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
        if self.stop == StopObservation::Incomplete {
            self.stop = StopObservation::ObservedAfterRestart;
        }
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

fn config() -> ManagedConfig {
    ManagedConfig {
        public_ipv4_enabled: false,
        limits: AdmissionLimits {
            max_active_global: 2,
            max_active_per_tenant: 1,
            max_disk_bytes_global: 4_000,
            max_disk_bytes_per_tenant: 1_000,
            cleanup_reserve_bytes: 1_000,
            max_timeout_secs: 86_400,
        },
        disk_bytes: 1_000,
        file_policy_digest: core::PolicyDigest(1),
        default_timeout_secs: 300,
        cpu_count: 1,
        memory_mb: 512,
        disk_size_mb: 1_024,
        envd_version: "0.9.0".into(),
        domain: "example.invalid".into(),
    }
}

fn setup_with_host(host: FakeHost) -> (TempDir, Arc<ManagedPort<FakeHost>>) {
    let directory = TempDir::new().unwrap();
    let journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    (
        directory,
        Arc::new(ManagedPort::new(journal, host, config())),
    )
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

#[test]
fn opening_fence_interrupted_before_host_write_is_completed_only_for_current_create() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
    admit_public_create(&mut journal);
    let sandbox = core::SandboxId(1);
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
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        host,
        settings,
    );
    let report = port.recover_on_startup().unwrap();
    assert_eq!(report.receipts_recovered, 1);
    assert_eq!(report.workflows_completed, 1);
    assert_eq!(report.unresolved_sandboxes, 0);
    assert_eq!(
        port.get("1", &format!("sb-{NAMESPACE}-1")).unwrap().state,
        "running"
    );
    assert_eq!(port.recover_on_startup().unwrap().receipts_recovered, 0);
}

#[test]
fn lost_public_network_receipt_is_reconciled_before_running() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
    admit_public_create(&mut journal);
    let sandbox = core::SandboxId(1);
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
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        host,
        settings,
    );
    let report = port.recover_on_startup().unwrap();
    assert_eq!(report.receipts_recovered, 1);
    assert_eq!(report.workflows_completed, 1);
    assert_eq!(report.unresolved_sandboxes, 0);
    assert_eq!(
        port.get("1", &format!("sb-{NAMESPACE}-1")).unwrap().state,
        "running"
    );
    assert_eq!(port.recover_on_startup().unwrap().receipts_recovered, 0);
}

#[test]
fn lost_terminal_fence_receipt_is_reapplied_before_failed_publication() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
    admit_public_create(&mut journal);
    let sandbox = core::SandboxId(1);
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
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        host,
        config(),
    );
    let report = port.recover_on_startup().unwrap();
    assert_eq!(report.receipts_recovered, 1);
    assert_eq!(report.workflows_completed, 1);
    assert_eq!(report.unresolved_sandboxes, 0);
    assert_eq!(
        port.get("1", &format!("sb-{NAMESPACE}-1")),
        Err(PortError::NotFound)
    );
    assert_eq!(port.recover_on_startup().unwrap().receipts_recovered, 0);
}

#[test]
fn startup_recovers_admitted_create_before_credential_write() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
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
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    let recovery = port.recover_on_startup().unwrap();
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
    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
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
    journal
        .apply(
            core::ControllerInput::Scheduler(core::SchedulerInput::ReserveStartup(
                core::OperationKey {
                    sandbox,
                    operation: core::OperationId(1),
                },
            )),
            &mut NeverCrash,
        )
        .unwrap();
    drop(journal);
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    let recovery = port.recover_on_startup().unwrap();
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
    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
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
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        host,
        config(),
    );
    let recovery = port.recover_on_startup().unwrap();
    assert_eq!(recovery.receipts_recovered, 1);
    assert_eq!(recovery.workflows_completed, 1);
    assert_eq!(recovery.unresolved_sandboxes, 0);
    assert_eq!(
        port.get("1", &format!("sb-{NAMESPACE}-1")).unwrap().state,
        "running"
    );
    let again = port.recover_on_startup().unwrap();
    assert_eq!(again.receipts_recovered, 0);
    assert_eq!(again.workflows_completed, 0);
    assert_eq!(again.unresolved_sandboxes, 0);
}

#[test]
fn periodic_recovery_reclaims_incomplete_prepare_without_replaying_launch() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
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
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        host,
        config(),
    );
    let recovered = port.recover_on_startup().unwrap();
    assert_eq!(recovered.receipts_recovered, 1);
    assert_eq!(recovered.unresolved_sandboxes, 0);
    assert_eq!(port.recover_on_startup().unwrap().receipts_recovered, 0);
    drop(port);
    let journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
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
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    let sandbox_id = port.create("1", request()).unwrap().sandbox_id;
    drop(port);

    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
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

    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        FakeHost {
            stop: StopObservation::ObservedAfterRestart,
            ..FakeHost::default()
        },
        config(),
    );
    let recovery = port.recover_on_startup().unwrap();
    assert_eq!(recovery.receipts_recovered, 1);
    assert_eq!(recovery.workflows_completed, 1);
    assert_eq!(recovery.unresolved_sandboxes, 0);
    assert_eq!(port.get("1", &sandbox_id), Err(PortError::NotFound));
    assert_eq!(port.recover_on_startup().unwrap().receipts_recovered, 0);
    drop(port);

    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
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
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    let sandbox_id = port.create("1", request()).unwrap().sandbox_id;
    drop(port);

    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
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

    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        FakeHost {
            stop: StopObservation::Incomplete,
            ..FakeHost::default()
        },
        config(),
    );
    let report = port.recover_on_startup().unwrap();
    assert_eq!(report.receipts_recovered, 1);
    assert_eq!(report.workflows_completed, 1);
    assert_eq!(report.unresolved_sandboxes, 0);
    assert_eq!(port.get("1", &sandbox_id), Err(PortError::NotFound));
    assert_eq!(port.recover_on_startup().unwrap().receipts_recovered, 0);
    drop(port);

    let journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
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
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    let sandbox_id = port.create("1", request()).unwrap().sandbox_id;
    drop(port);

    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
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

    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    let recovered = port.recover_on_startup().unwrap();
    assert_eq!(recovered.receipts_recovered, 1);
    assert_eq!(recovered.workflows_completed, 1);
    assert_eq!(recovered.unresolved_sandboxes, 0);
    assert_eq!(port.get("1", &sandbox_id), Err(PortError::NotFound));
    let idle = port.recover_on_startup().unwrap();
    assert_eq!(idle.receipts_recovered, 0);
    assert_eq!(idle.workflows_completed, 0);
    assert_eq!(idle.unresolved_sandboxes, 0);
    drop(port);

    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
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
    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
    let sandbox = core::SandboxId(1);
    let result = sandproof_controller_service::create_v1(
        &mut journal,
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
        &mut |action| match action.command.command() {
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
        },
    );
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
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        host,
        config(),
    );
    let recovered = port.recover_on_startup().unwrap();
    assert_eq!(recovered.receipts_recovered, 1);
    assert_eq!(recovered.unresolved_sandboxes, 0);
    let idle = port.recover_on_startup().unwrap();
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
        let report = port.recover_on_startup().unwrap();
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
            port.authorize_data(
                &live.envd_access_token,
                sandproof_api::managed::DataKind::Exec
            )
            .is_ok()
        );
        for session in &retired {
            assert_eq!(
                port.authorize_data(
                    &session.envd_access_token,
                    sandproof_api::managed::DataKind::Exec
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
        sandproof_api::managed::DataKind::Exec,
        sandproof_api::managed::DataKind::FileRead,
        sandproof_api::managed::DataKind::FileWrite,
    ] {
        assert_eq!(
            port.authorize_data(&created.envd_access_token, kind)
                .unwrap()
                .generation,
            1
        );
    }
    assert_eq!(
        port.authorize_data(&"0".repeat(64), sandproof_api::managed::DataKind::Exec),
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
    assert_eq!(info.state, "running");
    assert!(info.started_at.ends_with('Z'));
    assert!(info.end_at.ends_with('Z'));
    assert_eq!(
        port.list("1", ListQuery::default()).unwrap().items,
        vec![info]
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
        port.authorize_data(
            &created.envd_access_token,
            sandproof_api::managed::DataKind::Exec
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
            .metadata
            .get("job"),
        Some(&"job-7".to_owned())
    );
    drop(port);
    let journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let options = journal.create_options(core::SandboxId(1)).unwrap();
    assert_eq!(options.env_vars.get("TASK_ID"), Some(&"job-7".to_owned()));
    assert_eq!(options.metadata.get("job"), Some(&"job-7".to_owned()));
}

#[test]
fn restart_uses_admitted_environment_and_rejects_a_mismatched_host_record() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
    let options = sandproof_controller_service::CreateOptions {
        env_vars: BTreeMap::from([("TASK_ID".into(), "persisted".into())]),
        metadata: BTreeMap::from([("job".into(), "persisted".into())]),
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
    let wrong = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        wrong_host,
        config(),
    );
    assert_eq!(wrong.recover_on_startup().unwrap().unresolved_sandboxes, 1);
    assert_eq!(
        wrong.get("1", &format!("sb-{NAMESPACE}-1")),
        Err(PortError::NotFound)
    );
    drop(wrong);

    let good = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    assert_eq!(good.recover_on_startup().unwrap().workflows_completed, 1);
    assert_eq!(
        good.get("1", &format!("sb-{NAMESPACE}-1"))
            .unwrap()
            .metadata,
        options.metadata
    );
    assert_eq!(good.recover_on_startup().unwrap().unresolved_sandboxes, 0);
}

#[test]
fn data_revocation_is_scoped_and_cannot_be_undone_by_a_new_instance() {
    use sandproof_api::managed::DataKind;
    let (_directory, port) = setup();
    let first = port.create("1", request()).unwrap();
    let second = port.create("2", request()).unwrap();
    let first_routes = [DataKind::Exec, DataKind::FileRead, DataKind::FileWrite]
        .map(|kind| port.data_permit(&first.envd_access_token, kind).unwrap());
    let other_route = port
        .data_permit(&second.envd_access_token, DataKind::Exec)
        .unwrap();
    assert!(port.data_permit(&"0".repeat(64), DataKind::Exec).is_err());
    assert_eq!(port.kill("2", &first.sandbox_id), Err(PortError::NotFound));
    port.set_timeout("1", &first.sandbox_id, 60).unwrap();
    assert_eq!(port.expire_due().unwrap(), 0);
    assert!(first_routes.iter().all(|route| !*route.revocation.borrow()));
    port.kill("1", &first.sandbox_id).unwrap();
    assert!(first_routes.iter().all(|route| *route.revocation.borrow()));
    assert!(!*other_route.revocation.borrow());
    assert!(
        port.data_permit(&first.envd_access_token, DataKind::Exec)
            .is_err()
    );
    let replacement = port.create("1", request()).unwrap();
    let new_route = port
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
fn accepted_kill_revokes_routes_even_when_stop_receipt_is_unknown() {
    use sandproof_api::managed::DataKind;
    let (_directory, port) = setup_with_host(FakeHost {
        stop: StopObservation::LoseReceipt,
        ..FakeHost::default()
    });
    let sandbox = port.create("1", request()).unwrap();
    let route = port
        .data_permit(&sandbox.envd_access_token, DataKind::Exec)
        .unwrap();
    assert_eq!(
        port.kill("1", &sandbox.sandbox_id),
        Err(PortError::OutcomeUnknown)
    );
    assert!(*route.revocation.borrow());
    assert!(
        port.data_permit(&sandbox.envd_access_token, DataKind::Exec)
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
        .data_permit(
            &created.envd_access_token,
            sandproof_api::managed::DataKind::Exec,
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
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
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
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
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
        port.authorize_data(
            &session.envd_access_token,
            sandproof_api::managed::DataKind::Exec
        ),
        Err(PortError::NotFound)
    );
    drop(port);
    let journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
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
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
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
    let port = ManagedPort::new(
        journal,
        FakeHost {
            prepare_calls: Some(Arc::clone(&prepare_calls)),
            ..FakeHost::default()
        },
        config(),
    );
    let recovery = port.recover_on_startup().unwrap();
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
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
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
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
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
    let blocked = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
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
    let ready = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
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
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let mut settings = config();
    settings.limits.max_active_per_tenant = 2;
    settings.limits.max_disk_bytes_per_tenant = 2_000;
    let port = ManagedPort::new(journal, FakeHost::default(), settings);
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
    let (newest, oldest) = if first_info.started_at == second_info.started_at {
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
    settings.limits.max_disk_bytes_global = 8_000;
    settings.limits.max_disk_bytes_per_tenant = 3_000;
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
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
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
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
