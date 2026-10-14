//! Shared HTTP assembly with real accounts, registry and original gated worker.
use super::*;
use axum::http::{Method, Request as HttpRequest, StatusCode};
use sandproof_guest_service::command_http::{
    CommandHttp, Failure, Prepared, Processes, RequestPools, ResponseLimits,
};

#[path = "channel_http.rs"]
mod channels;
#[path = "closed_stdin_http.rs"]
mod closed_stdin;
#[path = "cwd_http.rs"]
mod cwd;
#[path = "end_stream_http.rs"]
mod end_stream;
#[path = "framing_http.rs"]
mod framing;
#[path = "guest_http.rs"]
mod guest;
#[path = "json_http.rs"]
mod json_errors;
#[path = "launch_http.rs"]
mod launch;

type Http = CommandHttp<MetadataFixture, CurrentAccount>;
fn processes(fixture: &Fixture, registry: &PidRegistry, registrar: &PidRegistrar) -> Processes {
    Processes {
        owners: fixture.owners.clone(),
        registry: registry.clone(),
        registrar: registrar.clone(),
        binding: binding(),
        session: 1,
        worker: WORKER.into(),
        private_directory: fixture.directory.path().join("gates"),
        process_limits: fixture.spec().process_limits,
        admission_limits: limits(),
    }
}
fn responses() -> ResponseLimits {
    ResponseLimits {
        message: response(),
        http: command_requests::http::reply::Limits {
            wire_bytes: 8192,
            chunk_bytes: 8192,
        },
        account_headers: 4096,
    }
}
fn http(
    fixture: &Fixture,
    registry: &PidRegistry,
    ordinary: &CommandRequests,
    control: &CommandRequests,
) -> Http {
    CommandHttp::new(
        fixture.session.clone(),
        RequestPools {
            ordinary: ordinary.clone(),
            control: control.clone(),
        },
        accounts(fixture),
        settings(),
        processes(fixture, registry, &PidRegistrar::new(store(fixture))),
        responses(),
    )
    .unwrap()
}
fn request(method: &str, payload: &Value) -> HttpRequest<Body> {
    let stream = matches!(method, "Start" | "Connect");
    let mut bytes = serde_json::to_vec(payload).unwrap();
    if stream {
        let mut prefix = vec![0];
        prefix.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_be_bytes());
        prefix.append(&mut bytes);
        bytes = prefix;
    }
    HttpRequest::builder()
        .method(Method::POST)
        .uri(format!("/process.Process/{method}"))
        .header("x-access-token", "token")
        .header(
            "content-type",
            if stream {
                "application/connect+json"
            } else {
                "application/json"
            },
        )
        .body(Body::from(bytes))
        .unwrap()
}
fn unread(request: &mut HttpRequest<Body>) {
    *request.body_mut() = Body::from_stream(futures_util::stream::poll_fn(|_| {
        panic!("rejected request polled its body");
        #[allow(unreachable_code)]
        Poll::<Option<Result<Bytes, std::io::Error>>>::Ready(None)
    }));
}
fn full(pool: &CommandRequests) {
    assert!(matches!(
        pool.admit(),
        Err(command_requests::AdmissionError::ResourceExhausted)
    ));
}
async fn bytes(response: axum::response::Response) -> Bytes {
    let mut body = response.into_body().into_data_stream();
    // Each bounded unary response fits one configured chunk. Do not collect
    // while retaining that chunk: the next poll waits for its consumption ack.
    timeout(Duration::from_secs(5), body.next())
        .await
        .unwrap()
        .map_or_else(Bytes::new, Result::unwrap)
}

async fn rotates(fixture: &Fixture) {
    timeout(
        Duration::from_secs(5),
        fixture
            .session
            .initialize(br#"{"accessToken":"token","defaultUser":"fixture","defaultWorkdir":"."}"#),
    )
    .await
    .unwrap()
    .unwrap();
}

#[tokio::test]
async fn shared_http_keeps_control_admission_available_while_start_stream_owns_ordinary_capacity() {
    let fixture = fixture().await;
    let registry = registry();
    let ordinary = requests(1);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    let mut body = http
        .handle(request(
            "Start",
            &json!({"process":{
        "cmd":"/bin/sh", "args":["-c","printf once >> effect; printf ready; exec /bin/cat"]
    },"stdin":true}),
        ))
        .await
        .unwrap()
        .into_body()
        .into_data_stream();
    let command = started(&mut body, &registry).await;
    assert_eq!(command.0.identity().command, 1);
    let pid = command.0.identity().pid;
    data(&mut body, "stdout", b"ready").await;
    full(&ordinary);
    rotates(&fixture).await; // Start's long response no longer owns auth.
    for method in ["List", "SendInput", "Connect", "Start"] {
        let mut rejected = request(method, &json!({}));
        unread(&mut rejected);
        assert!(matches!(
            http.clone().handle(rejected).await,
            Err(Failure::Admission(
                command_requests::AdmissionError::ResourceExhausted
            ))
        ));
    }
    let reply = http
        .clone()
        .handle(request("Update", &json!({"process":{"pid":pid}})))
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::OK);
    let first = bytes(reply).await;
    assert_eq!(first, "{}");
    let last = first.clone();
    drop(first);
    full(&control); // Completed HTTP payload clones retain the original slot.
    assert!(matches!(
        http.handle(request(
            "SendSignal",
            &json!({"process":{"pid":pid},"signal":"SIGNAL_KILL"})
        ))
        .await,
        Err(Failure::Admission(
            command_requests::AdmissionError::ResourceExhausted
        ))
    ));
    drop(last);
    assert_eq!(
        bytes(
            http.handle(request("CloseStdin", &json!({"process":{"pid":pid}})))
                .await
                .unwrap()
        )
        .await,
        "{}"
    );
    end(&mut body).await;
    drop(body);
    clean(&fixture, &registry, command).await;
    assert!(ordinary.admit().is_ok());
    assert!(control.admit().is_ok());
    // A second Start after the original record retires receives a new internal
    // ID from the same assembly, including when dispatched through a clone.
    let mut body = http
        .clone()
        .handle(request(
            "Start",
            &json!({"process":{
        "cmd":"/bin/sh", "args":["-c","printf twice >> effect; printf ready; exec /bin/cat"]
    },"stdin":true}),
        ))
        .await
        .unwrap()
        .into_body()
        .into_data_stream();
    let command = started(&mut body, &registry).await;
    assert_eq!(command.0.identity().command, 2);
    data(&mut body, "stdout", b"ready").await;
    let pid = command.0.identity().pid;
    assert_eq!(
        bytes(
            http.handle(request("CloseStdin", &json!({"process":{"pid":pid}})))
                .await
                .unwrap()
        )
        .await,
        "{}"
    );
    end(&mut body).await;
    drop(body);
    clean(&fixture, &registry, command).await;
    assert_eq!(
        std::fs::read(fixture.directory.path().join("effect")).unwrap(),
        b"oncetwice"
    );
}

#[tokio::test]
async fn shared_http_preflight_order_and_owned_failures_never_dispatch_or_refresh_capacity() {
    let fixture = fixture().await;
    let registry = registry();
    let ordinary = requests(1);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    for (token, basic, method, status, text) in [
        (
            "wrong",
            "missing",
            Method::GET,
            StatusCode::UNAUTHORIZED,
            "",
        ),
        (
            "token",
            "missing",
            Method::GET,
            StatusCode::UNAUTHORIZED,
            "invalid username: 'missing'",
        ),
        (
            "token",
            "fixture",
            Method::GET,
            StatusCode::METHOD_NOT_ALLOWED,
            "",
        ),
        (
            "token",
            "fixture",
            Method::POST,
            StatusCode::NOT_IMPLEMENTED,
            "unknown compression",
        ),
    ] {
        let mut rejected = request("List", &json!({}));
        *rejected.method_mut() = method;
        let headers = rejected.headers_mut();
        headers.insert("x-access-token", token.parse().unwrap());
        headers.insert(
            "authorization",
            format!("Basic {}", STANDARD.encode(format!("{basic}:")))
                .parse()
                .unwrap(),
        );
        headers.insert("content-encoding", "unknown".parse().unwrap());
        headers.insert("connect-protocol-version", "wrong".parse().unwrap());
        unread(&mut rejected);
        let reply = http.handle(rejected).await.unwrap();
        assert_eq!(reply.status(), status);
        full(&ordinary);
        let payload = bytes(reply).await;
        assert!(std::str::from_utf8(&payload).unwrap().contains(text));
        if !payload.is_empty() {
            full(&ordinary);
        }
        rotates(&fixture).await;
        drop(payload);
        assert!(ordinary.admit().is_ok());
    }
    let mut invalid = request("List", &json!({}));
    invalid
        .headers_mut()
        .insert("content-type", "application/proto".parse().unwrap());
    unread(&mut invalid);
    let failure = http.handle(invalid).await.unwrap_err();
    assert!(matches!(&failure, Failure::Selection(f) if
        f.selection_error() == &command_requests::http::Error::ContentType
        && f.error() == command_requests::http::reply::Error::UnclassifiedFault));
    full(&ordinary);
    rotates(&fixture).await;
    drop(failure);
    assert!(ordinary.admit().is_ok());

    let Prepared::Ready(ready) = http.prepare(request("List", &json!({}))).await.unwrap() else {
        panic!("expected original decoded request");
    };
    assert_eq!(
        ready.request(),
        &sandproof_guest_runtime::protocol::Request::List
    );
    let deadline = ready.deadline();
    tokio::time::pause();
    tokio::time::advance(
        deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
    )
    .await;
    let failure = ready.execute().await.unwrap_err();
    assert_eq!(failure.deadline(), Some(deadline));
    assert!(
        matches!(&failure, Failure::Operation(f) if f.request_error() == Error::DeadlineExceeded)
    );
    full(&ordinary);
    tokio::time::resume();
    rotates(&fixture).await;
    drop(failure);
    assert!(ordinary.admit().is_ok());
    fixture.assert_no_effect();
    assert_eq!(fixture.gate_count(), 0);
    assert!(fixture.owners.reserve().is_ok());
}

#[tokio::test]
async fn shared_http_rejects_aliased_pools_and_wrong_registry_session_before_any_work() {
    let fixture = fixture().await;
    let registry = registry();
    let registrar = PidRegistrar::new(store(&fixture));
    let pool = requests(1);
    for case in 0..4 {
        let mut processes = processes(&fixture, &registry, &registrar);
        let mut response = responses();
        match case {
            1 => processes.session = 2,
            2 => processes.binding = Binding::new([0x42; 32]).unwrap(),
            3 => response.message.keepalive = Duration::ZERO,
            _ => (),
        }
        let result = CommandHttp::new(
            fixture.session.clone(),
            RequestPools {
                ordinary: pool.clone(),
                control: if case == 0 { pool.clone() } else { requests(1) },
            },
            accounts(&fixture),
            settings(),
            processes,
            response,
        );
        assert!(matches!(
            result,
            Err(sandproof_guest_service::command_http::InvalidConfiguration)
        ));
        assert!(pool.admit().is_ok());
        assert!(fixture.owners.reserve().is_ok());
        fixture.assert_no_effect();
        assert_eq!(fixture.gate_count(), 0);
    }
}

#[tokio::test]
async fn shared_http_preserves_default_account_and_gzip_replies_without_inventing_tag_or_route_policy()
 {
    let fixture = fixture().await;
    let registry = registry();
    let ordinary = requests(1);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    fixture
        .session
        .initialize(br#"{"accessToken":"token","defaultUser":"missing"}"#)
        .await
        .unwrap();
    let reply = http
        .handle(request("Start", &json!({"process":{"cmd":"/bin/true"}})))
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::OK);
    let payload = bytes(reply).await;
    assert_eq!(
        json_frame(&payload, 2),
        json!({"error":{
            "code":"unauthenticated", "message":"invalid default user: 'missing'"
        }})
    );
    full(&ordinary);
    rotates(&fixture).await;
    drop(payload);

    let mut compressed = request("List", &json!({}));
    compressed
        .headers_mut()
        .insert("content-encoding", "gzip".parse().unwrap());
    *compressed.body_mut() = Body::from(vec![0x1f, 0x8b]);
    let reply = http.handle(compressed).await.unwrap();
    assert_eq!(reply.status(), StatusCode::BAD_REQUEST);
    let payload = bytes(reply).await;
    assert_eq!(
        serde_json::from_slice::<Value>(&payload).unwrap(),
        json!({
            "code":"invalid_argument", "message":"get decompressor: unexpected EOF"
        })
    );
    full(&ordinary);
    drop(payload);

    let failure = http
        .handle(request(
            "Start",
            &json!({"tag":"","process":{"cmd":"/bin/true"}}),
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(&failure, Failure::Start(f) if f.request_error() == Error::UnsupportedOperation
        && f.error() == command_requests::http::reply::Error::UnclassifiedFault)
    );
    full(&ordinary);
    rotates(&fixture).await;
    drop(failure);

    let mut unknown = request("List/", &json!({}));
    unread(&mut unknown);
    let failure = http.handle(unknown).await.unwrap_err();
    assert!(matches!(&failure, Failure::Route(_)));
    assert!(failure.deadline().is_some());
    full(&ordinary);
    rotates(&fixture).await;
    drop(failure);
    assert!(ordinary.admit().is_ok());
    assert!(control.admit().is_ok());
    assert!(fixture.owners.reserve().is_ok());
    assert_eq!(fixture.gate_count(), 0);
    fixture.assert_no_effect();
}
