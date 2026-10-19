use super::*;

#[cfg(feature = "sdk-contract-tests")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fixed_sync_async_sdk_observes_real_guest_capacity_errors() {
    use std::process::Stdio;
    let fixture = fixture().await;
    let components = parts(&fixture, &registry(), &fixture.session);
    let pools = components.pools;
    let guest = GuestHttp::new(fixture.session.clone(), components.components).unwrap();
    let held = (
        pools.ordinary.admit().unwrap(),
        pools.control.admit().unwrap(),
        pools.files.admit().unwrap(),
        pools.rpc.admit().unwrap(),
    );
    let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let server = tokio::spawn(guest.serve(transport::transport(), socket, async {
        let _ = stopped.await;
    }));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut sdk = tokio::process::Command::new("uv")
        .current_dir(root)
        .args([
            "run",
            "--locked",
            "--project",
            "tests/compat",
            "python",
            "tests/compat/guest_admission_sdk.py",
        ])
        .arg(format!("http://{address}"))
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let status = timeout(Duration::from_mins(1), sdk.wait()).await;
    let _ = stop.send(());
    let report = timeout(Duration::from_secs(10), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        status
            .expect("SDK capacity probe timed out")
            .unwrap()
            .success()
    );
    assert_eq!((report.failed, report.expired, report.refused), (0, 0, 0));
    fixture.assert_no_effect();
    assert_eq!(fixture.gate_count(), 0);
    drop(held);
    assert!(pools.ordinary.admit().is_ok());
    assert!(pools.control.admit().is_ok());
    assert!(pools.files.admit().is_ok());
    assert!(pools.rpc.admit().is_ok());
}

#[tokio::test]
async fn capacity_errors_match_shared_contract_before_authentication_or_body_work() {
    let fixture = fixture().await;
    let components = parts(&fixture, &registry(), &fixture.session);
    let pools = components.pools;
    let guest = GuestHttp::new(fixture.session.clone(), components.components).unwrap();
    let ordinary = pools.ordinary.admit().unwrap();
    let control = pools.control.admit().unwrap();
    let files = pools.files.admit().unwrap();
    let rpc = pools.rpc.admit().unwrap();
    let contract: Value = serde_json::from_str(include_str!(
        "../../../../tests/compat/guest-service-contract.json"
    ))
    .unwrap();
    for case in contract["errors"].as_array().unwrap() {
        let mut probe = at(
            case["request"]["method"].as_str().unwrap().parse().unwrap(),
            case["request"]["path"].as_str().unwrap(),
            Body::empty(),
        );
        // Admission precedes even malformed credentials and codec selection.
        probe
            .headers_mut()
            .insert("x-access-token", "wrong".parse().unwrap());
        probe
            .headers_mut()
            .insert("authorization", "Basic !".parse().unwrap());
        unread(&mut probe);
        let response = guest.handle(probe).await.unwrap();
        assert_eq!(
            u64::from(response.status().as_u16()),
            case["response"]["status"].as_u64().unwrap()
        );
        assert_eq!(response.headers()["content-type"], "application/json");
        assert_eq!(response.headers()["connection"], "close");
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(
            collected(response).await,
            case["response"]["body"].as_str().unwrap().as_bytes()
        );
    }
    fixture.assert_no_effect();
    assert_eq!(fixture.gate_count(), 0);
    assert_eq!(pools.streams.available_slots(), 1);
    drop((ordinary, control, files, rpc));
    assert!(pools.ordinary.admit().is_ok());
    assert!(pools.control.admit().is_ok());
    assert!(pools.files.admit().is_ok());
    assert!(pools.rpc.admit().is_ok());
}
