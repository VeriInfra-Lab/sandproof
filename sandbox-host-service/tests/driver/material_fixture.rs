//! Prepare original durable Full/Destroy state through public application and
//! journal transitions. All physical facts here are controlled test inputs.
use super::*;
use sandbox_controller_core::{
    self as core,
    workflow::{self, WorkflowStep},
};
use sandproof_controller_service::{
    ControllerJournal, DispatchResult, HostEvidence, NeverCrash, SourceWorkRecord,
    WorkerTransportError,
    application::WorkEvidence,
    capture::{
        CaptureRequestEndObservation, CaptureRequestEndOutcome, CaptureWorkerBinding,
        FullCaptureWorkRequest, HostEpochObservation, MaterialRetirementObservation,
        RawSourceObservation,
    },
};
use sandproof_protocol::{
    ResourceEvidence, ResourceRef, WorkAction, WorkOutcome, WorkReceipt,
    lifecycle::{
        ContentDigest, ExpectedSession, LifecycleOperation, VmCompatibility, WritableDrive,
    },
};
use serde::{Deserialize, Serialize};

pub(super) const PLAN: &[u8] = b"original bounded material plan";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CaptureEvidence {
    resource: ResourceRef,
    command: core::Command,
    established: bool,
    source: Option<SourceProcessIdentity>,
}
impl ResourceEvidence for CaptureEvidence {
    fn resource(&self) -> &ResourceRef {
        &self.resource
    }
}
impl HostEvidence for CaptureEvidence {
    fn validates(&self, command: &core::Command) -> bool {
        self.established && self.command == *command
    }
    fn capture_manifest_digest(&self, command: &core::Command) -> Option<ContentDigest> {
        (self.validates(command)
            && matches!(
                command,
                core::Command::SealSnapshot { .. } | core::Command::CommitSnapshot { .. }
            ))
        .then_some(ContentDigest::from_sha256([88; 32]))
    }
    fn capture_material_bytes(&self, command: &core::Command) -> Option<u64> {
        self.capture_manifest_digest(command).map(|_| 12288)
    }
}

fn bootstrap(journal: &mut ControllerJournal) {
    journal
        .establish_host_epoch(
            &mut |r| {
                Ok(HostEpochObservation {
                    node: r.node().clone(),
                    registration: r.registration().clone(),
                })
            },
            &mut NeverCrash,
        )
        .unwrap();
}

fn claim(journal: &mut ControllerJournal, sandbox: core::SandboxId) -> WorkAction<core::EffectId> {
    let mut claimed = None;
    // The public dispatcher persists the original claim before invoking this
    // controlled provider. Complete that same action below, never a raw claim.
    let result: DispatchResult<CaptureEvidence> = journal
        .dispatch_next_for(
            sandbox,
            &mut |action| {
                claimed = Some(action.clone());
                Err(WorkerTransportError::ReplyLost)
            },
            &mut NeverCrash,
        )
        .unwrap();
    assert!(matches!(result, DispatchResult::OutcomeUnknown { .. }));
    claimed.unwrap()
}

fn finish(
    journal: &mut ControllerJournal,
    sandbox: core::SandboxId,
    source: Option<&SourceWorkRecord>,
) {
    for _ in 0..64 {
        match workflow::next_step(&journal.world().unwrap(), sandbox) {
            WorkflowStep::Apply(input) => {
                journal.apply(*input, &mut NeverCrash).unwrap();
            }
            WorkflowStep::Claim => {
                let action = claim(journal, sandbox);
                let seal = matches!(action.command.command(), core::Command::SealSnapshot { .. });
                if seal {
                    let source = source.unwrap();
                    let worker = CaptureWorkerBinding::new(
                        SourceProcessIdentity {
                            pid: 2345,
                            start_time_ticks: 9876,
                            boot_id: "fixture-boot".into(),
                        },
                        ContentDigest::from_sha256([9; 32]),
                    )
                    .unwrap();
                    journal
                        .authorize_lifecycle_full_capture(
                            &source.work,
                            &current::context(),
                            &action,
                            worker,
                            10,
                            &mut NeverCrash,
                        )
                        .unwrap();
                }
                journal
                    .complete(
                        &WorkReceipt {
                            identity: action.identity.clone(),
                            outcome: WorkOutcome::ObservedSuccess(CaptureEvidence {
                                resource: action.identity.resource.clone(),
                                command: action.command.command().clone(),
                                established: true,
                                source: None,
                            }),
                        },
                        &mut NeverCrash,
                    )
                    .unwrap();
                if seal {
                    let saved = journal.lifecycle_capture_record(sandbox).unwrap().unwrap();
                    journal
                        .record_lifecycle_capture_request_end(
                            &saved.work,
                            &current::context(),
                            CaptureRequestEndObservation {
                                action: saved.capture_action().unwrap().clone(),
                                full_work_binding: saved
                                    .full_capture()
                                    .unwrap()
                                    .binding_digest()
                                    .unwrap(),
                                worker: saved.capture_worker().unwrap().clone(),
                                source: saved.work.requirement().source.clone(),
                                outcome: CaptureRequestEndOutcome::Returned { http_status: 204 },
                            },
                            &mut NeverCrash,
                        )
                        .unwrap();
                }
            }
            WorkflowStep::Complete => return,
            other => panic!("original capture/terminal workflow blocked: {other:?}"),
        }
    }
    panic!("original finite capture/terminal workflow did not complete");
}

fn full(source: &SourceWorkRecord) -> FullCaptureWorkRequest {
    FullCaptureWorkRequest {
        pause: source.work.pause_request().unwrap(),
        compatibility: VmCompatibility {
            snapshot_format: 1,
            tool: ContentDigest::from_sha256([1; 32]),
            cpu: ContentDigest::from_sha256([2; 32]),
            devices: ContentDigest::from_sha256([3; 32]),
            image_configuration: ContentDigest::from_sha256([4; 32]),
            writable_drives: vec![WritableDrive {
                drive_id: "rootfs".try_into().unwrap(),
                jail_relative_path: "rootfs.ext4".into(),
            }],
        },
        state_bytes: 4096,
        memory_bytes: 4096,
        disk_bytes: 4096,
        api_timeout_ms: 1000,
        worker_timeout_ms: 1000,
    }
}

pub(super) fn terminal(
    path: &Path,
    state: &Arc<Mutex<local::State>>,
) -> (AcceptedLifecycleIntent, MaterialRetirementObservation) {
    let app = current::application(path, state.clone(), true);
    app.establish_lifecycle_host_epoch().unwrap();
    let mut unload = current::request();
    app.record_lifecycle_intent(&unload, &current::context())
        .unwrap();
    current::advance(&app).unwrap();
    unload.accepted_version = 2;
    unload.request.binding.expected = ExpectedSession::Version(1);
    unload.request.binding.key.sequence = 2.try_into().unwrap();
    unload.request.operation = LifecycleOperation::Unload {
        source_incarnation: 101.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
    };
    app.record_lifecycle_intent(&unload, &current::context())
        .unwrap();
    let question = app
        .prepare_lifecycle_source_work(&unload, &current::context())
        .unwrap();
    state.lock().unwrap().source_probe = Some(Box::new(|r| {
        Ok(RawSourceObservation {
            work_binding: r.work_binding,
            resource: r.resource.clone(),
            source: r.source.clone(),
            configuration: br#"{"vcpu_count":1}"#.to_vec(),
            executable_sha256: [11; 32],
            kernel_sha256: [12; 32],
            kernel_bytes: 100,
            disk_bytes: 4096,
        })
    }));
    let source = app
        .observe_lifecycle_source_work(&question.work, &current::context())
        .unwrap();
    drop(app);
    let mut journal = current::open(path);
    bootstrap(&mut journal);
    let source = journal
        .admit_lifecycle_full_capture::<<local::TestHost as WorkEvidence>::Evidence>(
            &source.work,
            &current::context(),
            &full(&source),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    let sandbox = journal
        .lifecycle_work_state(&unload, &current::context())
        .unwrap()
        .local
        .unwrap()
        .sandbox;
    finish(&mut journal, sandbox, Some(&source));
    let context = journal.host_slot_context(sandbox).unwrap().unwrap();
    journal
        .release_host_slot(context, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    let mut destroy = unload;
    destroy.accepted_version = 3;
    destroy.request.binding.expected = ExpectedSession::Version(2);
    destroy.request.binding.key.sequence = 3.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    journal
        .record_lifecycle_intent(&destroy, &current::context(), &mut NeverCrash)
        .unwrap();
    finish(&mut journal, sandbox, None);
    assert!(
        !journal
            .lifecycle_destroy_cleaned(&destroy, &current::context())
            .unwrap()
    );
    let request = journal
        .lifecycle_material_retirement_request(&source.work, &current::context())
        .unwrap();
    let reply = MaterialRetirementObservation {
        work_binding: request.binding_digest().unwrap(),
        // SHA-256 of PLAN; the original journal independently checks these bytes.
        plan_binding: ContentDigest::from_sha256([
            240, 125, 42, 246, 224, 125, 121, 35, 215, 91, 173, 195, 51, 162, 165, 242, 130, 43,
            37, 7, 152, 192, 104, 16, 242, 75, 152, 96, 49, 213, 118, 23,
        ]),
    };
    (destroy, reply)
}
