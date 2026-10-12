//! Count actual Redis CAS revisions through the production business adapter.
//! Physical effects still use the original controlled host, not KVM.
use super::*;

fn revision(fixture: &Fixture) -> u64 {
    fixture.engine.revision().parse().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_list_and_routing_confirm_exactly_once_independent_of_page_size() {
    let fixture = Fixture::new().await;
    let application = fixture.application();
    let router = api_router(&application);
    let mut sessions = Vec::new();
    for rank in ["a", "b"] {
        let (status, session) = send(&router, "POST", "/v2/sandboxes", ALICE,
            Some(json!({"templateID":"base", "timeout":60, "allow_internet_access":false, "metadata":{"rank":rank}}))).await;
        assert_eq!(status, StatusCode::CREATED, "{session}");
        sessions.push(session);
    }
    for (query, count) in [
        ("/v2/sandboxes", 2),
        ("/v2/sandboxes?limit=1", 1),
        ("/v2/sandboxes?metadata=rank%3Da", 1),
        ("/v2/sandboxes?template=missing", 0),
    ] {
        let before = revision(&fixture);
        let (status, listed) = send(&router, "GET", query, ALICE, None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert_eq!(listed.as_array().unwrap().len(), count);
        assert_eq!(revision(&fixture) - before, 1, "{query}");
    }
    let before = revision(&fixture);
    assert_eq!(
        send(&router, "GET", "/v2/sandboxes", BOB, None).await.1,
        json!([])
    );
    assert_eq!(revision(&fixture) - before, 1);
    for session in sessions {
        let sandbox = session["sandboxID"].as_str().unwrap();
        let before = revision(&fixture);
        let (status, detail) = send(
            &router,
            "GET",
            &format!("/sandboxes/{sandbox}"),
            ALICE,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{detail}");
        assert_eq!(detail["templateID"], "base-fixed");
        assert_eq!(detail["envdVersion"], "fixed-envd");
        assert_eq!(revision(&fixture) - before, 1);
        let before = revision(&fixture);
        let route = application
            .resolve_data_route(session["envdAccessToken"].as_str().unwrap())
            .unwrap();
        assert_eq!(route.request.identity.sandbox_id.as_str(), sandbox);
        assert_eq!(revision(&fixture) - before, 1);
        assert_eq!(
            send(
                &router,
                "DELETE",
                &format!("/sandboxes/{sandbox}"),
                ALICE,
                None
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        application.snapshot().unwrap().reserved(None, None),
        Some(Resources::ZERO)
    );
    drop((router, application));
    fixture.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn query_failure_and_reconnect_cannot_reuse_cached_bindings_or_lift_the_fence() {
    let mut fixture = Fixture::new().await;
    let application = fixture.application();
    let router = api_router(&application);
    let (status, session) = send(
        &router,
        "POST",
        "/v2/sandboxes",
        ALICE,
        Some(json!({"templateID":"base", "timeout":60, "allow_internet_access":false})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    let sandbox = session["sandboxID"].as_str().unwrap();
    let token = session["envdAccessToken"].as_str().unwrap();
    fixture.engine.stop();
    assert_eq!(
        application.get("1", sandbox),
        Err(PortError::OutcomeUnknown)
    );
    fixture.engine.start();
    assert_eq!(
        application.get("1", sandbox),
        Err(PortError::OutcomeUnknown)
    );
    assert!(
        send(&router, "GET", "/v2/sandboxes", ALICE, None)
            .await
            .0
            .is_server_error()
    );
    assert_eq!(
        application.resolve_data_route(token),
        Err(PortError::OutcomeUnknown)
    );
    application.recover_authority().unwrap();
    assert_eq!(
        application.get("1", sandbox).unwrap().info.sandbox_id,
        sandbox
    );
    assert!(application.resolve_data_route(token).is_ok());
    assert_eq!(fixture.preparations(), 1);
    assert_eq!(
        send(
            &router,
            "DELETE",
            &format!("/sandboxes/{sandbox}"),
            ALICE,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    drop((router, application));
    fixture.stop().await;
}
