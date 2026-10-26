//! Identical assertions for original, candidate, wrong and restored Rust views.
use super::*;
use crate::controller::lifecycle_inbox::model::{Entry, proof_unload_entry};
use crate::controller::lifecycle_inbox::unload_resolution::UnloadResolution;
use sandproof_protocol::lifecycle::CurrentReadyFacts;

fn intent() -> AcceptedLifecycleIntent {
    let mut intent = create("owner", "session");
    intent.request.binding.expected = ExpectedSession::Version(0);
    intent.request.operation = LifecycleOperation::Unload {
        source_incarnation: 1.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
    };
    assert!(intent.validate(&context()).is_ok());
    intent
}

fn facts() -> CurrentReadyFacts {
    CurrentReadyFacts {
        started_at_ms: 0,
        end_at_ms: 10,
        max_end_at_ms: 20,
        startup_deadline_ms: 3,
        observed_at_ms: 1,
    }
}

fn snapshot() -> NativeCreateSnapshot {
    NativeCreateSnapshot {
        owner: core::TenantId::new(20),
        phase: core::Phase::Running,
        terminal: false,
        epoch: 1,
        generation: 1,
        current_vm: Some(core::VmId::new(40)),
        current_operation: None,
        lifetime: Some(crate::Lifetime {
            started_at_ms: Some(0),
            deadline_ms: 10,
            max_deadline_ms: 20,
            startup_deadline_ms: 3,
            disk_bytes: 0,
        }),
        uncertain_actions: vec![],
    }
}

pub(super) fn before() -> Entry {
    Entry {
        intent: intent(),
        local_operation: None,
        applied_deadline: None,
        previous_effective_end_at_ms: None,
        unload_resolution: Some(UnloadResolution::BeforeAdmission {
            original: Box::new(snapshot()),
            result: Some(facts()),
        }),
    }
}

fn after() -> Entry {
    Entry {
        local_operation: Some(core::OperationId::new(30)),
        unload_resolution: Some(UnloadResolution::AfterCapture { facts: facts() }),
        ..before()
    }
}

fn corrupt(entry: &Entry) {
    assert!(matches!(
        proof_unload_entry(entry),
        Err(JournalError::CorruptStore)
    ));
}

fn original(entry: &mut Entry) -> &mut NativeCreateSnapshot {
    let Some(UnloadResolution::BeforeAdmission { original, .. }) = entry.unload_resolution.as_mut()
    else {
        panic!("before-admission fixture required");
    };
    original
}

#[test]
fn normal_legal_shapes_accepted() {
    assert!(proof_unload_entry(&before()).is_ok());
    assert!(proof_unload_entry(&after()).is_ok());
    let mut entry = before();
    if let Some(UnloadResolution::BeforeAdmission { result, .. }) = entry.unload_resolution.as_mut()
    {
        *result = None;
    }
    assert!(proof_unload_entry(&entry).is_ok());
    entry.unload_resolution = None;
    assert!(proof_unload_entry(&entry).is_ok());
    let mut boundary = after();
    boundary.unload_resolution = Some(UnloadResolution::AfterCapture {
        facts: CurrentReadyFacts {
            started_at_ms: 1,
            observed_at_ms: 1,
            end_at_ms: 2,
            max_end_at_ms: 2,
            startup_deadline_ms: 1,
        },
    });
    assert!(proof_unload_entry(&boundary).is_ok());
}

#[test]
fn snapshot_requires_running() {
    let mut entry = before();
    original(&mut entry).phase = core::Phase::Paused;
    corrupt(&entry);
}

#[test]
fn capture_requires_operation() {
    let mut entry = after();
    entry.local_operation = None;
    corrupt(&entry);
}

#[test]
fn result_respects_maximum() {
    let mut entry = after();
    let mut result = facts();
    result.max_end_at_ms = 5;
    entry.unload_resolution = Some(UnloadResolution::AfterCapture { facts: result });
    corrupt(&entry);
}

#[test]
fn snapshot_fields_required() {
    let mut entry = before();
    original(&mut entry).terminal = true;
    corrupt(&entry);
    let mut entry = before();
    original(&mut entry).current_vm = None;
    corrupt(&entry);
    let mut entry = before();
    original(&mut entry).lifetime = None;
    corrupt(&entry);
    let mut entry = before();
    original(&mut entry)
        .uncertain_actions
        .push(core::EffectId::initial(core::Command::CleanupVm {
            sandbox: core::SandboxId::new(10),
            generation: 1,
            vm: core::VmId::new(40),
        }));
    corrupt(&entry);
}

#[test]
fn result_time_chain_required() {
    let mut result = facts();
    result.started_at_ms = 2;
    let mut expired = facts();
    expired.observed_at_ms = expired.end_at_ms;
    let mut late_start = facts();
    late_start.started_at_ms = 1;
    late_start.startup_deadline_ms = 0;
    for facts in [result, expired, late_start] {
        let mut entry = after();
        entry.unload_resolution = Some(UnloadResolution::AfterCapture { facts });
        corrupt(&entry);
    }
}

#[test]
fn resolution_requires_unload() {
    let mut entry = after();
    entry.intent.request.operation = LifecycleOperation::Destroy {};
    corrupt(&entry);
    entry.unload_resolution = None;
    assert!(proof_unload_entry(&entry).is_ok());
}

#[test]
fn before_admission_forbids_operation() {
    let mut entry = before();
    entry.local_operation = Some(core::OperationId::new(30));
    corrupt(&entry);
}

#[test]
fn unchecked_fields_are_not_extra_conditions() {
    let mut entry = before();
    let original = original(&mut entry);
    original.owner = core::TenantId::new(999);
    original.epoch = 0;
    original.generation = 0;
    original.lifetime.as_mut().unwrap().deadline_ms = -1;
    entry.applied_deadline = Some(-1);
    entry.previous_effective_end_at_ms = Some(-2);
    assert!(proof_unload_entry(&entry).is_ok());
}
