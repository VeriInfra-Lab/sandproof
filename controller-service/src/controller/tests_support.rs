//! Only complete component-test deployments. No production compatibility mode.
#[path = "../../tests/support/complete_fixture.rs"]
pub(super) mod fixture;
use super::*;
use serde::{Deserialize, Serialize};

/// Command-specific component evidence; never used as production observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct TestHostEvidence {
    resource: sandproof_protocol::ResourceRef,
    command: core::Command,
}
impl ResourceEvidence for TestHostEvidence {
    fn resource(&self) -> &sandproof_protocol::ResourceRef {
        &self.resource
    }
}
impl HostEvidence for TestHostEvidence {
    fn validates(&self, command: &core::Command) -> bool {
        &self.command == command
    }
}
#[expect(
    clippy::unnecessary_wraps,
    reason = "same worker signature as original workflow"
)]
pub(super) fn worker(
    action: &WorkAction<core::EffectId>,
) -> Result<WorkReceipt<TestHostEvidence>, WorkerTransportError> {
    Ok(WorkReceipt {
        identity: action.identity.clone(),
        outcome: WorkOutcome::ObservedSuccess(TestHostEvidence {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
        }),
    })
}

pub(super) fn open(
    path: impl AsRef<Path>,
    namespace: &str,
) -> Result<ControllerJournal, JournalError> {
    let path = path.as_ref();
    let fresh = !path.exists();
    let mut journal = ControllerJournal::open_complete(
        path,
        namespace,
        fixture::host(path, namespace, 64, 100_000, fixture::CLEANUP_BYTES),
        fixture::maintenance(),
        0,
    )?;
    if fresh {
        journal.publish_checkpoint(&mut crate::NeverCrash)?;
    }
    Ok(journal)
}

pub(super) fn host(slots: u64) -> BoundedHostConfig {
    // Synthetic trusted host paths are never activated by journal component tests.
    fixture::host(
        Path::new("/component-fixture/db"),
        "0123456789abcdef",
        slots,
        100_000,
        fixture::CLEANUP_BYTES,
    )
}

pub(super) fn limits() -> AdmissionLimits {
    AdmissionLimits {
        max_active_global: 64,
        max_active_per_tenant: 64,
        max_disk_bytes_global: fixture::CLEANUP_BYTES + 100_000,
        max_disk_bytes_per_tenant: 100_000,
        cleanup_reserve_bytes: fixture::CLEANUP_BYTES,
        max_timeout_secs: 600,
        max_lifetime_secs: 600,
        startup_timeout_secs: 60,
    }
}

pub(super) fn admit(
    journal: &mut ControllerJournal,
    input: core::ControllerInput,
    crashes: &mut impl CrashInjector,
) -> Result<core::ControllerDecision, JournalError> {
    let core::ControllerInput::Request(core::Request::Create(request)) = input else {
        return journal.apply(input, crashes);
    };
    journal
        .admit_create(request, 0, 60, 1000, limits(), crashes)
        .map_err(|error| match error {
            AdmissionError::Journal(error) => error,
            _ => JournalError::UnsupportedInput,
        })
}
