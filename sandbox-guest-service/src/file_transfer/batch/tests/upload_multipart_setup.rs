use super::{
    Fixture,
    gzip::encode,
    multipart::{CONTENT_TYPE, quiesce},
    upload::{request, settings},
    upload_errors::{collect, response_limits, upload},
    upload_headers::unread_body,
};
use crate::{
    file_transfer::{
        FileStreamError, UploadBodyError, UploadHttpError, UploadRequest, UploadRequestError,
    },
    upload_response::{Error, Limits},
};
use axum::http::HeaderValue;
use http_body_util::BodyExt;
use sandproof_guest_runtime::multipart::{BoundaryRejection, Error as DecodeError};
use std::{sync::atomic::Ordering, time::Duration};

fn missing() -> UploadRequest {
    let mut req = request(b"never", false, "multipart/form-data", None);
    req.body = unread_body();
    req
}

#[tokio::test]
async fn setup_errors_keep_first_header_and_reject_before_identity_body_or_file_io() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let path = root.path().join("kept");
    std::fs::write(&path, b"original").unwrap();
    let mut calls = 0;
    for compressed in [false, true] {
        for (content, message) in [
            (
                "multipart/related; boundary=B",
                "error parsing multipart form: request Content-Type isn't multipart/form-data",
            ),
            (
                "multipart/form-data; broken",
                "error parsing multipart form: request Content-Type isn't multipart/form-data",
            ),
            (
                "multipart/form-data",
                "error parsing multipart form: no multipart boundary param in Content-Type",
            ),
            (
                "multipart/mixed",
                "error parsing multipart form: no multipart boundary param in Content-Type",
            ),
            (
                "multipart/form-data; boundary=\"\"",
                "error reading form: multipart: boundary is empty",
            ),
        ] {
            let data = if compressed {
                encode(b"never")
            } else {
                b"never".to_vec()
            };
            let mut req = request(&data, compressed, content, path.to_str());
            req.headers
                .append("content-type", HeaderValue::from_static(CONTENT_TYPE));
            if !compressed {
                req.body = unread_body();
            }
            let response = upload(&f, req, response_limits()).await.unwrap();
            assert_eq!(response.status(), 500);
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            let actual: serde_json::Value =
                serde_json::from_slice(&collect(response.into_body()).await).unwrap();
            assert_eq!(actual, serde_json::json!({"code":500,"message":message}));
            quiesce(&f).await;
            calls += 1;
            assert_eq!(f.calls.load(Ordering::SeqCst), calls);
            assert_eq!(std::fs::read(&path).unwrap(), b"original");
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        }
    }
}

#[tokio::test]
async fn metadata_dispatch_and_budgets_precede_setup_and_body_errors_stay_separate() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    for invalid_metadata in [true, false] {
        let mut req = missing();
        if invalid_metadata {
            req.headers
                .insert("x-metadata-note", HeaderValue::from_static("bad\t"));
        } else {
            req.headers.insert(
                "content-type",
                HeaderValue::from_static("multipart/form-data; boundary=B; boundary=C"),
            );
        }
        let response = upload(&f, req, response_limits()).await.unwrap();
        assert_eq!(response.status(), 400);
        drop(response);
        quiesce(&f).await;
    }
    for oversized_body in [false, true] {
        let mut limits = settings();
        let mut req = missing();
        if oversized_body {
            req.expected = Some(limits.multipart.wire.total_bytes + 1);
        } else {
            limits.multipart.wire.header_bytes = 1;
        }
        let result = f
            .streams
            .upload_request_response(
                f.session.authorize(Some("token")).await.unwrap(),
                req,
                &f.accounts,
                &f.settings,
                limits,
                response_limits(),
            )
            .await;
        assert!(
            matches!(result, Err(UploadHttpError::Request(UploadRequestError::Body(error)))
            if error == if oversized_body {UploadBodyError::InputLimit} else {UploadBodyError::Decode(DecodeError::ResourceExhausted)})
        );
        quiesce(&f).await;
    }
    let result = upload(
        &f,
        request(b"--B\r\n", false, CONTENT_TYPE, None),
        response_limits(),
    )
    .await;
    assert!(matches!(
        result,
        Err(UploadHttpError::Request(UploadRequestError::Body(
            UploadBodyError::Decode(DecodeError::TruncatedHeaders)
        )))
    ));
    quiesce(&f).await;
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn setup_response_preserves_original_slot_deadline_and_releases_after_encoding_failure() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let response = upload(&f, missing(), response_limits()).await.unwrap();
    let mut body = response.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = bytes.clone();
    drop(bytes);
    assert!(matches!(
        upload(&f, missing(), response_limits()).await,
        Err(UploadHttpError::Request(UploadRequestError::Transfer(
            FileStreamError::ResourceExhausted
        )))
    ));
    f.session
        .initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#)
        .await
        .unwrap();
    let next = body.frame();
    tokio::pin!(next);
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(next.as_mut().poll(cx).is_pending()))
            .await
    );
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    assert!(next.await.unwrap().is_err());
    tokio::time::resume();
    drop(body);
    assert_eq!(f.streams.slots.available_permits(), 0);
    drop(copy);
    quiesce(&f).await;
    assert!(matches!(
        upload(
            &f,
            missing(),
            Limits {
                wire_bytes: 1,
                ..response_limits()
            }
        )
        .await,
        Err(UploadHttpError::Response(Error::ResponseLimit))
    ));
    quiesce(&f).await;
    let result = f
        .streams
        .upload_request(
            f.session.authorize(Some("token")).await.unwrap(),
            missing(),
            &f.accounts,
            &f.settings,
            settings(),
        )
        .await;
    assert!(matches!(
        result,
        Err(UploadRequestError::Body(UploadBodyError::MultipartSetup(
            BoundaryRejection::MissingBoundary
        )))
    ));
    quiesce(&f).await;
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
