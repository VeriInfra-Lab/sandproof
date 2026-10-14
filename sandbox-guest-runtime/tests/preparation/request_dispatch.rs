//! Real PID registry/worker composition; selected identity JSON, not a public
//! HTTP router or an SDK-versus-original-envd endpoint comparison.
use super::*;
use axum::{
    body::{Body, BodyDataStream, Bytes},
    http::HeaderMap,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use sandproof_guest_runtime::{
    PtySize,
    accounts::{Account, AccountError},
};
use sandproof_guest_service::{
    AccountPool, AccountSource, ContextLimits,
    command::{CommandBodyLimits, DeadlineAction},
    command_requests::{self, CommandRequests, Error, Ready},
    file_upload::HeaderLimits,
    upload_body::HttpBodyLimits,
};
use serde_json::{Value, json};

struct NoLookup;
impl AccountSource for NoLookup {
    fn lookup(&self, _: &str) -> Result<Account, AccountError> {
        panic!("non-Start default account lookup");
    }
}
fn requests(in_flight: usize) -> CommandRequests {
    CommandRequests::new(command_requests::Limits {
        in_flight,
        timeout: Duration::from_secs(30),
        headers: HeaderLimits {
            bytes: 256,
            fields: 8,
        },
        body: HttpBodyLimits {
            total_bytes: 16384,
            frame_bytes: 16384,
            frames: 8,
            trailer_bytes: 64,
            trailer_fields: 2,
        },
    })
    .unwrap()
}
fn response() -> CommandBodyLimits {
    CommandBodyLimits {
        message_bytes: 4096,
        keepalive: Duration::from_secs(10),
    }
}
async fn ready(
    pool: &CommandRequests,
    fixture: &Fixture,
    method: &'static str,
    payload: Value,
    deadline: Instant,
) -> Ready {
    let headers =
        HeaderMap::from_iter([("x-access-token".parse().unwrap(), "token".parse().unwrap())]);
    let accounts = AccountPool::new(NoLookup, 1, Duration::from_secs(5)).unwrap();
    let prepared = pool
        .admit()
        .unwrap()
        .authenticate(&fixture.session, headers)
        .await
        .unwrap()
        .prepare(
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
        .unwrap();
    let mut bytes = serde_json::to_vec(&payload).unwrap();
    if matches!(method, "Start" | "Connect") {
        let mut frame = vec![0];
        frame.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_be_bytes());
        frame.append(&mut bytes);
        bytes = frame;
    }
    prepared
        .decode(method, Body::from(bytes), deadline)
        .await
        .unwrap()
}
async fn dispatch(
    pool: &CommandRequests,
    fixture: &Fixture,
    registry: &PidRegistry,
    method: &'static str,
    payload: Value,
) -> command_requests::Reply {
    ready(
        pool,
        fixture,
        method,
        payload,
        Instant::now() + Duration::from_secs(30),
    )
    .await
    .execute_pid(registry, response())
    .await
    .unwrap()
}
async fn frame(body: &mut BodyDataStream) -> Bytes {
    timeout(Duration::from_secs(5), body.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}
fn json_frame(bytes: &[u8], flags: u8) -> Value {
    assert_eq!(bytes[0], flags);
    assert_eq!(
        u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize,
        bytes.len() - 5
    );
    serde_json::from_slice(&bytes[5..]).unwrap()
}
async fn empty(reply: command_requests::Reply) {
    assert!(!reply.is_stream());
    assert_eq!(
        axum::body::to_bytes(reply.into_body(), 2).await.unwrap(),
        "{}"
    );
}
async fn launch(fixture: &Fixture, registry: &PidRegistry, pty: bool) -> (Stop, KernelGuard) {
    let mut spec = fixture.spec();
    if pty {
        spec.launch.pty = Some(PtySize { cols: 80, rows: 24 });
        spec.launch.args = vec![
            "-c".into(),
            "stty -echo; printf ready; read value; stty size".into(),
        ];
    }
    let metadata = ProcessConfig {
        cmd: spec.launch.program.clone(),
        args: spec.launch.args.clone(),
        envs: spec.launch.env.clone(),
        cwd: Some(spec.launch.cwd.to_str().unwrap().into()),
    };
    let ticket = fixture
        .pending(spec)
        .await
        .register_pid(PidRegistrar::new(store(fixture)), binding())
        .await
        .unwrap();
    let guard = kernel_guard(ticket.identity());
    let (command, mut initial) = registry
        .reserve(&metadata, None)
        .unwrap()
        .publish(ticket)
        .unwrap()
        .started()
        .await
        .unwrap();
    let event = initial.next().await.unwrap().unwrap();
    let OutputEvent::Data(data) = event else {
        panic!("missing ready");
    };
    assert_eq!(data.bytes(), b"ready");
    drop(data);
    drop(initial);
    (Stop(command), guard)
}
async fn clean(fixture: &Fixture, registry: &PidRegistry, command: Stop) {
    command.0.wait().await.unwrap();
    expire_retention(
        registry,
        command.0.identity(),
        command.0.status().completed_at.unwrap(),
    )
    .await;
    registry_capacity(registry).await;
    drop(command);
    capacity_restored(&fixture.owners).await;
    assert_eq!(fixture.gate_count(), 0);
}

#[tokio::test]
async fn authenticated_pid_requests_list_write_close_and_reconnect_to_original_exit() {
    let fixture = Fixture::new().await;
    let registry = registry();
    let (command, _guard) = launch(&fixture, &registry, false).await;
    let pid = command.0.identity().pid;
    let pool = requests(2);
    let list = dispatch(&pool, &fixture, &registry, "List", json!({})).await;
    let list: Value =
        serde_json::from_slice(&axum::body::to_bytes(list.into_body(), 4096).await.unwrap())
            .unwrap();
    assert_eq!(list["processes"][0]["pid"], pid);
    let mut body = dispatch(
        &pool,
        &fixture,
        &registry,
        "Connect",
        json!({"process":{"pid":pid}}),
    )
    .await
    .into_body()
    .into_data_stream();
    assert_eq!(
        json_frame(&frame(&mut body).await, 0),
        json!({"event":{"start":{"pid":pid}}})
    );
    let payload: Vec<u8> = (0..=255).collect();
    for bytes in [&[][..], &payload[..]] {
        empty(
            dispatch(
                &pool,
                &fixture,
                &registry,
                "SendInput",
                json!({"process":{"pid":pid},"input":{"stdin":STANDARD.encode(bytes)}}),
            )
            .await,
        )
        .await;
    }
    let mut output = Vec::new();
    while output.len() < payload.len() {
        let event = json_frame(&frame(&mut body).await, 0);
        output.extend(
            STANDARD
                .decode(event["event"]["data"]["stdout"].as_str().unwrap())
                .unwrap(),
        );
    }
    assert_eq!(output, payload);
    empty(
        dispatch(
            &pool,
            &fixture,
            &registry,
            "Update",
            json!({"process":{"pid":pid}}),
        )
        .await,
    )
    .await;
    empty(
        dispatch(
            &pool,
            &fixture,
            &registry,
            "CloseStdin",
            json!({"process":{"pid":pid}}),
        )
        .await,
    )
    .await;
    assert_eq!(
        json_frame(&frame(&mut body).await, 0),
        json!({"event":{"end":{"exited":true,"status":"exit status 0"}}})
    );
    assert_eq!(json_frame(&frame(&mut body).await, 2), json!({}));
    assert!(body.next().await.is_none());
    drop(body);
    assert_eq!(command.0.wait().await.unwrap().code, 0);
    let mut late = dispatch(
        &pool,
        &fixture,
        &registry,
        "Connect",
        json!({"process":{"pid":pid}}),
    )
    .await
    .into_body()
    .into_data_stream();
    assert_eq!(
        json_frame(&frame(&mut late).await, 0)["event"]["start"]["pid"],
        pid
    );
    assert_eq!(
        json_frame(&frame(&mut late).await, 0)["event"]["end"]["exited"],
        true
    );
    assert_eq!(json_frame(&frame(&mut late).await, 2), json!({}));
    drop(late);
    clean(&fixture, &registry, command).await;
}

#[tokio::test]
async fn request_deadline_and_denied_token_do_not_touch_the_original_process() {
    let fixture = Fixture::new().await;
    let registry = registry();
    let (command, _guard) = launch(&fixture, &registry, false).await;
    let pid = command.0.identity().pid;
    let pool = requests(1);
    let headers =
        HeaderMap::from_iter([("x-access-token".parse().unwrap(), "wrong".parse().unwrap())]);
    let failure = pool
        .admit()
        .unwrap()
        .authenticate(&fixture.session, headers)
        .await
        .err()
        .unwrap();
    assert_eq!(
        failure.error(),
        Error::Authorization(ServiceError::Unauthenticated)
    );
    drop(failure);
    let deadline = Instant::now() + Duration::from_secs(5);
    let expired = ready(
        &pool,
        &fixture,
        "SendSignal",
        json!({"process":{"pid":pid},"signal":"SIGNAL_SIGKILL"}),
        deadline,
    )
    .await;
    tokio::time::pause();
    tokio::time::advance(deadline.saturating_duration_since(Instant::now())).await;
    assert_eq!(
        expired
            .execute_pid(&registry, response())
            .await
            .err()
            .unwrap()
            .error(),
        Error::DeadlineExceeded
    );
    tokio::time::resume();
    assert!(command.0.status().completion.is_none());
    assert_eq!(command.0.status().deadline_action, None::<DeadlineAction>);
    let unknown = ready(
        &pool,
        &fixture,
        "SendSignal",
        json!({"process":{"pid":pid},"signal":"SIGKILL"}),
        Instant::now() + Duration::from_secs(30),
    )
    .await;
    assert_eq!(
        unknown
            .execute_pid(&registry, response())
            .await
            .err()
            .unwrap()
            .error(),
        Error::Protocol(sandproof_guest_runtime::protocol::ProtocolError::Unimplemented)
    );
    assert!(command.0.status().completion.is_none());
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
    clean(&fixture, &registry, command).await;
}

#[tokio::test]
async fn pty_update_and_input_use_the_registered_terminal() {
    let fixture = Fixture::new().await;
    let registry = registry();
    let (command, _guard) = launch(&fixture, &registry, true).await;
    let pid = command.0.identity().pid;
    let pool = requests(2);
    let mut body = dispatch(
        &pool,
        &fixture,
        &registry,
        "Connect",
        json!({"process":{"pid":pid}}),
    )
    .await
    .into_body()
    .into_data_stream();
    drop(frame(&mut body).await);
    empty(
        dispatch(
            &pool,
            &fixture,
            &registry,
            "Update",
            json!({"process":{"pid":pid},"pty":{"size":{"rows":65566,"cols":65636}}}),
        )
        .await,
    )
    .await;
    empty(
        dispatch(
            &pool,
            &fixture,
            &registry,
            "SendInput",
            json!({"process":{"pid":pid},"input":{"pty":STANDARD.encode(b"continue\n")}}),
        )
        .await,
    )
    .await;
    let mut output = Vec::new();
    loop {
        let event = json_frame(&frame(&mut body).await, 0);
        if event["event"].get("end").is_some() {
            break;
        }
        output.extend(
            STANDARD
                .decode(event["event"]["data"]["pty"].as_str().unwrap())
                .unwrap(),
        );
    }
    assert_eq!(output, b"30 100\r\n");
    drop(body);
    clean(&fixture, &registry, command).await;
}
