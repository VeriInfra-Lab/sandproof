use super::*;

#[tokio::test]
async fn json_errors_preserve_basic_priority_no_file_effects_and_original_slots() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("json-refused");
    let wire = format!(
        r#"{{"path":{},"path":null}}"#,
        serde_json::to_string(path.to_str().unwrap()).unwrap()
    );
    for (wire_bytes, authorization, calls, status) in [
        (65536, None, 0, 400),
        (65536, Some("Basic dXNlcjo="), 1, 400),
        (65536, Some("Basic bWlzc2luZzo="), 1, 401),
        (2, None, 0, 400),
    ] {
        let f = fixture(root.path(), wire_bytes).await;
        let mut req = request("MakeDir", Body::from(wire.clone()));
        req.headers_mut()
            .insert("accept-encoding", "gzip".parse().unwrap());
        if let Some(auth) = authorization {
            req.headers_mut()
                .insert("authorization", auth.parse().unwrap());
        }
        let result = f.http.handle(req).await;
        assert_eq!(f.calls.load(Ordering::SeqCst), calls);
        assert!(!path.exists());
        full(&f);
        file_full(&f).await;
        f.session
            .initialize(br#"{"accessToken":"token","defaultUser":"user"}"#)
            .await
            .unwrap();
        if wire_bytes == 2 {
            let failure = result.unwrap_err();
            assert_eq!(
                failure.error(),
                &Error::Response(file_rpc_response::Error::ResponseLimit)
            );
            assert!(
                failure.deadline().unwrap() <= tokio::time::Instant::now() + Duration::from_secs(5)
            );
            drop(failure);
        } else {
            let response = result.unwrap();
            assert_eq!(response.status(), status);
            assert!(!response.headers().contains_key("content-encoding"));
            let value: serde_json::Value =
                serde_json::from_slice(&drain(response.into_body()).await).unwrap();
            if status == 401 {
                assert_eq!(value["message"], "invalid username: 'missing'");
            } else {
                assert_eq!(value["code"], "invalid_argument");
                assert!(
                    value["message"]
                        .as_str()
                        .unwrap()
                        .ends_with("duplicate field \"path\"")
                );
            }
        }
        released(&f).await;
    }
}
