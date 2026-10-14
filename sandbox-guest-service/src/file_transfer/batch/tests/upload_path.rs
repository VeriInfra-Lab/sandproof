use super::{
    Fixture,
    gzip::encode,
    multipart::{CONTENT_TYPE, field, quiesce},
    upload::{request, settings},
    upload_errors::{collect, response_limits, upload},
    upload_headers::unread_body,
};
use crate::{
    ContextError,
    file_transfer::{FileStreamError, UploadBodyError, UploadRequestError},
};

fn message() -> &'static str {
    "error resolving path: failed to expand path '' for user 'alice': cannot expand user-specific home dir"
}

#[tokio::test]
async fn raw_expansion_fails_before_identity_body_and_preserves_coarse_caller() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let mut req = request(
        b"never",
        false,
        "application/octet-stream",
        Some("~other/file"),
    );
    req.body = unread_body();
    let response = upload(&f, req, response_limits()).await.unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&collect(response.into_body()).await).unwrap(),
        serde_json::json!({"code":400,"message":message()})
    );
    quiesce(&f).await;
    let result = f
        .streams
        .upload_request(
            f.session.authorize(Some("token")).await.unwrap(),
            request(
                b"never",
                false,
                "application/octet-stream",
                Some("~other/file"),
            ),
            &f.accounts,
            &f.settings,
            settings(),
        )
        .await;
    assert!(matches!(
        result,
        Err(UploadRequestError::Body(UploadBodyError::Transfer(
            FileStreamError::Context(ContextError::InvalidArgument)
        )))
    ));
    quiesce(&f).await;
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn multipart_path_error_preserves_completed_file_and_stops_before_later_parts() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    for compressed in [false, true] {
        let first = format!("before-{compressed}");
        let last = format!("after-{compressed}");
        let bytes = [
            field("file", Some(&first), b"kept", false),
            field("file", Some("~other/never"), b"never", false),
            field("file", Some(&last), b"later", false),
            b"--B--\r\n".to_vec(),
        ]
        .concat();
        let bytes = if compressed { encode(&bytes) } else { bytes };
        let response = upload(
            &f,
            request(&bytes, compressed, CONTENT_TYPE, None),
            response_limits(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), 400);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&collect(response.into_body()).await)
                .unwrap(),
            serde_json::json!({"code":400,"message":message()})
        );
        quiesce(&f).await;
        assert_eq!(std::fs::read(root.path().join(first)).unwrap(), b"kept");
        assert!(!root.path().join(last).exists());
    }
    // Query override is selected before filename expansion; a rejected-looking
    // ignored filename cannot override a valid signed query path.
    let bytes = [
        field("file", Some("~other/ignored"), b"ok", false),
        b"--B--\r\n".to_vec(),
    ]
    .concat();
    let response = upload(
        &f,
        request(&bytes, false, CONTENT_TYPE, Some("override")),
        response_limits(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 200);
    drop(collect(response.into_body()).await);
    quiesce(&f).await;
    assert_eq!(std::fs::read(root.path().join("override")).unwrap(), b"ok");
}
