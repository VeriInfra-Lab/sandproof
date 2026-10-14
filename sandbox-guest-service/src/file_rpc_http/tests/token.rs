use super::*;

#[tokio::test]
async fn token_denials_precede_basic_method_codec_timeout_and_body_without_file_admission() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    for method in ["POST", "GET", "HEAD"] {
        for token in [None, Some(""), Some("private-wrong-token")] {
            let body = Body::from_stream(futures_util::stream::poll_fn(
                |_| -> std::task::Poll<Option<Result<Bytes, std::io::Error>>> {
                    panic!("token rejection read the request body")
                },
            ));
            let mut req = request("MakeDir", body);
            *req.method_mut() = method.parse().unwrap();
            req.headers_mut().remove("x-access-token");
            if let Some(token) = token {
                req.headers_mut()
                    .insert("x-access-token", token.parse().unwrap());
            }
            for (name, value) in [
                ("authorization", "Basic bWlzc2luZzo="),
                ("content-type", "application/grpc"),
                ("content-encoding", "br"),
                ("connect-protocol-version", "bad"),
                ("connect-timeout-ms", "0"),
            ] {
                req.headers_mut().insert(name, value.parse().unwrap());
            }
            let response = f.http.handle(req).await.unwrap();
            assert_eq!(response.status(), 401);
            assert_eq!(
                response.headers()["content-type"],
                "application/json; charset=utf-8"
            );
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            assert_eq!(f.calls.load(Ordering::SeqCst), 0);
            full(&f);
            f.session
                .initialize(br#"{"accessToken":"token","defaultUser":"user"}"#)
                .await
                .unwrap();
            let bytes = drain(response.into_body()).await;
            if method == "HEAD" {
                assert!(bytes.is_empty());
            } else {
                let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(value["code"], 401);
                assert_eq!(
                    value["message"],
                    "unauthorized access, please provide a valid access token or method signing if supported"
                );
                assert!(!String::from_utf8_lossy(&bytes).contains("private-wrong-token"));
            }
            released(&f).await;
        }
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn token_reply_budget_failure_keeps_only_request_capacity_and_original_deadline() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 1).await;
    let start = Instant::now();
    let mut req = request("MakeDir", Body::from(r#"{"path":"token-refused"}"#));
    req.headers_mut().remove("x-access-token");
    let failed = f.http.handle(req).await.unwrap_err();
    assert_eq!(
        failed.error(),
        &Error::Response(file_rpc_response::Error::ResponseLimit)
    );
    assert!(failed.deadline().unwrap() >= start + Duration::from_secs(30));
    assert!(failed.deadline().unwrap() <= Instant::now() + Duration::from_secs(30));
    full(&f);
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    f.session
        .initialize(br#"{"accessToken":"token","defaultUser":"user"}"#)
        .await
        .unwrap();
    // A token denial has never reserved a file slot. Its retained response
    // failure cannot block the independent request pool from legitimate I/O.
    let response = f
        .file_probe
        .handle(request(
            "MakeDir",
            Body::from(r#"{"path":"other-request"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    drain(response.into_body()).await;
    assert!(!root.path().join("token-refused").exists());
    assert!(root.path().join("other-request").is_dir());
    full(&f);
    drop(failed);
    released(&f).await;
}
