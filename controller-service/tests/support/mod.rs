use sandbox_controller_core as core;
use sandproof_controller_service::{
    AdmissionError, AdmissionLimits, ControllerJournal, CrashInjector, DispatchResult,
    HostEvidence, JournalError, NeverCrash, WorkerTransportError, finish_admitted_create_v1,
};
use sandproof_protocol::{ResourceEvidence, ResourceRef, WorkAction, WorkOutcome, WorkReceipt};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[allow(
    dead_code,
    reason = "shared fixture exposes limits for several independent test executables"
)]
pub mod complete_fixture;

pub use complete_fixture::CLEANUP_BYTES;

pub fn open_complete(
    path: impl AsRef<Path>,
    namespace: &str,
) -> Result<ControllerJournal, JournalError> {
    open_complete_with_slots(path, namespace, 64)
}

pub fn open_complete_with_slots(
    path: impl AsRef<Path>,
    namespace: &str,
    slots: u64,
) -> Result<ControllerJournal, JournalError> {
    let path = path.as_ref();
    ControllerJournal::open_complete(
        path,
        namespace,
        complete_fixture::host(path, namespace, slots, 2 * 1024 * 1024, CLEANUP_BYTES),
        complete_fixture::maintenance(),
        0,
    )
}

pub fn admission_limits() -> AdmissionLimits {
    AdmissionLimits {
        max_active_global: 64,
        max_active_per_tenant: 64,
        max_disk_bytes_global: CLEANUP_BYTES + 2 * 1024 * 1024,
        max_disk_bytes_per_tenant: 2 * 1024 * 1024,
        cleanup_reserve_bytes: CLEANUP_BYTES,
        max_timeout_secs: 300,
        max_lifetime_secs: 300,
        startup_timeout_secs: 300,
    }
}

pub fn admit(
    journal: &mut ControllerJournal,
    request: core::CreateRequest,
    crashes: &mut impl CrashInjector,
) -> Result<core::ControllerDecision, AdmissionError> {
    journal.admit_create(request, 0, 300, 1000, admission_limits(), crashes)
}

#[allow(
    dead_code,
    reason = "shared workflow helper is not used by journal-only test executables"
)]
pub fn admit_and_finish<W>(
    journal: &mut ControllerJournal,
    request: core::CreateRequest,
    worker: &mut W,
) -> Result<core::World, JournalError>
where
    W: FnMut(
        &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<FakeHostEvidence>, WorkerTransportError>,
{
    let sandbox = request.sandbox;
    let operation = request.operation;
    admit(journal, request, &mut NeverCrash).map_err(|error| match error {
        AdmissionError::Journal(error) => error,
        _ => JournalError::UnsupportedInput,
    })?;
    finish_admitted_create_v1(journal, sandbox, operation, worker)
}

pub const NAMESPACE: &str = "0123456789abcdef";
pub const SANDBOX: core::SandboxId = core::SandboxId(10);
pub const CREATE_OP: core::OperationId = core::OperationId(100);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FakeHostEvidence {
    pub resource: ResourceRef,
    pub command: core::Command,
    pub established: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<sandproof_controller_service::capture::SourceProcessIdentity>,
}

impl ResourceEvidence for FakeHostEvidence {
    fn resource(&self) -> &ResourceRef {
        &self.resource
    }
}

impl HostEvidence for FakeHostEvidence {
    fn validates(&self, command: &core::Command) -> bool {
        self.established && self.command == *command
    }

    fn source_process_identity(
        &self,
        command: &core::Command,
        resource: &ResourceRef,
    ) -> Option<sandproof_controller_service::capture::SourceProcessIdentity> {
        (self.established
            && self.command == *command
            && self.resource == *resource
            && matches!(
                command,
                core::Command::PrepareVm { .. } | core::Command::ActivateVm { .. }
            ))
        .then(|| self.source.clone())
        .flatten()
    }
}

pub fn success(action: &WorkAction<core::EffectId>) -> WorkReceipt<FakeHostEvidence> {
    WorkReceipt {
        identity: action.identity.clone(),
        outcome: WorkOutcome::ObservedSuccess(FakeHostEvidence {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
            established: true,
            source: None,
        }),
    }
}

// Keep the production transport signature for this deliberately infallible fake.
#[allow(clippy::unnecessary_wraps)]
pub fn worker(
    action: &WorkAction<core::EffectId>,
) -> Result<WorkReceipt<FakeHostEvidence>, WorkerTransportError> {
    Ok(success(action))
}

pub fn create() -> core::ControllerInput {
    core::ControllerInput::Request(core::Request::Create(core::CreateRequest {
        actor: core::Actor::Tenant(core::TenantId(1)),
        owner: core::TenantId(1),
        sandbox: SANDBOX,
        workspace: core::FsId(10),
        file_policy: core::FilePolicyRef {
            digest: core::PolicyDigest(1),
        },
        policy: core::NetworkPolicy::DenyAll,
        operation: CREATE_OP,
    }))
}

pub fn kill(owner: u64) -> core::ControllerInput {
    core::ControllerInput::Request(core::Request::Kill(core::KillRequest {
        actor: core::Actor::Tenant(core::TenantId(owner)),
        sandbox: SANDBOX,
        operation: core::OperationId(101),
    }))
}

pub fn apply(journal: &mut ControllerJournal, input: core::ControllerInput) -> core::World {
    // Test setup uses the same mandatory atomic admission as the formal service.
    // All non-create inputs still pass directly to the production journal.
    match input {
        core::ControllerInput::Request(core::Request::Create(request)) => {
            admit(journal, request, &mut NeverCrash).unwrap().next
        }
        input => journal.apply(input, &mut NeverCrash).unwrap().next,
    }
}

pub fn dispatch_all(journal: &mut ControllerJournal) -> usize {
    let mut count = 0;
    loop {
        match journal.dispatch_next(&mut worker, &mut NeverCrash).unwrap() {
            DispatchResult::NoReadyAction => return count,
            DispatchResult::ReceiptRecorded(_) => count += 1,
            DispatchResult::OutcomeUnknown { .. } => panic!("fake worker should return evidence"),
        }
        assert!(count < 20);
    }
}
