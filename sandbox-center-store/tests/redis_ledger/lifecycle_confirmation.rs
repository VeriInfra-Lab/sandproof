//! Actual Redis/CAS regression matrix; physical observations are controlled.
use super::*;
use sandproof_protocol::lifecycle::{
    ConfirmedRenewal, ContentDigest, CreateObservation, CreateRefusalReason, CurrentReadyFacts,
    CurrentResumeReadiness, ResumeObservation, UnloadObservation, UnloadOutcome, UnloadedFacts,
};

#[derive(Clone)]
enum Input {
    Create(CreateObservation),
    Resume(ResumeObservation),
    Renewal(ConfirmedRenewal),
    Refusal(CreateObservation),
    Unload(UnloadObservation),
    Terminal(TerminalLifecycleObservation),
    Expire,
}

impl Input {
    fn pure(&self, state: &CenterState) -> CenterState {
        match self {
            Self::Create(value) => state.accept_lifecycle_create_observation(value).unwrap(),
            Self::Resume(value) => state.accept_lifecycle_resume_observation(value).unwrap(),
            Self::Renewal(value) => state.confirm_lifecycle_renewal(value).unwrap(),
            Self::Refusal(value) => state.accept_lifecycle_create_refusal(value).unwrap().0,
            Self::Unload(value) => state
                .accept_lifecycle_unload_observation(value, 160)
                .unwrap(),
            Self::Terminal(value) => state
                .accept_lifecycle_terminal_observation(value, 160)
                .unwrap(),
            Self::Expire => state.terminate_expired(200).unwrap(),
        }
    }

    fn apply(&self, authority: &mut CenterAuthority) -> Result<(), AuthorityError> {
        match self {
            Self::Create(value) => authority.accept_lifecycle_create_observation(value),
            Self::Resume(value) => authority.accept_lifecycle_resume_observation(value),
            Self::Renewal(value) => authority.confirm_lifecycle_renewal(value),
            Self::Refusal(value) => {
                authority
                    .accept_lifecycle_create_refusal(value)
                    .map(|intent| {
                        assert_eq!(
                            authority.state().unwrap().sandboxes()[0]
                                .operations()
                                .last()
                                .unwrap()
                                .intent(),
                            &intent
                        );
                    })
            }
            Self::Unload(value) => authority.accept_lifecycle_unload_observation(value, 160),
            Self::Terminal(value) => authority.accept_lifecycle_terminal_observation(value, 160),
            Self::Expire => authority.terminate_expired(200),
        }
    }
}

struct Case {
    name: &'static str,
    before: CenterState,
    after: CenterState,
    update: Input,
    replay: Input,
}

fn case(name: &'static str, before: CenterState, update: Input) -> Case {
    let after = update.pure(&before);
    assert_ne!(after, before, "{name} must really update state");
    let mut replay = update.clone();
    if let Input::Refusal(value) = &mut replay {
        value.current_request = after.sandboxes()[0]
            .operations()
            .last()
            .unwrap()
            .intent()
            .clone();
    }
    assert_eq!(
        replay.pure(&after),
        after,
        "{name} must really be a no-op on replay"
    );
    Case {
        name,
        before,
        after,
        update,
        replay,
    }
}

fn ready(intent: &AcceptedLifecycleIntent) -> CreateObservation {
    let LifecycleOperation::Create { create, .. } = &intent.request.operation else {
        unreachable!()
    };
    CreateObservation {
        request: intent.clone(),
        current_request: intent.clone(),
        historical_create_succeeded: true,
        readiness: CurrentCreateReadiness::Observed {
            facts: CurrentReadyFacts {
                started_at_ms: 100,
                end_at_ms: 200,
                max_end_at_ms: 600,
                startup_deadline_ms: create.startup_deadline_ms,
                observed_at_ms: 100,
            },
        },
    }
}

fn unload_resume_cases(running: &CenterState, original: &AcceptedLifecycleIntent) -> (Case, Case) {
    let owner = original.request.binding.key.owner.clone();
    let mut key = original.request.binding.key.clone();
    key.sequence = 2.try_into().unwrap();
    let (unloading, unload) = running
        .accept_lifecycle(
            &owner,
            &LifecycleSubmission::Unload {
                key: key.clone(),
                session: original.request.binding.session.clone(),
                expected_version: 1,
            },
            150,
        )
        .unwrap();
    let LifecycleOperation::Unload {
        source_incarnation,
        capture,
    } = unload.request.operation
    else {
        unreachable!()
    };
    let prior = running.sandboxes()[0].last_running().unwrap();
    let unloaded = case(
        "unload",
        unloading,
        Input::Unload(UnloadObservation {
            request: unload.clone(),
            outcome: UnloadOutcome::Unloaded {
                facts: UnloadedFacts {
                    source_incarnation,
                    capture,
                    manifest_digest: ContentDigest::from_sha256([7; 32]),
                    material_bytes: 7,
                    started_at_ms: prior.started_at_ms,
                    end_at_ms: prior.end_at_ms,
                    max_end_at_ms: prior.max_end_at_ms,
                    startup_deadline_ms: prior.startup_deadline_ms,
                },
            },
        }),
    );
    key.sequence = 3.try_into().unwrap();
    let (resuming, resume) = unloaded
        .after
        .accept_lifecycle(
            &owner,
            &LifecycleSubmission::Resume {
                key,
                session: original.request.binding.session.clone(),
                expected_version: 2,
            },
            170,
        )
        .unwrap();
    let mut facts = prior.clone();
    facts.observed_at_ms = 180;
    let resumed = case(
        "resume",
        resuming,
        Input::Resume(ResumeObservation {
            request: resume.clone(),
            current_request: resume,
            readiness: CurrentResumeReadiness::Observed { facts },
        }),
    );
    (unloaded, resumed)
}

fn cases() -> Vec<Case> {
    let mut wire = serde_json::to_value(center_state()).unwrap();
    wire["bounds"]["operations_per_sandbox"] = 8.into();
    let initial: CenterState = serde_json::from_value(wire).unwrap();
    assert!(initial.is_valid());
    let owner = "1".try_into().unwrap();
    let input = create_submission(lifecycle_fixture::binding(), 1);
    let (creating, original) = initial.accept_lifecycle(&owner, &input, 10).unwrap();
    let create = case("create", creating.clone(), Input::Create(ready(&original)));
    let running = create.after.clone();
    let refusal = case(
        "refusal",
        creating,
        Input::Refusal(CreateObservation {
            request: original.clone(),
            current_request: original.clone(),
            historical_create_succeeded: false,
            readiness: CurrentCreateReadiness::Refused {
                reason: CreateRefusalReason::Capacity,
            },
        }),
    );
    let (unloaded, resumed) = unload_resume_cases(&running, &original);
    let (renewing, renewal) = running
        .accept_lifecycle(
            &owner,
            &LifecycleSubmission::Renew {
                key: running.next_sdk_operation_key(&owner).unwrap(),
                session: original.request.binding.session.clone(),
                expected_version: 1,
                timeout_secs: 1,
                extend_only: false,
            },
            150,
        )
        .unwrap();
    let LifecycleOperation::Renew { end_at_ms, .. } = renewal.request.operation else {
        unreachable!()
    };
    let mut observed = ready(&original);
    observed.current_request = renewal.clone();
    let CurrentCreateReadiness::Observed { facts } = &mut observed.readiness else {
        unreachable!()
    };
    facts.end_at_ms = u64::try_from(end_at_ms).unwrap();
    facts.observed_at_ms = 150;
    let applied = renewing
        .accept_lifecycle_create_observation(&observed)
        .unwrap();
    let confirmation = ConfirmedRenewal {
        result: applied.lifecycle_renewal_confirmation(&renewal).unwrap(),
    };
    let renewal = case("renewal", applied, Input::Renewal(confirmation));
    let (terminating, destroy) = running
        .accept_lifecycle(
            &owner,
            &LifecycleSubmission::Destroy {
                key: running.next_sdk_operation_key(&owner).unwrap(),
                session: original.request.binding.session.clone(),
                expected_version: 1,
            },
            0,
        )
        .unwrap();
    let terminal = case(
        "terminal",
        terminating,
        Input::Terminal(TerminalLifecycleObservation {
            request: destroy,
            status: TerminalStatus::Cleaned,
        }),
    );
    let expiry = case("expiry", running, Input::Expire);
    vec![
        create, resumed, renewal, refusal, unloaded, terminal, expiry,
    ]
}

fn revision(fixture: &Engine) -> u64 {
    fixture.revision().parse().unwrap()
}

#[test]
fn seven_methods_use_one_confirmation_for_noop_and_two_for_real_update() {
    for case in cases() {
        let fixture = Engine::new();
        let mut authority = center_authority_with_state(&fixture, fixture.address, &case.before);
        let before = revision(&fixture);
        case.update.apply(&mut authority).unwrap();
        assert_eq!(revision(&fixture) - before, 2, "{} update", case.name);
        assert_eq!(authority.state().unwrap(), &case.after);
        let before = revision(&fixture);
        case.replay.apply(&mut authority).unwrap();
        assert_eq!(revision(&fixture) - before, 1, "{} no-op", case.name);
        assert_eq!(authority.state().unwrap(), &case.after);
    }
}

#[test]
fn seven_noops_keep_first_confirmation_unknown_fenced_until_recovery() {
    for case in cases() {
        let fixture = Engine::new();
        let proxy = ReplyLoss::new(fixture.address);
        let mut authority = center_authority_with_state(&fixture, proxy.address, &case.after);
        let before = revision(&fixture);
        proxy.lose_after(0);
        assert_eq!(
            case.replay.apply(&mut authority),
            Err(AuthorityError::Store(StoreError::OutcomeUnknown)),
            "{}",
            case.name
        );
        assert_eq!(revision(&fixture) - before, 1);
        assert!(matches!(
            authority.state(),
            Err(AuthorityError::RecoveryRequired)
        ));
        assert_eq!(
            case.replay.apply(&mut authority),
            Err(AuthorityError::RecoveryRequired)
        );
        authority.recover().unwrap();
        assert_eq!(authority.state().unwrap(), &case.after);
        let before = revision(&fixture);
        case.replay.apply(&mut authority).unwrap();
        assert_eq!(revision(&fixture) - before, 1);
        assert_eq!(authority.state().unwrap(), &case.after);
    }
}

#[test]
fn seven_updates_keep_unknown_commit_and_recover_exact_original_result() {
    for case in cases() {
        let fixture = Engine::new();
        let proxy = ReplyLoss::new(fixture.address);
        let mut authority = center_authority_with_state(&fixture, proxy.address, &case.before);
        let before = revision(&fixture);
        proxy.lose_after(1);
        assert_eq!(
            case.update.apply(&mut authority),
            Err(AuthorityError::Store(StoreError::OutcomeUnknown)),
            "{}",
            case.name
        );
        assert_eq!(revision(&fixture) - before, 2);
        assert_eq!(
            case.replay.apply(&mut authority),
            Err(AuthorityError::RecoveryRequired)
        );
        authority.recover().unwrap();
        assert_eq!(authority.state().unwrap(), &case.after);
        case.replay.apply(&mut authority).unwrap();
        assert_eq!(
            authority.state().unwrap(),
            &case.after,
            "{} cannot allocate a replacement or refresh retention",
            case.name
        );
    }
}

#[test]
fn seven_noops_reject_unhealthy_storage_and_do_not_clear_fence_when_it_returns() {
    for case in cases() {
        let fixture = Engine::new();
        let mut authority = center_authority_with_state(&fixture, fixture.address, &case.after);
        let before = revision(&fixture);
        let _: () = redis::cmd("CONFIG")
            .arg("SET")
            .arg("maxmemory")
            .arg(1)
            .query(&mut connect(fixture.address))
            .unwrap();
        assert_eq!(
            case.replay.apply(&mut authority),
            Err(AuthorityError::Store(StoreError::Configuration)),
            "{}",
            case.name
        );
        assert_eq!(revision(&fixture), before);
        let _: () = redis::cmd("CONFIG")
            .arg("SET")
            .arg("maxmemory")
            .arg(MEMORY)
            .query(&mut connect(fixture.address))
            .unwrap();
        assert_eq!(
            case.replay.apply(&mut authority),
            Err(AuthorityError::RecoveryRequired)
        );
        authority.recover().unwrap();
        assert_eq!(authority.state().unwrap(), &case.after);
    }
}

#[test]
fn already_cleaned_can_still_require_a_real_retention_update() {
    let case = cases().into_iter().find(|c| c.name == "terminal").unwrap();
    let Input::Terminal(result) = case.update else {
        unreachable!()
    };
    let unsampled = case
        .before
        .accept_lifecycle_terminal_observation(&result, 0)
        .unwrap();
    let expected = unsampled
        .accept_lifecycle_terminal_observation(&result, 160)
        .unwrap();
    assert_ne!(unsampled, expected);
    let fixture = Engine::new();
    let mut authority = center_authority_with_state(&fixture, fixture.address, &unsampled);
    let before = revision(&fixture);
    authority
        .accept_lifecycle_terminal_observation(&result, 160)
        .unwrap();
    assert_eq!(revision(&fixture) - before, 2);
    assert_eq!(authority.state().unwrap(), &expected);
}

#[test]
fn seven_noops_use_the_last_revision_once_and_never_wrap_or_clear_exhaustion() {
    for case in cases() {
        let fixture = Engine::new();
        let authority = center_authority_with_state(&fixture, fixture.address, &case.after);
        drop(authority);
        fixture.field("revision", (u64::MAX - 2).to_string());
        let (ledger, _) = RedisLedger::open(
            endpoint(fixture.address),
            Engine::config(case.after.required_authority_bytes()),
            fixture.lease(),
        )
        .unwrap();
        let mut authority = CenterAuthority::attach(ledger).unwrap();
        assert_eq!(revision(&fixture), u64::MAX - 1);
        case.replay.apply(&mut authority).unwrap();
        assert_eq!(
            revision(&fixture),
            u64::MAX,
            "{} consumes one revision",
            case.name
        );
        assert_eq!(authority.state().unwrap(), &case.after);
        assert_eq!(
            case.replay.apply(&mut authority),
            Err(AuthorityError::Store(StoreError::RevisionExhausted))
        );
        assert_eq!(revision(&fixture), u64::MAX);
        assert_eq!(
            case.replay.apply(&mut authority),
            Err(AuthorityError::RecoveryRequired)
        );
        assert!(authority.recover().is_err());
        assert_eq!(revision(&fixture), u64::MAX);
    }
}
