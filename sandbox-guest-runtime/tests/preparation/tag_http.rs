use super::*;
use sandproof_guest_service::command::RegistryLimits;

fn payload(tag: &str, marker: &str) -> Value {
    json!({"tag":tag, "stdin":true, "process":{"cmd":"/bin/sh",
        "args":["-c",format!("printf {marker} >> effect; printf ready; exec /bin/cat")]}})
}

async fn begin(
    http: &Http,
    registry: &PidRegistry,
    tag: &str,
    marker: &str,
) -> (Stop, BodyDataStream) {
    let mut body = http
        .handle(request("Start", &payload(tag, marker)))
        .await
        .unwrap()
        .into_body()
        .into_data_stream();
    let command = started(&mut body, registry).await;
    data(&mut body, "stdout", b"ready").await;
    assert_eq!(
        registry.live_tag(tag).unwrap().identity(),
        command.0.identity()
    );
    (command, body)
}

async fn unary(http: &Http, method: &str, payload: &Value) {
    let reply = http.handle(request(method, payload)).await.unwrap();
    assert_eq!(reply.status(), StatusCode::OK);
    assert_eq!(bytes(reply).await, "{}");
}

async fn duplicate(http: &Http, fixture: &Fixture, tag: &str) {
    let claims = fixture.directory.path().join("pid-claims.bin");
    let before = std::fs::read(&claims).unwrap();
    let gates = fixture.gate_count();
    let mut denied = request("Start", &payload(tag, "forbidden"));
    denied
        .headers_mut()
        .insert("x-access-token", "wrong".parse().unwrap());
    unread(&mut denied);
    assert_eq!(
        http.handle(denied).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let response = http
        .handle(request("Start", &payload(tag, "forbidden")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_frame(&bytes(response).await, 2),
        json!({"error":{
            "code":"already_exists", "message":format!("process with tag {tag} already exists")
        }})
    );
    assert_eq!(std::fs::read(claims).unwrap(), before); // no new PID claim or worker
    assert_eq!(fixture.gate_count(), gates);
    assert_eq!(
        std::fs::read(fixture.directory.path().join("effect")).unwrap(),
        b"once"
    );
}

async fn exchange_and_finish(
    http: &Http,
    registry: &PidRegistry,
    tag: &str,
    pid: u32,
    body: &mut BodyDataStream,
) {
    let list: Value = serde_json::from_slice(
        &bytes(http.handle(request("List", &json!({}))).await.unwrap()).await,
    )
    .unwrap();
    assert_eq!(list["processes"][0]["tag"], tag);
    assert_eq!(list["processes"][0]["pid"], pid);
    let mut connected = http
        .handle(request("Connect", &json!({"process":{"tag":tag}})))
        .await
        .unwrap()
        .into_body()
        .into_data_stream();
    assert_eq!(
        json_frame(&frame(&mut connected).await, 0),
        json!({"event":{"start":{"pid":pid}}})
    );
    let input: Vec<u8> = (0..=255).collect();
    for value in [&[][..], input.as_slice()] {
        unary(
            http,
            "SendInput",
            &json!({"process":{"tag":tag},"input":{"stdin":STANDARD.encode(value)}}),
        )
        .await;
    }
    data(body, "stdout", &input).await;
    data(&mut connected, "stdout", &input).await;
    unary(http, "Update", &json!({"process":{"tag":tag}})).await;
    unary(http, "CloseStdin", &json!({"process":{"tag":tag}})).await;
    end(body).await;
    end(&mut connected).await;
    drop(connected);
    assert!(matches!(
        registry.live_tag(tag),
        Err(RegistryError::NotFound)
    ));
    let mut retained = http
        .handle(request("Connect", &json!({"process":{"tag":tag}})))
        .await
        .unwrap()
        .into_body()
        .into_data_stream();
    assert_eq!(
        json_frame(&frame(&mut retained).await, 0),
        json!({"event":{"start":{"pid":pid}}})
    );
    end(&mut retained).await; // no replayed stdout
}

async fn released_pair(fixture: &Fixture, registry: &PidRegistry) {
    timeout(Duration::from_secs(5), async {
        loop {
            if registry.available_slots() == 2 {
                let first = fixture.owners.reserve();
                let second = fixture.owners.reserve();
                if first.is_ok() && second.is_ok() {
                    return;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.gate_count(), 0);
}

#[tokio::test]
async fn tagged_http_lifecycle_preserves_pid_identity_and_reuses_only_after_terminal() {
    for tag in ["", "name\"\\<>&中\n"] {
        let mut fixture = fixture().await;
        fixture.owners = CommandOwners::new(OwnerLimits {
            records: 2,
            subscribers: 2,
            queued_chunks: 8,
            output_bytes: 8192,
            input_chunk_bytes: 8192,
        })
        .unwrap();
        let registry = PidRegistry::new(
            binding(),
            1,
            RegistryLimits {
                records: 2,
                metadata_bytes: 4096,
                retention: Duration::from_secs(30),
            },
        )
        .unwrap();
        let ordinary = requests(3);
        let control = requests(1);
        let http = http(&fixture, &registry, &ordinary, &control);
        let (original, mut body) = begin(&http, &registry, tag, "once").await;
        let pid = original.0.identity().pid;
        duplicate(&http, &fixture, tag).await;
        let response = http
            .handle(request(
                "SendSignal",
                &json!({"process":{"tag":pid.to_string()}}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND); // a numeric string is still a tag
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes(response).await).unwrap(),
            json!({"code":"not_found", "message":format!("process with tag {pid} not found")})
        );
        exchange_and_finish(&http, &registry, tag, pid, &mut body).await;
        drop(body);
        let (replacement, mut body) = begin(&http, &registry, tag, "twice").await;
        let next = replacement.0.identity();
        assert_ne!(next.pid, pid);
        assert!(matches!(
            registry.connectable(pid),
            Err(RegistryError::NotFound)
        ));
        assert_eq!(original.0.wait().await.unwrap().code, 0);
        unary(
            &http,
            "SendSignal",
            &json!({"process":{"tag":tag},"signal":"SIGNAL_SIGKILL"}),
        )
        .await;
        assert_eq!(
            json_frame(&frame(&mut body).await, 0),
            json!({"event":{"end":{
                "exitCode":-1,"status":"signal: killed","error":"signal: killed"
            }}})
        );
        assert_eq!(json_frame(&frame(&mut body).await, 2), json!({}));
        drop(body);
        replacement.0.wait().await.unwrap();
        expire_retention(
            &registry,
            next,
            replacement.0.status().completed_at.unwrap(),
        )
        .await;
        assert!(matches!(
            registry.connectable_tag(tag),
            Err(RegistryError::NotFound)
        ));
        drop((original, replacement));
        released_pair(&fixture, &registry).await;
        assert_eq!(
            std::fs::read(fixture.directory.path().join("effect")).unwrap(),
            b"oncetwice"
        );
        let slots = [
            ordinary.admit().unwrap(),
            ordinary.admit().unwrap(),
            ordinary.admit().unwrap(),
        ];
        drop(slots);
        assert!(control.admit().is_ok());
    }
}

#[tokio::test]
async fn tagged_pty_update_and_input_use_the_original_terminal() {
    let fixture = fixture().await;
    let registry = registry();
    let ordinary = requests(2);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    let tag = "123";
    let mut body = http.handle(request("Start", &json!({"tag":tag,"process":{
        "cmd":"/bin/sh","args":["-c","stty -echo; stty size; read value; stty size; printf 'got:%s' \"$value\""]
    },"pty":{"size":{"cols":80,"rows":24}}}))).await.unwrap().into_body().into_data_stream();
    let command = started(&mut body, &registry).await;
    data(&mut body, "pty", b"24 80\r\n").await;
    unary(
        &http,
        "Update",
        &json!({"process":{"tag":tag},"pty":{"size":{"cols":90,"rows":31}}}),
    )
    .await;
    unary(
        &http,
        "SendInput",
        &json!({"process":{"tag":tag},"input":{"pty":STANDARD.encode(b"go\n")}}),
    )
    .await;
    data(&mut body, "pty", b"31 90\r\ngot:go").await;
    end(&mut body).await;
    drop(body);
    clean(&fixture, &registry, command).await;
    assert!(matches!(
        registry.connectable_tag(tag),
        Err(RegistryError::NotFound)
    ));
    assert!(ordinary.admit().is_ok());
    assert!(control.admit().is_ok());
}
