use super::*;

#[tokio::test]
async fn expansion_rejections_use_one_account_and_never_dispatch_either_move_path() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    std::fs::write(root.path().join("source"), b"keep").unwrap();
    for explicit in [false, true] {
        for (method, value) in [
            ("Stat", serde_json::json!({"path":"~other"})),
            ("MakeDir", serde_json::json!({"path":"~other"})),
            ("ListDir", serde_json::json!({"path":"~other"})),
            ("Remove", serde_json::json!({"path":"~other"})),
            (
                "Move",
                serde_json::json!({"source":"~other","destination":"partial/dest"}),
            ),
            (
                "Move",
                serde_json::json!({"source":"source","destination":"~other"}),
            ),
        ] {
            let before = f.calls.load(Ordering::SeqCst);
            let mut req = request(method, Body::from(value.to_string()));
            if explicit {
                req.headers_mut()
                    .insert("authorization", "Basic dXNlcjpwdw==".parse().unwrap());
            }
            let response = f.http.handle(req).await.unwrap();
            assert_eq!(response.status(), 400);
            assert_eq!(f.calls.load(Ordering::SeqCst), before + 1);
            full(&f);
            file_full(&f).await;
            let value: serde_json::Value =
                serde_json::from_slice(&drain(response.into_body()).await).unwrap();
            assert_eq!(
                value,
                serde_json::json!({"code":"invalid_argument","message":"failed to expand path '' for user 'user': cannot expand user-specific home dir"})
            );
            released(&f).await;
            assert!(!root.path().join("partial").exists());
            assert_eq!(std::fs::read(root.path().join("source")).unwrap(), b"keep");
        }
    }
    // Empty input uses the authenticated default workdir, but the fixed error
    // contains expand's empty result, not that default or the submitted path.
    f.session
        .initialize(br#"{"accessToken":"token","defaultUser":"user","defaultWorkdir":"~fallback"}"#)
        .await
        .unwrap();
    let response = f
        .http
        .handle(request("Stat", Body::from("{}")))
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    // Retaining the response must not retain account lookup authorization.
    f.session
        .initialize(br#"{"accessToken":"token","defaultUser":"missing"}"#)
        .await
        .unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&drain(response.into_body()).await).unwrap();
    assert!(
        value["message"]
            .as_str()
            .unwrap()
            .contains("for user 'user'")
    );
    released(&f).await;
    let response = f
        .http
        .handle(request("Stat", Body::from(r#"{"path":"~other"}"#)))
        .await
        .unwrap();
    assert_eq!(response.status(), 401); // Missing account precedes expansion.
    drain(response.into_body()).await;
    released(&f).await;
}

#[tokio::test]
async fn path_error_encoding_failure_keeps_capacity_and_original_deadline() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 1).await;
    let prepared = f
        .http
        .prepare(request(
            "Move",
            Body::from(r#"{"source":"~other","destination":"partial/dest"}"#),
        ))
        .await
        .unwrap();
    let Prepared::Ready(ready) = prepared else {
        panic!("expected decoded request")
    };
    let end = ready.deadline();
    let failure = ready.execute().await.unwrap_err();
    assert_eq!(
        failure.error(),
        &Error::Response(file_rpc_response::Error::ResponseLimit)
    );
    assert_eq!(failure.deadline(), Some(end));
    assert!(!root.path().join("partial").exists());
    full(&f);
    file_full(&f).await;
    drop(failure);
    released(&f).await;
}
