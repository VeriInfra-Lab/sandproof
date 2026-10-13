use super::{
    Fixture,
    multipart::quiesce,
    upload::{request, settings},
    upload_errors::{collect, response_limits, upload},
};
use crate::{
    file_transfer::{FileStreamError, UploadHttpError, UploadRequest, UploadRequestError},
    upload_response::{Error, Limits},
};
use axum::{
    body::{Body, Bytes},
    http::HeaderValue,
};
use http_body_util::BodyExt;
use std::{sync::atomic::Ordering, time::Duration};

fn unread_body() -> Body {
    Body::from_stream(futures_util::stream::poll_fn(
        |_| -> std::task::Poll<Option<Result<Bytes, std::io::Error>>> {
            panic!("rejected identity request must not poll its body")
        },
    ))
}
fn raw_without_path() -> UploadRequest {
    let mut req = request(b"never", false, "application/octet-stream", None);
    req.body = unread_body();
    req
}

#[tokio::test]
async fn header_rejections_preserve_order_first_values_and_never_poll_or_open_files() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    for case in 0..4 {
        let mut req = raw_without_path();
        let (message, calls) = match case {
            0 => {
                req.username = Some(String::new());
                req.headers
                    .insert("content-encoding", HeaderValue::from_static("br"));
                req.headers
                    .append("content-encoding", HeaderValue::from_static("identity"));
                (
                    "error decompressing request body: unsupported Content-Encoding: br, supported: [gzip]",
                    0,
                )
            }
            1 => {
                req.headers
                    .insert("content-type", HeaderValue::from_static("text/plain"));
                req.headers
                    .insert("x-metadata-note", HeaderValue::from_static("bad\t"));
                (
                    "invalid metadata: metadata value for key \"note\" contains non-printable-ASCII byte 0x09 at offset 3",
                    1,
                )
            }
            2 => {
                req.headers
                    .insert("content-type", HeaderValue::from_static("text/plain"));
                req.headers.append(
                    "content-type",
                    HeaderValue::from_static("application/octet-stream"),
                );
                (
                    "unsupported content type: text/plain, expected multipart/form-data or application/octet-stream",
                    2,
                )
            }
            _ => ("path query parameter is required for raw body upload", 3),
        };
        let response = upload(&f, req, response_limits()).await.unwrap();
        assert_eq!(response.status(), 400);
        assert_eq!(
            response.headers()["content-type"],
            "application/json; charset=utf-8"
        );
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        let actual: serde_json::Value =
            serde_json::from_slice(&collect(response.into_body()).await).unwrap();
        assert_eq!(actual, serde_json::json!({"code":400,"message":message}));
        quiesce(&f).await;
        assert_eq!(f.calls.load(Ordering::SeqCst), calls);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn rejected_request_uses_original_slot_and_expiry_while_init_can_continue() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let response = upload(&f, raw_without_path(), response_limits())
        .await
        .unwrap();
    let mut body = response.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = bytes.clone();
    drop(bytes);
    assert!(matches!(
        upload(&f, raw_without_path(), response_limits()).await,
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
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn component_budgets_remain_explicit_and_error_encoding_failure_releases_capacity() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let mut limits = settings();
    limits.encoding.header_bytes = 1;
    let result = f
        .streams
        .upload_request_response(
            f.session.authorize(Some("token")).await.unwrap(),
            raw_without_path(),
            &f.accounts,
            &f.settings,
            limits,
            response_limits(),
        )
        .await;
    assert!(matches!(
        result,
        Err(UploadHttpError::Request(UploadRequestError::Encoding(
            crate::file_encoding::Error::HeaderLimit
        )))
    ));
    quiesce(&f).await;
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        upload(
            &f,
            raw_without_path(),
            Limits {
                wire_bytes: 1,
                ..response_limits()
            }
        )
        .await,
        Err(UploadHttpError::Response(Error::ResponseLimit))
    ));
    quiesce(&f).await;
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
