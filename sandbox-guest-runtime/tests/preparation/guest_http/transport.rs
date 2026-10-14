use super::*;
use wire::Wire;

fn transport() -> listener::Listener {
    listener::Listener::new(listener::Limits {
        connections: 4,
        header_count: 32,
        buffer_bytes: 8192,
        header_timeout: Duration::from_secs(5),
        connection_timeout: Duration::from_secs(30),
    })
    .unwrap()
}

#[tokio::test]
async fn guest_listener_composes_actual_command_and_file_effects_and_preserves_control_capacity() {
    let fixture = fixture().await;
    let registry = registry();
    let components = parts(&fixture, &registry, &fixture.session);
    let pools = components.pools;
    let guest = GuestHttp::new(fixture.session.clone(), components.components).unwrap();
    let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let server = tokio::spawn(guest.clone().serve(transport(), socket, async {
        let _ = stopped.await;
    }));
    let initialized = Wire::open(address, init("token")).await;
    assert_eq!(initialized.status, 204);
    initialized.finish().await;
    let mut output = Wire::open(address,request("Start",&json!({"process":{
        "cmd":"/bin/sh","args":["-c","printf once >> effect; printf ready; exec /bin/cat"]},"stdin":true}))).await;
    assert_eq!(output.status, 200);
    let event = output.frame(0).await;
    let pid = u32::try_from(event["event"]["start"]["pid"].as_u64().unwrap()).unwrap();
    let command = Stop(registry.connectable(pid).unwrap());
    assert_eq!(command.0.identity().command, 1);
    assert_eq!(
        output.frame(0).await,
        json!({"event":{"data":{"stdout":STANDARD.encode(b"ready")}}})
    );
    full(&pools.ordinary);
    let health = Wire::open(address, at(Method::GET, "/health", Body::empty())).await;
    assert_eq!(health.status, 204);
    health.finish().await;
    let mut upload = at(Method::POST, "/files?path=data", Body::from("file-content"));
    upload
        .headers_mut()
        .insert("content-type", "application/octet-stream".parse().unwrap());
    let uploaded = Wire::open(address, upload).await;
    assert_eq!(uploaded.status, 200);
    assert_eq!(
        serde_json::from_slice::<Value>(&uploaded.body().await).unwrap()[0]["name"],
        "data"
    );
    let stat = Wire::open(
        address,
        at(
            Method::POST,
            "/filesystem.Filesystem/Stat",
            r#"{"path":"data"}"#,
        ),
    )
    .await;
    assert_eq!(stat.status, 200);
    assert_eq!(
        serde_json::from_slice::<Value>(&stat.body().await).unwrap()["entry"]["size"],
        "12"
    );
    let download = Wire::open(address, at(Method::GET, "/files?path=data", Body::empty())).await;
    assert_eq!(download.status, 200);
    assert_eq!(download.body().await, b"file-content");
    wire::rejected(address, "/process.Process/List", b"{}").await; // Ordinary pool is still occupied.
    full(&pools.ordinary);
    let closed = Wire::open(
        address,
        request("CloseStdin", &json!({"process":{"pid":pid}})),
    )
    .await;
    assert_eq!(closed.status, 200);
    assert_eq!(closed.body().await, b"{}");
    assert_eq!(
        output.frame(0).await,
        json!({"event":{"end":{"exited":true,"status":"exit status 0"}}})
    );
    assert_eq!(output.frame(2).await, json!({}));
    output.finish().await;
    stop.send(()).unwrap();
    let report = timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(report.failed, 1);
    assert_eq!(report.refused, 0);
    assert_eq!(report.expired, 0);
    assert_eq!(report.admitted, report.completed + report.failed);
    assert!(pools.ordinary.admit().is_ok());
    assert!(pools.control.admit().is_ok());
    clean(&fixture, &registry, command).await;
    assert_eq!(
        std::fs::read(fixture.directory.path().join("effect")).unwrap(),
        b"once"
    );
    assert_eq!(
        std::fs::read(fixture.directory.path().join("data")).unwrap(),
        b"file-content"
    );
}

#[tokio::test]
async fn guest_transport_rejects_unmapped_connections_without_poisoning_other_routes() {
    let fixture = fixture().await;
    let components = parts(&fixture, &registry(), &fixture.session);
    let pools = components.pools;
    let guest = GuestHttp::new(fixture.session.clone(), components.components).unwrap();
    let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let server = tokio::spawn(guest.serve(transport(), socket, async {
        let _ = stopped.await;
    }));
    for path in [
        "/unknown",
        "/process.Process/List/",
        "/filesystem.Filesystem/Stat/",
        "/process.Process/List",
        "/filesystem.Filesystem/Stat",
    ] {
        wire::rejected(address, path, b"{}").await;
        assert!(pools.ordinary.admit().is_ok());
        assert!(pools.rpc.admit().is_ok());
        let list = Wire::open(address, request("List", &json!({}))).await;
        assert_eq!(list.status, 200);
        assert_eq!(list.body().await, b"{}");
    }
    stop.send(()).unwrap();
    let report = timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(report.failed, 5);
    assert_eq!(report.completed, 5);
    assert_eq!(report.expired, 0);
    fixture.assert_no_effect();
    assert_eq!(fixture.gate_count(), 0);
}

#[tokio::test]
async fn guest_transport_cancellation_drops_subscription_but_retains_original_command() {
    let fixture = fixture().await;
    let registry = registry();
    let components = parts(&fixture, &registry, &fixture.session);
    let pools = components.pools;
    let guest = GuestHttp::new(fixture.session.clone(), components.components).unwrap();
    let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    let server = tokio::spawn(
        guest
            .clone()
            .serve(transport(), socket, std::future::pending()),
    );
    let mut output = Wire::open(address,request("Start",&json!({"process":{
        "cmd":"/bin/sh","args":["-c","printf once >> effect; printf ready; exec /bin/cat"]},"stdin":true}))).await;
    let pid = u32::try_from(
        output.frame(0).await["event"]["start"]["pid"]
            .as_u64()
            .unwrap(),
    )
    .unwrap();
    let command = Stop(registry.connectable(pid).unwrap());
    output.frame(0).await;
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
    drop(output);
    timeout(Duration::from_secs(5), async {
        while pools.ordinary.admit().is_err() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(registry.live(pid).is_ok());
    let mut reconnected = guest
        .handle(request("Connect", &json!({"process":{"pid":pid}})))
        .await
        .unwrap()
        .into_body()
        .into_data_stream();
    assert_eq!(
        json_frame(&frame(&mut reconnected).await, 0),
        json!({"event":{"start":{"pid":pid}}})
    );
    assert_eq!(
        bytes(
            guest
                .handle(request("CloseStdin", &json!({"process":{"pid":pid}})))
                .await
                .unwrap()
        )
        .await,
        "{}"
    );
    end(&mut reconnected).await;
    drop(reconnected);
    clean(&fixture, &registry, command).await;
    assert_eq!(
        std::fs::read(fixture.directory.path().join("effect")).unwrap(),
        b"once"
    );
}
