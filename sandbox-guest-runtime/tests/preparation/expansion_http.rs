use super::*;

#[tokio::test]
async fn path_expansion_rejects_before_worker_or_pid_claim_and_keeps_last_reply_owner() {
    for explicit in [false, true] {
        for cwd in [None, Some(""), Some("~other/x")] {
            let fixture = fixture().await;
            fixture.session.initialize(br#"{"accessToken":"token","defaultUser":"fixture","defaultWorkdir":"~default"}"#).await.unwrap();
            let registry = registry();
            let ordinary = requests(1);
            let control = requests(1);
            let mut processes = processes(&fixture, &registry, &PidRegistrar::new(store(&fixture)));
            // A wrongly attempted prepare cannot silently pass this test.
            processes.worker = fixture.directory.path().join("absent-worker");
            let http = CommandHttp::new(
                fixture.session.clone(),
                RequestPools {
                    ordinary: ordinary.clone(),
                    control,
                },
                accounts(&fixture),
                settings(),
                processes,
                responses(),
            )
            .unwrap();
            let claims = fixture.directory.path().join("pid-claims.bin");
            let original_claims = std::fs::read(&claims).unwrap();
            let mut payload = json!({"process":{"cmd":"/bin/sh","args":["-c","printf forbidden > effect"], "envs":{"USER":"not-the-account"}}});
            if let Some(cwd) = cwd {
                payload["process"]["cwd"] = json!(cwd);
            }
            let mut req = request("Start", &payload);
            if explicit {
                req.headers_mut()
                    .insert("authorization", "Basic Zml4dHVyZTo=".parse().unwrap());
            }
            let response = http.handle(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let mut body = response.into_body().into_data_stream();
            let first = frame(&mut body).await;
            assert_eq!(
                json_frame(&first, 2),
                json!({"error":{"code":"invalid_argument","message":"failed to expand path '' for user 'fixture': cannot expand user-specific home dir"}})
            );
            let last = first.clone();
            drop(first);
            drop(body);
            full(&ordinary);
            rotates(&fixture).await;
            assert_eq!(fixture.gate_count(), 0);
            fixture.assert_no_effect();
            assert_eq!(std::fs::read(&claims).unwrap(), original_claims);
            assert_eq!(registry.available_slots(), 1);
            drop(fixture.owners.reserve().unwrap());
            drop(last);
            timeout(Duration::from_secs(5), async {
                loop {
                    if ordinary.admit().is_ok() {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert_eq!(
                bytes(http.handle(request("List", &json!({}))).await.unwrap()).await,
                "{}"
            );
        }
    }
}

#[tokio::test]
async fn explicit_workdir_overrides_invalid_default_and_runs_the_original_command() {
    let fixture = fixture().await;
    fixture
        .session
        .initialize(
            br#"{"accessToken":"token","defaultUser":"fixture","defaultWorkdir":"~default"}"#,
        )
        .await
        .unwrap();
    let registry = registry();
    let ordinary = requests(1);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    let mut body = http
        .handle(request(
            "Start",
            &json!({"process":{"cmd":"/bin/sh","args":["-c","printf ready"],"cwd":"."}}),
        ))
        .await
        .unwrap()
        .into_body()
        .into_data_stream();
    let command = started(&mut body, &registry).await;
    data(&mut body, "stdout", b"ready").await;
    end(&mut body).await;
    drop(body);
    clean(&fixture, &registry, command).await;
    fixture.assert_no_effect();
}

#[tokio::test]
async fn default_account_error_precedes_path_expansion() {
    let fixture = fixture().await;
    fixture
        .session
        .initialize(
            br#"{"accessToken":"token","defaultUser":"missing","defaultWorkdir":"~default"}"#,
        )
        .await
        .unwrap();
    let registry = registry();
    let ordinary = requests(1);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    let response = http
        .handle(request(
            "Start",
            &json!({"process":{"cmd":"/bin/true","cwd":"~other"}}),
        ))
        .await
        .unwrap();
    let mut body = response.into_body().into_data_stream();
    let first = frame(&mut body).await;
    assert_eq!(
        json_frame(&first, 2),
        json!({"error":{"code":"unauthenticated","message":"invalid default user: 'missing'"}})
    );
    drop(first);
    drop(body);
    assert_eq!(fixture.gate_count(), 0);
    fixture.assert_no_effect();
    assert_eq!(registry.available_slots(), 1);
    drop(fixture.owners.reserve().unwrap());
    rotates(&fixture).await;
}
