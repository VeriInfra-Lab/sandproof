//! Original Redis authority and exact control outcomes. Native process
//! observations are supplied by the fixture, rather than a live parent VM.
#![cfg(unix)]

#[allow(dead_code, reason = "selected shared Redis fixture helpers")]
#[path = "../../sandbox-center-store/tests/support/engine.rs"]
mod engine;
#[path = "branch_parent_exit/support.rs"]
mod support;

use sandproof_api::PortError;
use sandproof_center_service::{AuthorityPort, LifecyclePolicy, business::SharedBusinessAuthority};
use sandproof_controller_service::application::branching::{
    BranchControlReply, BranchControlRequest, BranchControlStatus, BranchStartFailure,
    PublicBranchBatchRequest, PublicBranchBatchResult,
};
use sandproof_protocol::lifecycle::{
    ContentDigest, CreateObservation, CurrentCreateReadiness, CurrentReadyFacts,
    LifecycleOperation, PublicCreateRequest, TerminalLifecycleObservation, TerminalStatus,
};

fn start_branches(
    authority: &mut SharedBusinessAuthority,
    request: &PublicCreateRequest,
) -> (
    CreateObservation,
    PublicBranchBatchRequest,
    BranchControlRequest,
) {
    let owner = "8".try_into().unwrap();
    let create = authority
        .accept_public_create(
            &owner,
            request,
            LifecyclePolicy {
                max_timeout_secs: 5,
                max_lifetime_secs: 5,
                startup_timeout_secs: 1,
                public_ipv4_enabled: false,
            },
            || Ok(10),
        )
        .unwrap();
    let ready = CreateObservation {
        request: create.clone(),
        current_request: create.clone(),
        historical_create_succeeded: true,
        readiness: CurrentCreateReadiness::Observed {
            facts: CurrentReadyFacts {
                started_at_ms: 10,
                end_at_ms: 10 + request.parameters.timeout_ms,
                max_end_at_ms: 10 + request.parameters.max_lifetime_ms,
                startup_deadline_ms: 10 + request.parameters.startup_timeout_ms,
                observed_at_ms: 10,
            },
        },
    };
    authority
        .accept_lifecycle_create_observation(&ready)
        .unwrap();
    let session = &create.request.binding.session;
    let batch_request = PublicBranchBatchRequest {
        window: create.request.binding.key.window,
        sequence: 2.try_into().unwrap(),
        expected_version: create.accepted_version,
        checkpoint: "training".try_into().unwrap(),
        count: 2.try_into().unwrap(),
    };
    authority
        .reserve_branch_batch(
            &owner,
            session.clone(),
            batch_request.clone(),
            "old".try_into().unwrap(),
            20,
        )
        .unwrap();
    let start = authority.pending_branch_work(20).unwrap().pop().unwrap();
    authority
        .observe_branch_work(
            &start,
            &BranchControlReply {
                original: start.clone(),
                status: BranchControlStatus::Ready {},
            },
            20,
        )
        .unwrap();
    (
        ready,
        batch_request,
        authority.pending_branch_work(20).unwrap().pop().unwrap(),
    )
}

fn renew(authority: &mut SharedBusinessAuthority, mut ready: CreateObservation) -> u64 {
    let original = &ready.request.request.binding;
    let renewal = authority
        .renew_current(
            &original.key.owner,
            original.session.as_protocol_id(),
            2,
            false,
            30,
        )
        .unwrap();
    let LifecycleOperation::Renew { end_at_ms, .. } = renewal.request.operation else {
        panic!("original SDK renewal expected");
    };
    let version = renewal.accepted_version;
    ready.current_request = renewal;
    let CurrentCreateReadiness::Observed { facts } = &mut ready.readiness else {
        panic!("successful create required");
    };
    facts.end_at_ms = end_at_ms.try_into().unwrap();
    facts.observed_at_ms = 30;
    authority
        .accept_lifecycle_create_observation(&ready)
        .unwrap();
    version
}

#[test]
fn confirmed_branch_parent_exit_survives_same_execution_renewal_and_preserves_cleanup() {
    let engine = engine::Engine::new();
    let (mut authority, create_request) = support::authority(&engine);
    let (ready, batch_request, in_flight) = start_branches(&mut authority, &create_request);
    let owner = &in_flight.operation.owner;
    let session = &in_flight.batch.parent;
    let renewed_version = renew(&mut authority, ready);
    assert_eq!(authority.public_snapshot().unwrap().branch_routes.len(), 2);
    let batch_result = |authority: &SharedBusinessAuthority| {
        authority
            .query_branch_batch(owner, session, batch_request.window, batch_request.sequence)
            .unwrap()
            .result
    };

    let source = ContentDigest::from_sha256([7; 32]);
    let reply = |request| BranchControlReply {
        original: request,
        status: BranchControlStatus::ParentExited { source },
    };
    let mut wrong = in_flight.clone();
    wrong.parent.incarnation = 2.try_into().unwrap();
    wrong.batch.incarnation = wrong.parent.incarnation;
    assert_eq!(
        authority.observe_branch_work(&wrong, &reply(wrong.clone()), 30),
        Err(PortError::Conflict)
    );
    authority
        .observe_branch_work(
            &in_flight,
            &BranchControlReply {
                original: in_flight.clone(),
                status: BranchControlStatus::Pending {},
            },
            30,
        )
        .unwrap();
    assert_eq!(authority.public_snapshot().unwrap().branch_routes.len(), 2);
    authority
        .observe_branch_work(&in_flight, &reply(in_flight.clone()), 30)
        .unwrap();
    assert!(
        authority
            .public_snapshot()
            .unwrap()
            .branch_routes
            .is_empty()
    );
    assert!(authority.pending_branch_work(30).unwrap().is_empty());
    assert_eq!(
        batch_result(&authority),
        PublicBranchBatchResult::CleaningUp {}
    );
    assert_eq!(
        authority.reserve_branch_batch(
            owner,
            session.clone(),
            PublicBranchBatchRequest {
                sequence: 3.try_into().unwrap(),
                expected_version: renewed_version,
                count: 1.try_into().unwrap(),
                ..batch_request.clone()
            },
            "new".try_into().unwrap(),
            30
        ),
        Err(PortError::Conflict)
    );
    let world = authority.snapshot().unwrap();
    let parent = world.center.lifecycle_session(owner, session).unwrap();
    assert_eq!(parent.current_version(), Some(renewed_version));
    assert_eq!(parent.phase(), sandproof_center_core::SessionPhase::Running);

    let destroy = authority
        .destroy_current(owner, session.as_protocol_id())
        .unwrap();
    authority
        .accept_lifecycle_terminal_observation(
            &TerminalLifecycleObservation {
                request: destroy,
                status: TerminalStatus::Cleaned,
            },
            40,
        )
        .unwrap();
    assert_eq!(
        batch_result(&authority),
        PublicBranchBatchResult::Failed {
            reason: BranchStartFailure::ParentEnded,
        }
    );
    assert_eq!(
        authority.observe_branch_work(&in_flight, &reply(in_flight.clone()), 40),
        Err(PortError::Conflict)
    );
}
