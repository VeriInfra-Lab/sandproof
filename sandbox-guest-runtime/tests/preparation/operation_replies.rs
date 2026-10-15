use super::*;
use axum::http::Method;
use sandproof_guest_runtime::gzip::{DecodeLimits, Decoder};
use sandproof_guest_service::command_requests::http::{Rpc, reply};

async fn selected(pool: &CommandRequests, fixture: &Fixture, rpc: Rpc, value: Value) -> Ready {
    let mut headers =
        HeaderMap::from_iter([("x-access-token".parse().unwrap(), "token".parse().unwrap())]);
    headers.insert(
        "content-type",
        if rpc.is_stream() {
            "application/connect+json; charset=utf-8"
        } else {
            "application/json; charset=utf-8"
        }
        .parse()
        .unwrap(),
    );
    headers.insert(
        if rpc.is_stream() {
            "connect-accept-encoding"
        } else {
            "accept-encoding"
        },
        "gzip".parse().unwrap(),
    );
    let accounts = AccountPool::new(NoLookup, 1, Duration::from_secs(5)).unwrap();
    let selected = pool
        .admit()
        .unwrap()
        .authenticate_http(&fixture.session, &Method::POST, headers)
        .await
        .unwrap()
        .prepare_http(
            &Method::POST,
            &accounts,
            ContextLimits {
                bytes: 4096,
                env_entries: 32,
                arguments: 32,
                username_bytes: 64,
                authorization_bytes: 256,
            },
        )
        .await
        .unwrap()
        .select_json(&Method::POST, rpc)
        .unwrap();
    let mut body = serde_json::to_vec(&value).unwrap();
    if rpc.is_stream() {
        let mut frame = vec![0];
        frame.extend(u32::try_from(body.len()).unwrap().to_be_bytes());
        frame.append(&mut body);
        body = frame;
    }
    selected.decode(Body::from(body)).await.unwrap()
}
fn decode(bytes: &[u8]) -> Value {
    assert_eq!(bytes[0], 3);
    assert_eq!(
        bytes.len(),
        u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize + 5
    );
    let mut decoder = Decoder::new(
        &bytes[5..],
        DecodeLimits {
            encoded_bytes: 4096,
            decoded_bytes: 4096,
            chunk_bytes: 4096,
            header_bytes: 128,
            members: 1,
        },
    )
    .unwrap();
    let json = decoder.next_chunk().unwrap().unwrap();
    assert!(decoder.next_chunk().unwrap().is_none());
    serde_json::from_slice::<Value>(&json).unwrap()["error"].clone()
}
async fn rejected(
    pool: &CommandRequests,
    fixture: &Fixture,
    registry: &PidRegistry,
    rpc: Rpc,
    value: Value,
) -> Value {
    let ready = selected(pool, fixture, rpc, value).await;
    let deadline = ready.deadline();
    let failure = ready.execute_pid(registry, response()).await.err().unwrap();
    assert_eq!(failure.deadline(), deadline);
    assert!(pool.admit().is_err());
    timeout(
        Duration::from_secs(1),
        fixture.session.initialize(br#"{"accessToken":"token"}"#),
    )
    .await
    .unwrap()
    .unwrap();
    let response = failure
        .into_operation_response(reply::Limits {
            wire_bytes: 4096,
            chunk_bytes: 4096,
        })
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(
        response.headers()["content-type"],
        if rpc.is_stream() {
            "application/connect+json; charset=utf-8"
        } else {
            "application/json"
        }
    );
    let mut body = response.into_body().into_data_stream();
    let bytes = frame(&mut body).await;
    let json: Value = if rpc.is_stream() {
        decode(&bytes)
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    assert_eq!(
        status,
        if rpc.is_stream() {
            200
        } else if json["code"] == "not_found" {
            404
        } else {
            501
        }
    );
    let copy = bytes.clone();
    drop(body);
    drop(bytes);
    assert!(pool.admit().is_err());
    drop(copy);
    drop(pool.admit().unwrap());
    json
}

async fn check_missing(pool: &CommandRequests, fixture: &Fixture, registry: &PidRegistry) {
    for rpc in [
        Rpc::Connect,
        Rpc::SendInput,
        Rpc::SendSignal,
        Rpc::CloseStdin,
        Rpc::Update,
    ] {
        for selector in [json!({}), json!({"process":null}), json!({"process":{}})] {
            let error = rejected(pool, fixture, registry, rpc, selector).await;
            assert_eq!(
                error,
                json!({"code":"unimplemented","message":"invalid input type *process.ProcessSelector"})
            );
        }
        // Unknown pid wins even over absent input or unspecified signal.
        let error = rejected(
            pool,
            fixture,
            registry,
            rpc,
            json!({"process":{"pid":u32::MAX}}),
        )
        .await;
        assert_eq!(
            error,
            json!({"code":"not_found","message":format!("process with pid {} not found",u32::MAX)})
        );
    }
}

#[tokio::test]
async fn original_pid_lookup_precedes_parameter_rejections_and_valid_control_still_works() {
    let fixture = Fixture::new().await;
    let registry = registry();
    let (command, _guard) = launch(&fixture, &registry, false).await;
    let pid = command.0.identity().pid;
    let pool = requests(1);
    check_missing(&pool, &fixture, &registry).await;
    for input in [
        json!({"process":{"pid":pid}}),
        json!({"process":{"pid":pid},"input":{}}),
    ] {
        assert_eq!(
            rejected(&pool, &fixture, &registry, Rpc::SendInput, input).await,
            json!({"code":"unimplemented","message":"invalid input type <nil>"})
        );
    }
    for signal in [
        json!(0),
        json!("unrecognized"),
        json!(i32::MIN),
        json!(i32::MAX),
    ] {
        let expected = if signal == json!(0) || signal.is_string() {
            "SIGNAL_UNSPECIFIED".to_owned()
        } else {
            signal.to_string()
        };
        let error = rejected(
            &pool,
            &fixture,
            &registry,
            Rpc::SendSignal,
            json!({"process":{"pid":pid},"signal":signal}),
        )
        .await;
        assert_eq!(
            error,
            json!({"code":"unimplemented","message":format!("invalid signal: {expected}")})
        );
        assert!(command.0.status().completion.is_none());
    }
    empty(
        dispatch(
            &pool,
            &fixture,
            &registry,
            "SendSignal",
            json!({"process":{"pid":pid},"signal":"SIGNAL_SIGTERM"}),
        )
        .await,
    )
    .await;
    assert_eq!(command.0.wait().await.unwrap().signal, Some(15));
    let error = rejected(
        &pool,
        &fixture,
        &registry,
        Rpc::SendInput,
        json!({"process":{"pid":pid}}),
    )
    .await;
    assert_eq!(error["code"], "not_found");
    // Retained Connect still succeeds; ordinary controls cannot use that exit.
    let mut retained = selected(
        &pool,
        &fixture,
        Rpc::Connect,
        json!({"process":{"pid":pid}}),
    )
    .await
    .execute_pid(&registry, response())
    .await
    .unwrap()
    .into_body()
    .into_data_stream();
    assert_eq!(
        json_frame(&frame(&mut retained).await, 0)["event"]["start"]["pid"],
        pid
    );
    drop(retained);
    clean(&fixture, &registry, command).await;
    let error = rejected(
        &pool,
        &fixture,
        &registry,
        Rpc::Connect,
        json!({"process":{"pid":pid}}),
    )
    .await;
    assert_eq!(error["code"], "not_found");
}

#[tokio::test]
async fn operation_sender_does_not_classify_tag_or_unselected_component_failure() {
    let fixture = Fixture::new().await;
    let registry = registry();
    let pool = requests(1);
    for tagged in [false, true] {
        let ready = if tagged {
            selected(
                &pool,
                &fixture,
                Rpc::Connect,
                json!({"process":{"tag":"pending"}}),
            )
            .await
        } else {
            ready(
                &pool,
                &fixture,
                "Connect",
                json!({"process":{"pid":123}}),
                Instant::now() + Duration::from_secs(30),
            )
            .await
        };
        let failure = ready
            .execute_pid(&registry, response())
            .await
            .err()
            .unwrap();
        let error = failure.error();
        let failure = failure
            .into_operation_response(reply::Limits {
                wire_bytes: 4096,
                chunk_bytes: 4096,
            })
            .await
            .unwrap_err();
        assert_eq!(failure.error(), reply::Error::UnclassifiedFault);
        assert_eq!(failure.request_error(), error);
        assert!(pool.admit().is_err());
        drop(failure);
        drop(pool.admit().unwrap());
    }
}
