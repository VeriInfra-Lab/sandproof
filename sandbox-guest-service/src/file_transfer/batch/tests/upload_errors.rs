use super::{
    Fixture,
    gzip::encode,
    multipart::{CONTENT_TYPE, field, quiesce},
    upload::{request, settings},
};
use crate::{
    file_transfer::{FileStreamError, UploadHttpError, UploadRequest, UploadRequestError},
    upload_response::{Error, Limits},
};
use axum::{body::Body, response::Response};
use http_body_util::BodyExt;
use std::sync::atomic::Ordering;

fn response_limits() -> Limits {
    Limits {
        wire_bytes: 4096,
        chunk_bytes: 7,
    }
}
async fn upload(
    f: &Fixture,
    request: UploadRequest,
    limits: Limits,
) -> Result<Response, UploadHttpError> {
    f.streams
        .upload_request_response(
            f.session.authorize(Some("token")).await.unwrap(),
            request,
            &f.accounts,
            &f.settings,
            settings(),
            limits,
        )
        .await
}
async fn collect(mut body: Body) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        bytes.extend_from_slice(&frame.unwrap().into_data().unwrap());
    }
    bytes
}
#[tokio::test]
async fn real_parent_and_target_faults_cross_worker_for_identity_and_gzip() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("parent");
    std::fs::write(&parent, b"kept").unwrap();
    let f = Fixture::new(root.path()).await;
    for compressed in [false, true] {
        for (path, status, message) in [
            (
                root.path().to_owned(),
                400,
                format!("path is a directory: {}", root.path().display()),
            ),
            (
                parent.join("child"),
                500,
                format!(
                    "error ensuring directories: path is a file: {}",
                    parent.display()
                ),
            ),
        ] {
            let bytes = if compressed {
                encode(b"body")
            } else {
                b"body".to_vec()
            };
            let response = upload(
                &f,
                request(
                    &bytes,
                    compressed,
                    "application/octet-stream",
                    path.to_str(),
                ),
                response_limits(),
            )
            .await
            .unwrap();
            assert_eq!(response.status(), status);
            assert_eq!(
                response.headers()["content-type"],
                "application/json; charset=utf-8"
            );
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            let actual: serde_json::Value =
                serde_json::from_slice(&collect(response.into_body()).await).unwrap();
            assert_eq!(
                actual,
                serde_json::json!({"code":status, "message":message})
            );
            quiesce(&f).await;
        }
    }
    assert_eq!(f.calls.load(Ordering::SeqCst), 4);
    assert_eq!(std::fs::read(parent).unwrap(), b"kept");
}
fn partial_request(root: &std::path::Path) -> UploadRequest {
    let input = [
        field("file", Some("first"), b"done", false),
        field("file", root.to_str(), b"fail", false),
        field("file", Some("never"), b"never", false),
        b"--B--\r\n".to_vec(),
    ]
    .concat();
    request(&input, false, CONTENT_TYPE, None)
}
#[tokio::test]
async fn partial_batch_error_keeps_slot_through_byte_clones_and_allows_init_after_io() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let response = upload(&f, partial_request(root.path()), response_limits())
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let mut body = response.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = bytes.clone();
    drop(bytes);
    drop(body);
    assert_eq!(f.streams.slots.available_permits(), 0);
    let denied = upload(
        &f,
        request(b"new", false, "application/octet-stream", Some("denied")),
        response_limits(),
    )
    .await;
    assert!(matches!(
        denied,
        Err(UploadHttpError::Request(UploadRequestError::Transfer(
            FileStreamError::ResourceExhausted
        )))
    ));
    f.session
        .initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#)
        .await
        .unwrap();
    assert_eq!(std::fs::read(root.path().join("first")).unwrap(), b"done");
    assert!(!root.path().join("never").exists());
    assert!(!root.path().join("denied").exists());
    drop(copy);
    quiesce(&f).await;
}
#[tokio::test]
async fn error_encoding_budget_failure_preserves_completed_files_and_returns_slot() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let result = upload(
        &f,
        partial_request(root.path()),
        Limits {
            wire_bytes: 1,
            ..response_limits()
        },
    )
    .await;
    assert!(matches!(
        result,
        Err(UploadHttpError::Response(Error::ResponseLimit))
    ));
    quiesce(&f).await;
    assert_eq!(std::fs::read(root.path().join("first")).unwrap(), b"done");
    assert!(!root.path().join("never").exists());
}
