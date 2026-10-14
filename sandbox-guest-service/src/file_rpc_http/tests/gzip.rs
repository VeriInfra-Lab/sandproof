use super::*;
use sandproof_guest_runtime::gzip::{DecodeLimits, Decoder};

fn gzip_request(method: &str, value: &serde_json::Value) -> Request<Body> {
    let mut req = request(method, Body::from(value.to_string()));
    req.headers_mut()
        .insert("accept-encoding", "gzip".parse().unwrap());
    req.headers_mut().insert(
        "content-type",
        "application/json; charset=utf-8".parse().unwrap(),
    );
    req
}
fn decoded(bytes: &[u8]) -> serde_json::Value {
    let mut decoder = Decoder::new(
        bytes,
        DecodeLimits {
            chunk_bytes: 13,
            encoded_bytes: 65536,
            decoded_bytes: 65536,
            header_bytes: 64,
            members: 1,
        },
    )
    .unwrap();
    let mut raw = Vec::new();
    while let Some(chunk) = decoder.next_chunk().unwrap() {
        raw.extend(chunk);
    }
    serde_json::from_slice(&raw).unwrap()
}

#[tokio::test]
async fn unary_gzip_success_and_identity_errors_keep_original_http_ownership() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    prepare_error_tree(root.path());
    for (method, value) in [
        ("Stat", serde_json::json!({"path":"file"})),
        ("ListDir", serde_json::json!({"path":"dir"})),
        ("MakeDir", serde_json::json!({"path":"new"})),
        (
            "Move",
            serde_json::json!({"source":"file","destination":"moved"}),
        ),
        ("Remove", serde_json::json!({"path":"moved"})),
    ] {
        let response = f.http.handle(gzip_request(method, &value)).await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-encoding"], "gzip");
        assert_eq!(
            response.headers()["content-type"],
            "application/json; charset=utf-8"
        );
        full(&f);
        file_full(&f).await;
        let value = decoded(&drain(response.into_body()).await);
        match method {
            "Remove" => assert_eq!(value, serde_json::json!({})),
            "ListDir" => assert_eq!(value, serde_json::json!({"entries":[]})),
            _ => assert!(value["entry"]["name"].is_string()),
        }
        released(&f).await;
    }
    for path in ["missing", "~other"] {
        let response = f
            .http
            .handle(gzip_request("Stat", &serde_json::json!({"path":path})))
            .await
            .unwrap();
        assert!(response.status().is_client_error());
        assert!(!response.headers().contains_key("content-encoding"));
        assert_eq!(response.headers()["content-type"], "application/json");
        let value: serde_json::Value =
            serde_json::from_slice(&drain(response.into_body()).await).unwrap();
        assert_eq!(
            value["code"],
            if path == "missing" {
                "not_found"
            } else {
                "invalid_argument"
            }
        );
        released(&f).await;
    }
}

#[tokio::test]
async fn gzip_limit_failure_preserves_completed_delete_and_original_slots() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 2).await;
    std::fs::write(root.path().join("victim"), b"data").unwrap();
    let Prepared::Ready(ready) = f
        .http
        .prepare(gzip_request(
            "Remove",
            &serde_json::json!({"path":"victim"}),
        ))
        .await
        .unwrap()
    else {
        panic!("expected prepared request")
    };
    let end = ready.deadline();
    let failure = ready.execute().await.unwrap_err();
    assert_eq!(
        failure.error(),
        &Error::Response(file_rpc_response::Error::ResponseLimit)
    );
    assert_eq!(failure.deadline(), Some(end));
    assert!(!root.path().join("victim").exists());
    full(&f);
    file_full(&f).await;
    drop(failure);
    released(&f).await;
}
