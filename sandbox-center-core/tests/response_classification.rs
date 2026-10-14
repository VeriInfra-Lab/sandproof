#[path = "../../sandbox-protocol/tests/support/lifecycle.rs"]
mod fixture;
use fixture::{request, response};
use sandproof_center_core::{DecisionError, Operation, ResponseDisposition};
use sandproof_protocol::internal::PositiveCounter;
use sandproof_protocol::internal::lifecycle::{Intent, LifecycleFact, LifecycleResponse};

// Frozen pre-refactor checks. Test evidence, not an alternative production path
// or a general equivalence proof of the entire center handler.
fn original_classify(
    operation: &Operation,
    observed: Option<PositiveCounter>,
    actual: &LifecycleResponse,
) -> Result<ResponseDisposition, DecisionError> {
    let rank = |fact| match fact {
        LifecycleFact::Recorded => 0,
        LifecycleFact::Stopped => 1,
        _ => 2,
    };
    actual
        .validate_for(&operation.request)
        .map_err(|_| DecisionError::Stale)?;
    if operation.response.as_ref() == Some(actual) {
        return Ok(ResponseDisposition::Duplicate);
    }
    if observed.is_some_and(|v| actual.observation_version.get() <= v.get())
        || operation
            .response
            .as_ref()
            .is_some_and(|r| rank(actual.fact) <= rank(r.fact))
    {
        return Err(DecisionError::Stale);
    }
    Ok(ResponseDisposition::Advance)
}

#[test]
fn classifier_preserves_original_order_and_results() {
    let facts = [
        LifecycleFact::Recorded,
        LifecycleFact::Ready {
            started_at_ms: 100,
            end_at_ms: 200,
            max_end_at_ms: 600,
        },
        LifecycleFact::Renewed { end_at_ms: 200 },
        LifecycleFact::Stopped,
        LifecycleFact::Cleaned,
        LifecycleFact::NotAccepted,
    ];
    for intent in [
        Intent::Create,
        Intent::Renew { end_at_ms: 200 },
        Intent::Destroy,
    ] {
        let expected = request(1, 1, intent);
        let mut saved = vec![None];
        saved.extend(facts.map(|fact| Some(response(&expected, 2, fact))));
        for prior in saved {
            let operation = Operation {
                request: expected.clone(),
                response: prior,
            };
            for observation in [1, 2, u64::MAX] {
                for fact in facts {
                    let actual = response(&expected, observation, fact);
                    let mut wrong_identity = actual.clone();
                    wrong_identity.identity.operation_id = "operation-2".try_into().unwrap();
                    for candidate in [actual, wrong_identity] {
                        for observed in [
                            None,
                            Some(1.try_into().unwrap()),
                            Some(2.try_into().unwrap()),
                            Some(u64::MAX.try_into().unwrap()),
                        ] {
                            assert_eq!(
                                operation.classify_response(observed, &candidate),
                                original_classify(&operation, observed, &candidate)
                            );
                            assert_eq!(operation.request, expected);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn exact_replay_is_checked_before_observation_but_after_original_request() {
    let expected = request(1, 1, Intent::Create);
    let exact = response(&expected, 1, LifecycleFact::Recorded);
    let operation = Operation {
        request: expected.clone(),
        response: Some(exact.clone()),
    };
    assert_eq!(
        operation.classify_response(Some(u64::MAX.try_into().unwrap()), &exact),
        Ok(ResponseDisposition::Duplicate)
    );
    let invalid = response(&expected, 1, LifecycleFact::Stopped);
    let operation = Operation {
        request: expected,
        response: Some(invalid.clone()),
    };
    assert_eq!(
        operation.classify_response(None, &invalid),
        Err(DecisionError::Stale)
    );
}
