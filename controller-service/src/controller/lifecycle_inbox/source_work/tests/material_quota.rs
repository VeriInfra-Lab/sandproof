//! Real journal transactions with controlled host facts, not physical capture.
//! Original public entry points create every row; no charge or GC record is
//! inserted, rewritten or held back to manufacture the refusal.
use super::*;
use crate::capture::{CaptureRequestEndOutcome, OriginalContinueWorkRequest};
use sandproof_protocol::{WorkOutcome, WorkReceipt};

const GIB: u64 = 1024 * 1024 * 1024;
const MATERIAL_LIMIT: u64 = 24 * GIB;
// Independent expected value for the real scene's 10 MB state, 2 GiB RAM and
// 1 GiB disk upper caps, including overlapping copies and prepaid metadata.
const RESERVATION: u64 = 6_463_040_768;

fn scene_full(record: &SourceWorkRecord) -> crate::capture::FullCaptureWorkRequest {
    let mut full = full_request(record);
    full.state_bytes = 10_000_000;
    full.memory_bytes = 2 * GIB;
    full.disk_bytes = GIB;
    full
}

pub(super) fn resumed_with_scene_budget(
    path: &Path,
) -> (ControllerJournal, AcceptedLifecycleIntent) {
    let mut source_limits = config();
    source_limits.max_disk_bytes = GIB;
    source_limits.max_material_bytes = MATERIAL_LIMIT;
    let mut journal = v3::open(path);
    journal
        .initialize_lifecycle_inbox(v3::config(), source_limits, &mut NeverCrash)
        .unwrap();
    drop(journal);
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        source: Some(SourceProcessIdentity {
            pid: 1234,
            start_time_ticks: 5678,
            boot_id: "fixture-boot".into(),
        }),
        ..State::default()
    }));
    let app = v3::application(path, state, false);
    app.record_lifecycle_intent(&v3::request(), &v3::context())
        .unwrap();
    app.establish_lifecycle_host_epoch().unwrap();
    v3::advance(&app).unwrap();
    drop(app);
    let mut journal = v3::open(path);
    bootstrap(&mut journal);
    journal
        .record_lifecycle_intent(&unload(), &v3::context(), &mut NeverCrash)
        .unwrap();
    let source = observed_for_full(&mut journal);
    let accepted = journal
        .admit_lifecycle_full_capture::<Evidence>(
            &source.work,
            &v3::context(),
            &scene_full(&source),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    finish_full_component(&mut journal, &accepted);
    let sandbox = source.work.anchor.local.sandbox;
    let slot = journal.host_slot_context(sandbox).unwrap().unwrap();
    journal
        .release_host_slot(slot, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    let resume = record_real_resume(&mut journal);
    admit_real_resume(&mut journal, &resume);
    journal
        .register_data_credential(sandbox, 2, [99; 32], &mut NeverCrash)
        .unwrap();
    finish_restored_component_with(&mut journal, sandbox, restore_exit::application_evidence);
    (journal, resume)
}

pub(super) fn next_unload(
    previous: &AcceptedLifecycleIntent,
    capture: u64,
) -> AcceptedLifecycleIntent {
    let mut intent = previous.clone();
    intent.accepted_version += 1;
    intent.request.binding.expected = ExpectedSession::Version(previous.accepted_version);
    intent.request.binding.key.sequence = intent.accepted_version.try_into().unwrap();
    intent.request.operation = LifecycleOperation::Unload {
        source_incarnation: 102.try_into().unwrap(),
        capture: capture.try_into().unwrap(),
    };
    intent
}

fn observe_source(
    journal: &mut ControllerJournal,
    intent: &AcceptedLifecycleIntent,
) -> SourceWorkRecord {
    journal
        .record_lifecycle_intent(intent, &v3::context(), &mut NeverCrash)
        .unwrap();
    let source = journal
        .prepare_lifecycle_source_work::<Evidence>(intent, &v3::context(), 10, &mut NeverCrash)
        .unwrap();
    journal
        .save_source_observation::<Evidence>(
            &source.work,
            &v3::context(),
            observation(source.work.requirement()),
            10,
            &mut NeverCrash,
        )
        .unwrap()
}

pub(super) fn continue_explicit_no_request(
    journal: &mut ControllerJournal,
    intent: &AcceptedLifecycleIntent,
) -> OriginalContinueWorkRequest {
    let source = observe_source(journal, intent);
    journal
        .admit_lifecycle_full_capture::<Evidence>(
            &source.work,
            &v3::context(),
            &scene_full(&source),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    let sandbox = source.work.anchor.local.sandbox;
    let action = claim_full_before_worker_release(journal, sandbox);
    let index = intent.accepted_version;
    let worker = crate::capture::CaptureWorkerBinding::new(
        SourceProcessIdentity {
            pid: u32::try_from(2300 + index).unwrap(),
            start_time_ticks: 9800 + index,
            boot_id: "fixture-boot".into(),
        },
        ContentDigest::from_sha256([u8::try_from(index).unwrap(); 32]),
    )
    .unwrap();
    let permit = journal
        .authorize_lifecycle_full_capture(
            &source.work,
            &v3::context(),
            &action,
            worker.clone(),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    assert_eq!(permit.worker(), &worker);
    drop(permit);
    let record = journal.lifecycle_capture_record(sandbox).unwrap().unwrap();
    let mut end = actual_api_end(&record);
    end.outcome = CaptureRequestEndOutcome::NoRequest;
    // Explicit controlled provider fact, just as in the existing End tests;
    // neither a dropped permit nor worker disappearance establishes NoRequest.
    journal
        .record_lifecycle_capture_request_end(
            &source.work,
            &v3::context(),
            end.clone(),
            &mut NeverCrash,
        )
        .unwrap();
    let work = journal
        .prepare_lifecycle_original_continue(
            &source.work,
            &v3::context(),
            Some(end),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    let activation = journal
        .claim_next(Some(sandbox), &mut NeverCrash)
        .unwrap()
        .unwrap();
    assert_eq!(
        *activation.command.command(),
        work.identity().activation_command()
    );
    let original_permit = journal
        .lifecycle_original_continue_permit(&work, &activation, &v3::context(), 10)
        .unwrap();
    assert_eq!(
        original_permit.work_binding(),
        &work.binding_digest().unwrap()
    );
    let receipt = WorkReceipt {
        identity: activation.identity.clone(),
        outcome: WorkOutcome::ObservedSuccess(restore_exit::application_evidence(&activation)),
    };
    journal.complete(&receipt, &mut NeverCrash).unwrap();
    journal
        .publish_lifecycle_original_continued(
            &work,
            &activation,
            &receipt,
            &v3::context(),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    work
}

fn assert_charges(journal: &ControllerJournal, full_count: usize) -> u64 {
    let records = journal.lifecycle_source_work(&v3::config().node).unwrap();
    let captures: Vec<_> = records
        .iter()
        .filter_map(|record| record.capture.as_ref())
        .collect();
    assert_eq!(captures.len(), full_count);
    for capture in captures {
        assert_eq!(capture.reserved_material_bytes, RESERVATION);
    }
    let charged: u64 = records
        .iter()
        .map(|record| super::super::retirement::material_charge(record).unwrap())
        .sum();
    assert_eq!(charged, u64::try_from(full_count).unwrap() * RESERVATION);
    assert!(records.iter().all(|record| record.retirement.is_none()));
    charged
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one original quota transaction history and its reopen invariants"
)]
fn three_original_full_charges_refuse_fourth_before_pause_and_keep_prior_continued_qualification() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, resume) = resumed_with_scene_budget(&path);
    assert_charges(&journal, 1);
    let first_failed = next_unload(&resume, 2);
    let first_continue = continue_explicit_no_request(&mut journal, &first_failed);
    assert_charges(&journal, 2);
    let second_failed = next_unload(&first_failed, 3);
    let original = continue_explicit_no_request(&mut journal, &second_failed);
    assert!(original.identity().epoch > first_continue.identity().epoch);
    assert_eq!(original.identity().vm, first_continue.identity().vm);
    assert_eq!(original.source().source, first_continue.source().source);
    let held = assert_charges(&journal, 3);
    assert!(held <= MATERIAL_LIMIT && MATERIAL_LIMIT < held + RESERVATION);
    let historical = journal
        .lifecycle_original_continued_result(&second_failed, &v3::context())
        .unwrap()
        .unwrap();
    let refused = next_unload(&second_failed, 4);
    let source = observe_source(&mut journal, &refused);
    let sandbox = source.work.anchor.local.sandbox;
    let before = journal.world().unwrap();
    let sources = journal.lifecycle_source_work(&v3::config().node).unwrap();
    let slot = journal.host_slot_context(sandbox).unwrap();
    let lifetime = resume_lifetime(&journal, sandbox);
    let credential = journal.data_credential(sandbox).unwrap();
    let operation_id = journal.next_operation_id(sandbox).unwrap();
    let result = journal.admit_lifecycle_full_capture::<Evidence>(
        &source.work,
        &v3::context(),
        &scene_full(&source),
        10,
        &mut NeverCrash,
    );
    assert!(matches!(
        result,
        Err(JournalError::CaptureMaterialQuotaExceeded)
    ));
    assert_eq!(journal.world().unwrap(), before);
    assert_eq!(journal.next_operation_id(sandbox).unwrap(), operation_id);
    assert_eq!(
        journal.lifecycle_source_work(&v3::config().node).unwrap(),
        sources
    );
    assert_eq!(journal.host_slot_context(sandbox).unwrap(), slot);
    assert_eq!(resume_lifetime(&journal, sandbox), lifetime);
    assert_eq!(journal.data_credential(sandbox).unwrap(), credential);
    assert_eq!(
        journal
            .lifecycle_work_state(&refused, &v3::context())
            .unwrap()
            .local_operation,
        None
    );
    assert!(
        journal
            .lifecycle_source_work(&v3::config().node)
            .unwrap()
            .iter()
            .find(|record| record.work == source.work)
            .unwrap()
            .capture
            .is_none()
    );
    assert_charges(&journal, 3);
    journal
        .close_lifecycle_unload_before_admission(&refused, &v3::context(), 10, &mut NeverCrash)
        .unwrap();
    let absent = journal
        .lifecycle_continued_exit_candidates::<Evidence>(&resume, &v3::context(), 10)
        .unwrap();
    assert!(
        absent.0.is_none() && absent.1.is_none(),
        "unpublished history grants no qualification"
    );
    let candidate = journal
        .lifecycle_ready_candidate::<Evidence>(&resume, &v3::context(), 10)
        .unwrap();
    assert!(candidate.can_probe());
    let probes: Vec<_> = candidate
        .actions
        .iter()
        .map(|action| {
            Some(WorkReceipt {
                identity: action.identity.clone(),
                outcome: WorkOutcome::ObservedSuccess(restore_exit::application_evidence(action)),
            })
        })
        .collect();
    let view = journal
        .finish_lifecycle_ready(&candidate, &v3::context(), &probes, true, 10)
        .unwrap();
    let saved = journal
        .publish_pre_admission_original_continued(&view, &v3::context(), 10, &mut NeverCrash)
        .unwrap()
        .unwrap();
    assert_eq!(
        journal.world().unwrap(),
        before,
        "a refusal and its fixed result create no native operation"
    );
    assert_eq!(
        journal
            .lifecycle_original_continued_result(&second_failed, &v3::context())
            .unwrap(),
        Some(historical)
    );
    assert_eq!(
        journal
            .lifecycle_work_state(&refused, &v3::context())
            .unwrap()
            .local_operation,
        None
    );
    assert_charges(&journal, 3);
    let qualified = journal
        .lifecycle_continued_exit_candidates::<Evidence>(&resume, &v3::context(), 10)
        .unwrap();
    assert!(
        qualified.0.is_none(),
        "prior Continue must not borrow the old Resume fence"
    );
    let continued = qualified
        .1
        .as_ref()
        .expect("the actual previous Continue remains qualified");
    assert_eq!(continued.work, original);
    assert_eq!(continued.current_request(), &refused);
    assert!(
        journal
            .lifecycle_restore_exit_candidate(&resume, &v3::context(), 10)
            .unwrap()
            .is_none()
    );
    journal
        .record_lifecycle_intent(&refused, &v3::context(), &mut NeverCrash)
        .unwrap();
    assert_eq!(
        journal
            .lifecycle_original_continued_result(&refused, &v3::context())
            .unwrap(),
        Some(saved.clone())
    );
    drop(journal);
    let mut journal = v3::open(&path);
    bootstrap(&mut journal);
    assert_eq!(journal.world().unwrap(), before);
    assert_charges(&journal, 3);
    assert_eq!(
        journal
            .lifecycle_original_continued_result(&refused, &v3::context())
            .unwrap(),
        Some(saved)
    );
    assert_eq!(
        journal
            .lifecycle_continued_exit_candidates::<Evidence>(&resume, &v3::context(), 10)
            .unwrap(),
        qualified
    );
}
