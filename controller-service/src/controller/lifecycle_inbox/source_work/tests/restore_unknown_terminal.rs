//! Original SQLite/workflow with controlled physical port outcomes, not KVM.
//! Unknown stays held. Only the second phase supplies the controlled physical
//! port's exact original Prepare observation after the fault has cleared.
use super::*;
use sandbox_controller_core as core;
use core::workflow::{self, WorkflowStep};
use sandproof_protocol::{UnknownReason, WorkOutcome, WorkReceipt};

fn unknown_prepare_then_terminal(expiry: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, source) = unloaded_resume_source(&path);
    let sandbox = source.work.anchor.local.sandbox;
    let resume = record_real_resume(&mut journal);
    admit_real_resume(&mut journal, &resume);
    let prepare = claim_restore_action(&mut journal, sandbox, |command| matches!(
        command, core::Command::PrepareVm { source: Some(_), .. }
    ));
    journal.complete::<FullEvidence>(&WorkReceipt {
        identity: prepare.identity.clone(),
        outcome: WorkOutcome::Unknown { reason: UnknownReason::ObservationUnavailable, last_observation: None },
    }, &mut NeverCrash).unwrap();
    let held = journal.lifecycle_source_work(&v3::config().node).unwrap();
    assert!(held[0].restore.as_ref().unwrap().material_use);
    if expiry {
        let lifetime = resume_lifetime(&journal, sandbox);
        assert_eq!(journal.abort_due_startups(lifetime.deadline_ms, &mut NeverCrash).unwrap(), 1);
    } else {
        let mut destroy = resume.clone();
        destroy.accepted_version += 1;
        destroy.request.binding.expected = ExpectedSession::Version(resume.accepted_version);
        destroy.request.binding.key.sequence = (resume.request.binding.key.sequence.get()+1).try_into().unwrap();
        destroy.request.operation = LifecycleOperation::Destroy {};
        journal.record_lifecycle_intent(&destroy, &v3::context(), &mut NeverCrash).unwrap();
    }
    drop(journal);
    let mut journal = v3::open(&path);
    assert_eq!(journal.lifecycle_source_work(&v3::config().node).unwrap(), held,
        "terminal admission/reopen cannot drop original loader/File backing obligation");
    let observed = journal.reconcile_observed(&prepare, &WorkReceipt {
        identity: prepare.identity.clone(),
        outcome: WorkOutcome::ObservedSuccess(FullEvidence {
            resource: prepare.identity.resource.clone(), command: prepare.command.command().clone(),
        }),
    });
    assert!(matches!(&observed, Ok(true)),
        "after fault clearance the exact original readonly Prepare observation must reconcile while keeping terminal authority: {observed:?}");
    let mut stopped = false;
    let mut completed = false;
    for _ in 0..64 {
        match workflow::next_step(&journal.world().unwrap(), sandbox) {
            WorkflowStep::Apply(input) => {
                let diagnostic = format!("{input:?}");
                if let Err(error) = journal.apply(*input, &mut NeverCrash) {
                    let world = journal.world().unwrap();
                    panic!("expiry={expiry} input={diagnostic} error={error:?} sandboxes={:?} plans={:?} vms={:?} slots={:?}", world.sandboxes, world.operation_plans, world.vms, world.net_slots);
                }
            }
            WorkflowStep::Claim => {
                let action=journal.claim_next(Some(sandbox), &mut NeverCrash).unwrap().unwrap();
                assert!(!matches!(action.command.command(),core::Command::PrepareVm { .. } | core::Command::ActivateVm { .. }),
                    "terminal recovery must never issue a second load or late activation");
                stopped |= matches!(action.command.command(),core::Command::StopVm { .. });
                journal.complete(&WorkReceipt {
                    identity:action.identity.clone(),
                    outcome:WorkOutcome::ObservedSuccess(FullEvidence {
                        resource:action.identity.resource.clone(),command:action.command.command().clone(),
                    }),
                }, &mut NeverCrash).unwrap();
            }
            WorkflowStep::Complete => { completed=true;break; }
            other => panic!("original unknown Prepare terminal workflow blocked: expiry={expiry} step={other:?}"),
        }
    }
    assert!(stopped && completed, "original terminal workflow must finish after its exact original Prepare is physically confirmed");
    assert!(!journal.uncertain_actions().unwrap().contains(&prepare),
        "original unknown loader must be retired by exact terminal cleanup, not forgotten");
}

#[test]
fn unknown_prepare_destroy_preserves_backing_and_finishes_without_second_load() {
    unknown_prepare_then_terminal(false);
}

#[test]
fn unknown_prepare_expiry_preserves_backing_and_finishes_without_second_load() {
    unknown_prepare_then_terminal(true);
}
