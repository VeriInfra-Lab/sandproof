use super::{
    Fixture,
    multipart::{CONTENT_TYPE, quiesce},
    upload::{request, settings},
    upload_errors::{collect, response_limits, upload},
};
use crate::{
    file_transfer::{FileStreamError, UploadHttpError, UploadRequestError},
    upload_body::GzipError,
    upload_response::{Error, Limits},
};
use axum::{
    body::{Body, Bytes},
    http::HeaderValue,
};
use futures_util::stream;
use http_body_util::BodyExt;
use std::{sync::atomic::Ordering, time::Duration};

#[tokio::test]
async fn first_header_errors_precede_accounts_metadata_modes_and_file_effects() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let path = root.path().join("kept");
    std::fs::write(&path, b"original").unwrap();
    for content in ["application/octet-stream", CONTENT_TYPE] {
        for (bytes, message) in [
            (b"".as_slice(), "EOF"),
            (b"\x1f".as_slice(), "unexpected EOF"),
            (b"invalid gzip header".as_slice(), "gzip: invalid header"),
        ] {
            let mut req = request(bytes, true, content, path.to_str());
            req.username = Some("nobody-looked-up".into());
            req.headers
                .insert("x-metadata-bad", HeaderValue::from_static("bad\t"));
            let response = upload(&f, req, response_limits()).await.unwrap();
            assert_eq!(response.status(), 400);
            let actual: serde_json::Value =
                serde_json::from_slice(&collect(response.into_body()).await).unwrap();
            assert_eq!(
                actual,
                serde_json::json!({"code":400, "message":format!("error decompressing request body: failed to create gzip reader: {message}")})
            );
            quiesce(&f).await;
            assert_eq!(f.calls.load(Ordering::SeqCst), 0);
            assert_eq!(std::fs::read(&path).unwrap(), b"original");
        }
    }
}

#[tokio::test]
async fn gzip_header_response_keeps_original_capacity_expiry_and_can_release_auth() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let response = upload(
        &f,
        request(b"", true, "application/octet-stream", Some("never")),
        response_limits(),
    )
    .await
    .unwrap();
    let mut body = response.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = bytes.clone();
    drop(bytes);
    assert!(matches!(
        upload(
            &f,
            request(b"", true, CONTENT_TYPE, None),
            response_limits()
        )
        .await,
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
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn transport_and_budget_failures_are_not_empty_headers_and_failure_releases_slot() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    for broken in [false, true] {
        let mut req = request(b"", true, "application/octet-stream", Some("never"));
        if broken {
            req.body = Body::from_stream(stream::iter([Err::<Bytes, _>(std::io::Error::other(
                "private source fault",
            ))]));
        } else {
            req.expected = Some(1);
        }
        assert!(matches!(
            upload(&f, req, response_limits()).await,
            Err(UploadHttpError::Request(UploadRequestError::GzipPreflight(
                GzipError::Transport
            )))
        ));
        quiesce(&f).await;
    }
    let mut limits = settings();
    limits.gzip.decode.encoded_bytes = 0;
    assert!(matches!(
        f.streams
            .upload_request_response(
                f.session.authorize(Some("token")).await.unwrap(),
                request(b"x", true, CONTENT_TYPE, None),
                &f.accounts,
                &f.settings,
                limits,
                response_limits()
            )
            .await,
        Err(UploadHttpError::Request(UploadRequestError::GzipPreflight(
            GzipError::InputLimit
        )))
    ));
    quiesce(&f).await;
    assert!(matches!(
        upload(
            &f,
            request(b"", true, CONTENT_TYPE, None),
            Limits {
                wire_bytes: 1,
                ..response_limits()
            }
        )
        .await,
        Err(UploadHttpError::Response(Error::ResponseLimit))
    ));
    quiesce(&f).await;
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
