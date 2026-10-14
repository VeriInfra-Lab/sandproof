use super::*;

#[tokio::test]
async fn non_post_methods_follow_basic_preflight_and_precede_codec_headers_and_body() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    let mut expected_calls = 0;
    for rpc in ["Stat", "MakeDir", "ListDir", "Move", "Remove"] {
        for method in [
            "GET", "HEAD", "PUT", "PATCH", "DELETE", "OPTIONS", "get", "post",
        ] {
            for auth in [None, Some("Basic dXNlcjo="), Some("Basic bWlzc2luZzo=")] {
                let body = Body::from_stream(futures_util::stream::poll_fn(
                    |_| -> std::task::Poll<Option<Result<Bytes, std::io::Error>>> {
                        panic!("method/Basic rejection polled request body")
                    },
                ));
                let mut req = request(rpc, body);
                *req.method_mut() = method.parse().unwrap();
                if let Some(auth) = auth {
                    req.headers_mut()
                        .insert("authorization", auth.parse().unwrap());
                    expected_calls += 1;
                } else {
                    // Method rejection also precedes unsupported media type.
                    req.headers_mut()
                        .insert("content-type", "application/proto".parse().unwrap());
                }
                for (name, value) in [
                    ("content-encoding", "br"),
                    ("connect-protocol-version", "bad"),
                    ("connect-timeout-ms", "0"),
                ] {
                    req.headers_mut().insert(name, value.parse().unwrap());
                }
                let response = f.http.handle(req).await.unwrap();
                assert_eq!(f.calls.load(Ordering::SeqCst), expected_calls);
                full(&f);
                file_full(&f).await;
                f.session
                    .initialize(br#"{"accessToken":"token","defaultUser":"user"}"#)
                    .await
                    .unwrap();
                if auth == Some("Basic bWlzc2luZzo=") {
                    assert_eq!(response.status(), 401);
                    let data = drain(response.into_body()).await;
                    let value: serde_json::Value = serde_json::from_slice(&data).unwrap();
                    assert_eq!(value["message"], "invalid username: 'missing'");
                } else {
                    assert_eq!(response.status(), 405);
                    assert_eq!(response.headers()["allow"], "POST");
                    assert_eq!(
                        response.headers().len(),
                        if method == "HEAD" { 1 } else { 2 }
                    );
                    assert!(drain(response.into_body()).await.is_empty());
                }
                released(&f).await;
            }
        }
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn failed_method_response_retains_original_two_slots_and_shorter_deadline() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 0).await;
    let start = Instant::now();
    let mut req = request("MakeDir", Body::from(r#"{"path":"refused"}"#));
    *req.method_mut() = axum::http::Method::GET;
    let failure = f.http.handle(req).await.unwrap_err();
    assert_eq!(
        failure.error(),
        &Error::Response(file_rpc_response::Error::InvalidConfiguration)
    );
    assert!(failure.deadline().unwrap() >= start);
    assert!(failure.deadline().unwrap() <= Instant::now() + Duration::from_secs(5));
    full(&f);
    file_full(&f).await;
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    f.session
        .initialize(br#"{"accessToken":"token","defaultUser":"user"}"#)
        .await
        .unwrap();
    assert!(!root.path().join("refused").exists());
    drop(failure);
    released(&f).await;
    // The file slot recovers as well; the independent request pool reaches I/O.
    let response = f
        .file_probe
        .handle(request("MakeDir", Body::from(r#"{"path":"recovered"}"#)))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    drain(response.into_body()).await;
    assert!(root.path().join("recovered").is_dir());
}
