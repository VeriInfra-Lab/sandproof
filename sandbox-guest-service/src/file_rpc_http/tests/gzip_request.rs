use super::*;
use crate::file_rpc_requests::tests::gzip::compressed;

fn encoded_request(method: &str, data: Vec<u8>) -> Request<Body> {
    let mut req = request(method, Body::from(data));
    req.headers_mut()
        .insert("content-encoding", "gzip".parse().unwrap());
    // Connect still compresses its success response for a gzip request.
    req.headers_mut()
        .insert("accept-encoding", "identity".parse().unwrap());
    req
}

#[tokio::test]
async fn compressed_methods_keep_both_slots_and_do_not_refresh_the_account() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    prepare_error_tree(root.path());
    for (method, raw) in [
        ("Stat", r#"{"path":"file"}"#),
        ("ListDir", r#"{"path":"dir"}"#),
        ("MakeDir", r#"{"path":"new"}"#),
        ("Move", r#"{"source":"file","destination":"moved"}"#),
        ("Remove", r#"{"path":"moved"}"#),
    ] {
        let before = f.calls.load(Ordering::SeqCst);
        let mut req = encoded_request(method, compressed(raw.as_bytes()));
        req.headers_mut()
            .insert("authorization", "Basic dXNlcjo=".parse().unwrap());
        let Prepared::Ready(ready) = f.http.prepare(req).await.unwrap() else {
            panic!("expected ready")
        };
        assert_eq!(f.calls.load(Ordering::SeqCst), before + 1);
        let response = ready.execute().await.unwrap();
        assert_eq!(f.calls.load(Ordering::SeqCst), before + 1);
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-encoding"], "gzip");
        full(&f);
        file_full(&f).await;
        let decoded = super::gzip::decoded(&drain(response.into_body()).await);
        if method == "Remove" {
            assert_eq!(decoded, serde_json::json!({}));
        }
        released(&f).await;
    }
    assert!(!root.path().join("moved").exists());
    assert!(root.path().join("new").is_dir());
}

#[tokio::test]
async fn invalid_compressed_request_never_dispatches_and_releases_capacity_after_failure() {
    use crate::{
        file_rpc_requests::{CompressionStage, Error as InputError},
        upload_body::GzipError,
    };
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    let mut crc = compressed(br#"{"path":"refused"}"#);
    let len = crc.len();
    crc[len - 8] ^= 1;
    let mut later = compressed(br#"{"path":"refused"}"#);
    later.extend(b"bad");
    let big = format!(r#"{{"path":"{}"}}"#, "a".repeat(4096));
    for (data, expected) in [
        (
            crc,
            InputError::Compression {
                stage: CompressionStage::Body,
                error: GzipError::Checksum,
            },
        ),
        (
            later,
            InputError::Compression {
                stage: CompressionStage::Body,
                error: GzipError::Truncated,
            },
        ),
        (compressed(big.as_bytes()), InputError::BodyLimit),
    ] {
        let result = f.http.handle(encoded_request("MakeDir", data)).await;
        assert_eq!(f.calls.load(Ordering::SeqCst), 0);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        full(&f);
        file_full(&f).await;
        f.session
            .initialize(br#"{"accessToken":"token","defaultUser":"user"}"#)
            .await
            .unwrap();
        if expected == InputError::BodyLimit {
            let failure = result.unwrap_err();
            assert_eq!(failure.error(), &Error::Request(expected));
            assert!(failure.deadline().is_some());
            drop(failure);
        } else {
            let response = result.unwrap();
            assert_eq!(response.status(), 400);
            assert_eq!(response.headers()["content-type"], "application/json");
            assert!(!response.headers().contains_key("content-encoding"));
            let body: serde_json::Value =
                serde_json::from_slice(&drain(response.into_body()).await).unwrap();
            assert_eq!(body["code"], "invalid_argument");
            assert_eq!(
                body["message"],
                if matches!(
                    expected,
                    InputError::Compression {
                        error: GzipError::Checksum,
                        ..
                    }
                ) {
                    "decompress: gzip: invalid checksum"
                } else {
                    "decompress: unexpected EOF"
                }
            );
        }
        released(&f).await;
    }
    let mut req = encoded_request("MakeDir", b"invalid gzip".to_vec());
    req.headers_mut()
        .insert("authorization", "Basic bWlzc2luZzo=".parse().unwrap());
    let response = f.http.handle(req).await.unwrap();
    assert_eq!(response.status(), 401);
    assert!(!response.headers().contains_key("content-encoding"));
    drop(drain(response.into_body()).await);
    released(&f).await;
    let response = f
        .http
        .handle(encoded_request(
            "MakeDir",
            compressed(br#"{"path":"recovered"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    drop(drain(response.into_body()).await);
    released(&f).await;
    assert!(root.path().join("recovered").is_dir());
}

#[tokio::test]
async fn gzip_error_encoding_failure_retains_original_deadline_and_both_slots() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 2).await;
    let failure = f
        .http
        .handle(encoded_request("MakeDir", b"invalid gzip header".to_vec()))
        .await
        .unwrap_err();
    assert_eq!(
        failure.error(),
        &Error::Response(file_rpc_response::Error::ResponseLimit)
    );
    let end = failure.deadline().unwrap();
    assert!(end <= tokio::time::Instant::now() + Duration::from_secs(5));
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    full(&f);
    file_full(&f).await;
    f.session
        .initialize(br#"{"accessToken":"token","defaultUser":"user"}"#)
        .await
        .unwrap();
    assert_eq!(failure.deadline(), Some(end));
    drop(failure);
    released(&f).await;
}
