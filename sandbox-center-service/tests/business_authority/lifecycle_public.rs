//! Original Redis business envelope; cache observations are controlled inputs.
//! No VM/Ready/Unloaded or manager-process acceptance is claimed by this fixture.
use super::*;
use sandproof_center_core::LifecycleWindow;
use sandproof_center_service::LifecyclePolicy;
use sandproof_protocol::lifecycle::{
    CenterIdentity, CreateParameters, PublicCreateRequest, PublicSessionPhase,
};
use std::sync::atomic::{AtomicUsize, Ordering};

fn policy() -> LifecyclePolicy {
    LifecyclePolicy {
        max_timeout_secs: 5,
        max_lifetime_secs: 5,
        startup_timeout_secs: 1,
        public_ipv4_enabled: false,
    }
}
pub(super) fn request() -> PublicCreateRequest {
    let template =
        PublicTemplate::from_binding(descriptor("base", None), "base-r1".try_into().unwrap())
            .unwrap();
    PublicCreateRequest {
        window: 1.try_into().unwrap(),
        sequence: 1.try_into().unwrap(),
        parameters: CreateParameters {
            template_revision: template.revision().clone(),
            template_digest: template.digest(),
            resources: resources(1),
            options: serde_json::to_vec(&CreateOptions {
                networking: Some(CreateNetworking {
                    allow_internet_access: Some(false),
                    network: None,
                }),
                ..CreateOptions::default()
            })
            .unwrap(),
            startup_timeout_ms: 1000,
            timeout_ms: 5000,
            max_lifetime_ms: 5000,
        },
    }
}
pub(super) fn setup(fixture: &Engine, proxy: &reply_loss::ReplyLoss) -> SharedBusinessAuthority {
    let mut state = envelope();
    state.center = state
        .center
        .enable_lifecycle(
            CenterIdentity {
                deployment: engine::deployment(),
                center: "center".try_into().unwrap(),
            },
            vec![LifecycleWindow {
                owner: "8".try_into().unwrap(),
                window: 1.try_into().unwrap(),
            }],
        )
        .unwrap();
    let (ledger, _) = RedisLedger::initialize(
        engine::endpoint(proxy.address),
        Engine::config(state.required_block_bytes()),
        fixture.lease(),
        &state,
    )
    .unwrap();
    let business = SharedBusinessAuthority::new(CenterAuthority::attach(ledger).unwrap());
    let p = request().parameters;
    let cache = sandproof_internal::cache::CacheRequest {
        node: wire::node("host-a"),
        revision: p.template_revision,
        digest: p.template_digest,
        descriptor: descriptor("base", None),
    };
    business
        .register_cache(
            &cache,
            &sandproof_internal::cache::CacheAvailability {
                node: cache.node.clone(),
                revision: cache.revision.clone(),
                digest: cache.digest,
            },
        )
        .unwrap();
    business
}

// BusinessEnvelope/RegistryMetadata intentionally have no PartialEq. Keep
// direct equality for ALL CenterState fields, including private lifecycle
// state, rather than replacing that check with a JSON projection. For metadata
// compare the complete durable representation and assert all five actual
// RegistryMetadata fields are present; neither type has a serde skip/custom
// serializer. Also independently check the original protected registration.
fn assert_same_business_state(
    actual: &BusinessEnvelope<RegistryMetadata>,
    expected: &BusinessEnvelope<RegistryMetadata>,
) {
    assert_eq!(actual.center, expected.center);
    assert!(actual.metadata.registration_matches(&expected.metadata));
    let metadata = serde_json::to_value(&actual.metadata).unwrap();
    let mut fields: Vec<_> = metadata
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        [
            "budget",
            "catalog",
            "private_builds",
            "registrations",
            "registry"
        ]
    );
    assert_eq!(metadata, serde_json::to_value(&expected.metadata).unwrap());
}

#[test]
fn public_replay_preserves_full_relative_parameters_without_reading_new_clock() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = setup(&fixture, &proxy);
    let owner = "8".try_into().unwrap();
    let request = request();
    let reads = AtomicUsize::new(0);
    let first = business
        .accept_public_create(&owner, &request, policy(), || {
            reads.fetch_add(1, Ordering::SeqCst);
            Ok(100)
        })
        .unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    let mut lowered = policy();
    lowered.max_timeout_secs = 1;
    lowered.max_lifetime_secs = 1;
    assert_eq!(
        business.accept_public_create(&owner, &request, lowered, || panic!(
            "replay must not read current clock"
        )),
        Ok(first.clone())
    );
    let mut changed = request.clone();
    changed.parameters.timeout_ms -= 1;
    assert_eq!(
        business.accept_public_create(&owner, &changed, policy(), || panic!(
            "full-input conflict precedes current clock"
        )),
        Err(PortError::Conflict)
    );
    let view = business
        .query_public_lifecycle(&owner, &first.request.binding.session)
        .unwrap();
    assert_eq!(view.phase, PublicSessionPhase::Creating);
    assert_eq!(view.operation, first.request.binding.key);
    assert_eq!(view.current_version, 1);
    assert_eq!(
        business.query_public_lifecycle(&"9".try_into().unwrap(), &view.session),
        Err(PortError::NotFound)
    );
    assert_eq!(business.snapshot().unwrap().center.sandboxes().len(), 1);
}

#[test]
fn protected_template_shape_options_and_lifetime_are_checked_before_commit() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = setup(&fixture, &proxy);
    let owner = "8".try_into().unwrap();
    let before = serde_json::to_value(business.snapshot().unwrap()).unwrap();
    for (mut request, error) in [
        (
            {
                let mut r = request();
                r.parameters.resources.cpu += 1;
                r
            },
            PortError::Invalid,
        ),
        (
            {
                let mut r = request();
                r.parameters.template_digest[0] ^= 1;
                r
            },
            PortError::NotFound,
        ),
        (
            {
                let mut r = request();
                r.parameters.max_lifetime_ms = 6000;
                r
            },
            PortError::Invalid,
        ),
        (
            {
                let mut r = request();
                r.parameters.options = vec![255];
                r
            },
            PortError::Invalid,
        ),
    ] {
        request.sequence = 2.try_into().unwrap();
        assert_eq!(
            business.accept_public_create(&owner, &request, policy(), || panic!(
                "invalid protected input precedes current clock"
            )),
            Err(error)
        );
        assert_eq!(
            serde_json::to_value(business.snapshot().unwrap()).unwrap(),
            before
        );
    }
}

#[test]
fn unknown_public_commit_fences_query_then_recovers_original_key_and_deadline() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = setup(&fixture, &proxy);
    let owner = "8".try_into().unwrap();
    let request = request();
    proxy.lose_after(1);
    assert_eq!(
        business.accept_public_create(&owner, &request, policy(), || Ok(100)),
        Err(PortError::OutcomeUnknown)
    );
    let session = sandproof_protocol::lifecycle::SessionId::try_from("sandbox-1").unwrap();
    assert_eq!(
        business.query_public_lifecycle(&owner, &session),
        Err(PortError::OutcomeUnknown)
    );
    AuthorityPort::recover(&mut business.clone()).unwrap();
    let intent = business
        .accept_public_create(&owner, &request, policy(), || {
            panic!("recovered original key must not allocate a new deadline")
        })
        .unwrap();
    assert_eq!(intent.request.binding.session, session);
    assert_eq!(business.snapshot().unwrap().center.sandboxes().len(), 1);
    let sandproof_protocol::lifecycle::LifecycleOperation::Create { create, .. } =
        intent.request.operation
    else {
        panic!("expected original create")
    };
    assert_eq!(create.startup_deadline_ms, 1100);
}

const OWNER_KEY: &str = "lifecycle-http-owner-fixture-only-key-0001";
const OTHER_KEY: &str = "lifecycle-http-other-fixture-only-key-0002";
fn public_credentials() -> Vec<sandproof_api::ApiCredential> {
    vec![
        sandproof_api::ApiCredential {
            tenant: "8".into(),
            secret: OWNER_KEY.into(),
        },
        sandproof_api::ApiCredential {
            tenant: "9".into(),
            secret: OTHER_KEY.into(),
        },
    ]
}
async fn send_public(
    router: &axum::Router,
    method: &str,
    path: &str,
    secret: &str,
    value: Option<&serde_json::Value>,
) -> axum::response::Response {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("x-api-key", secret);
    let body = if let Some(value) = value {
        builder = builder.header("content-type", "application/json");
        Body::from(serde_json::to_vec(value).unwrap())
    } else {
        Body::empty()
    };
    router
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap()
}
async fn read_public<T: serde::de::DeserializeOwned>(response: axum::response::Response) -> T {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap(),
    )
    .unwrap()
}
async fn assert_public_views<P: sandproof_api::ControllerPort>(
    router: &axum::Router,
    port: std::sync::Arc<P>,
    accepted: &sandproof_protocol::lifecycle::PublicAcceptance,
) {
    use axum::http::StatusCode;
    use sandproof_protocol::lifecycle::PublicSessionView;
    let path = format!(
        "/sandproof/lifecycle/sessions/{}",
        accepted.session.as_protocol_id().as_str()
    );
    let response = send_public(router, "GET", &path, OWNER_KEY, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let view: PublicSessionView = read_public(response).await;
    assert_eq!(view.session, accepted.session);
    assert_eq!(view.operation, accepted.key);
    assert_eq!(view.phase, PublicSessionPhase::Creating);
    assert_eq!(
        send_public(router, "GET", &path, OTHER_KEY, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // A v3 durable acceptance cannot become a legacy session/credential or
    // physical Running claim through the old public projection.
    let legacy = sandproof_api::router(port, public_credentials()).unwrap();
    let path = format!("/sandboxes/{}", accepted.session.as_protocol_id().as_str());
    assert_eq!(
        send_public(&legacy, "GET", &path, OWNER_KEY, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_public_http_port_commits_only_authenticated_full_input_and_replays() {
    use axum::http::StatusCode;
    use sandproof_api::admission::RequestAdmission;
    use sandproof_center_service::CenterPort;
    use sandproof_protocol::lifecycle::PublicAcceptance;
    use std::sync::Arc;
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = setup(&fixture, &proxy);
    let (_, application, _) = templates::application(&business, false);
    let port = Arc::new(CenterPort::new(Arc::new(application)));
    let router = sandproof_api::lifecycle::router_with_admission(
        port.clone(),
        public_credentials(),
        Arc::new(RequestAdmission::default()),
    )
    .unwrap();
    let input = request();
    let value = serde_json::to_value(&input).unwrap();
    let path = "/sandproof/lifecycle/create";
    let before = business.snapshot().unwrap();
    assert_eq!(
        send_public(&router, "POST", path, "wrong", Some(&value))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let mut forged = value.clone();
    forged["owner"] = "9".into();
    assert_eq!(
        send_public(&router, "POST", path, OWNER_KEY, Some(&forged))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_same_business_state(&business.snapshot().unwrap(), &before);
    let response = send_public(&router, "POST", path, OWNER_KEY, Some(&value)).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let accepted: PublicAcceptance = read_public(response).await;
    assert_eq!(accepted.key.owner.as_str(), "8");
    assert_eq!(accepted.key.deployment, engine::deployment());
    assert_eq!(accepted.key.window, input.window);
    assert_eq!(accepted.key.sequence, input.sequence);
    let saved = business.snapshot().unwrap();
    assert_eq!(saved.center.sandboxes().len(), 1);
    assert_eq!(saved.center.pending_lifecycle().len(), 1);
    let response = send_public(&router, "POST", path, OWNER_KEY, Some(&value)).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(read_public::<PublicAcceptance>(response).await, accepted);
    assert_same_business_state(&business.snapshot().unwrap(), &saved);
    let mut changed = value;
    changed["parameters"]["timeout_ms"] = 4999.into();
    assert_eq!(
        send_public(&router, "POST", path, OWNER_KEY, Some(&changed))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_same_business_state(&business.snapshot().unwrap(), &saved);
    assert_public_views(&router, port, &accepted).await;
}
