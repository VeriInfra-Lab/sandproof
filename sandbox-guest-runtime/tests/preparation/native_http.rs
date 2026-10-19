use super::*;

#[tokio::test]
async fn original_native_chdir_failure_returns_exact_pipe_or_pty_spawn_error() {
    for pty in [false, true] {
        let fixture = fixture().await;
        let registry = registry();
        let ordinary = requests(1);
        let control = requests(1);
        let http = http(&fixture, &registry, &ordinary, &control);
        let cwd = fixture.directory.path().join("file-'\"<>&");
        std::fs::write(&cwd, b"original").unwrap();
        let script = "printf forbidden > effect";
        let mut payload = json!({"process":{"cmd":"/bin/sh","args":["-c",script],"cwd":cwd}});
        if pty {
            payload["pty"] = json!({"size":{"cols":65537,"rows":65538}});
        }
        let response = http.handle(request("Start", &payload)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body().into_data_stream();
        let first = frame(&mut body).await;
        let command = format!("/bin/sh -c {script}");
        let message = if pty {
            format!(
                "error starting pty with command '{command}' in dir '{}' with '65537' cols and '65538' rows: fork/exec /bin/sh: not a directory",
                cwd.display()
            )
        } else {
            format!("error starting process '{command}': fork/exec /bin/sh: not a directory")
        };
        assert_eq!(
            json_frame(&first, 2),
            json!({"error":{"code":"invalid_argument","message":message}})
        );
        full(&ordinary);
        rotates(&fixture).await;
        drop(first);
        assert!(
            timeout(Duration::from_secs(5), body.next())
                .await
                .unwrap()
                .is_none()
        );
        drop(body);
        capacity_restored(&fixture.owners).await;
        registry_capacity(&registry).await;
        assert_eq!(fixture.gate_count(), 0);
        fixture.assert_no_effect();
        assert_eq!(std::fs::read(cwd).unwrap(), b"original");
        assert_eq!(
            bytes(http.handle(request("List", &json!({}))).await.unwrap()).await,
            "{}"
        );
    }
}
