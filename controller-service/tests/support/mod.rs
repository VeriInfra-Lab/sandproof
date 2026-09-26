use sandbox_controller_core as core;
use sandproof_controller_service::{
    ControllerJournal, DispatchResult, HostEvidence, NeverCrash, WorkerTransportError,
};
use sandproof_protocol::{ResourceEvidence, ResourceRef, WorkAction, WorkOutcome, WorkReceipt};
use serde::{Deserialize, Serialize};

pub const NAMESPACE: &str = "0123456789abcdef";
pub const SANDBOX: core::SandboxId = core::SandboxId(10);
pub const CREATE_OP: core::OperationId = core::OperationId(100);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FakeHostEvidence {
    pub resource: ResourceRef,
    pub command: core::Command,
    pub established: bool,
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
}

pub fn success(action: &WorkAction<core::EffectId>) -> WorkReceipt<FakeHostEvidence> {
    WorkReceipt {
        identity: action.identity.clone(),
        outcome: WorkOutcome::ObservedSuccess(FakeHostEvidence {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
            established: true,
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
    journal.apply(input, &mut NeverCrash).unwrap().next
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
