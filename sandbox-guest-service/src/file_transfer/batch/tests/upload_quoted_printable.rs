use super::{
    Fixture,
    gzip::{encode, metadata},
    multipart::{CONTENT_TYPE, field, quiesce},
    upload::{request, settings},
    upload_errors::{collect, response_limits, upload},
};
use crate::{
    file_transfer::{
        FileStreamError, UploadBodyError, UploadHttpError, UploadRequest, UploadRequestError,
    },
    upload_response::{Error, Limits},
};
use axum::{
    body::{Body, Bytes},
    http::HeaderValue,
};
use futures_util::{StreamExt, stream};
use http_body_util::BodyExt;
use sandproof_guest_runtime::multipart::Error as DecodeError;
use std::{task::Poll, time::Duration};

fn partial(data: &[u8]) -> Vec<u8> {
    let mut wire = field("file", Some("partial"), data, true);
    wire.truncate(wire.len() - 2);
    wire
}
fn form(data: &[u8], compressed: bool) -> UploadRequest {
    request(
        &if compressed {
            encode(data)
        } else {
            data.to_vec()
        },
        compressed,
        CONTENT_TYPE,
        None,
    )
}
async fn check(f: &Fixture, request: UploadRequest, message: &str) {
    let response = upload(f, request, response_limits()).await.unwrap();
    assert_eq!(response.status(), 500);
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let value: serde_json::Value =
        serde_json::from_slice(&collect(response.into_body()).await).unwrap();
    assert_eq!(value, serde_json::json!({"code":500,"message":message}));
    quiesce(f).await;
}
#[tokio::test]
async fn qp_failures_keep_prior_file_prefix_and_old_metadata_for_identity_and_gzip() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    for compressed in [false, true] {
        for (data, message) in [
            (
                b"prefix\0tail".to_vec(),
                "error writing file: quotedprintable: invalid unescaped byte 0x00 in body",
            ),
            (
                b"prefix=\rX".to_vec(),
                "error writing file: quotedprintable: invalid hex byte 0x0d",
            ),
            (b"prefix==".to_vec(), "error writing file: unexpected EOF"),
            (
                b"prefix=\r\r".to_vec(),
                "error writing file: quotedprintable: invalid bytes after =: \"\\r\\r\"",
            ),
            (
                [b"prefix".to_vec(), vec![b' '; 4091]].concat(),
                "error writing file: bufio: buffer full",
            ),
        ] {
            let mut seed = request(b"old", false, "application/octet-stream", Some("partial"));
            seed.headers
                .insert("x-metadata-note", HeaderValue::from_static("old"));
            drop(upload(&f, seed, response_limits()).await.unwrap());
            quiesce(&f).await;
            let input = [
                field("file", Some("first"), b"done", false),
                field("file", Some("partial"), &data, true),
                field("file", Some("never"), b"never", false),
                b"--B--\r\n".to_vec(),
            ]
            .concat();
            check(&f, form(&input, compressed), message).await;
            assert_eq!(
                std::fs::read(root.path().join("partial")).unwrap(),
                b"prefix"
            );
            assert_eq!(
                metadata(&f, &root.path().join("partial"))
                    .get("note")
                    .unwrap(),
                "old"
            );
            assert_eq!(std::fs::read(root.path().join("first")).unwrap(), b"done");
            assert_eq!(
                metadata(&f, &root.path().join("first"))
                    .get("note")
                    .unwrap(),
                "kept"
            );
            assert!(!root.path().join("never").exists());
        }
    }
}
#[tokio::test]
async fn complete_bad_line_stops_polling_and_actual_source_failure_uses_abort() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let mut req = form(b"", false);
    req.expected = None;
    let first = stream::iter([Ok::<_, std::io::Error>(Bytes::from(partial(
        b"prefix\0tail\n",
    )))]);
    let tail = stream::poll_fn(|_| -> Poll<Option<Result<Bytes, std::io::Error>>> {
        panic!("polled after QP failed")
    });
    req.body = Body::from_stream(first.chain(tail));
    check(
        &f,
        req,
        "error writing file: quotedprintable: invalid unescaped byte 0x00 in body",
    )
    .await;
    let mut req = form(b"", false);
    req.expected = None;
    req.body = Body::from_stream(stream::iter([
        Ok(Bytes::from(partial(b"prefix="))),
        Err(std::io::Error::other("actual source failure")),
    ]));
    check(
        &f,
        req,
        "error writing file: quotedprintable: invalid bytes after =: \"\"",
    )
    .await;
    assert_eq!(
        std::fs::read(root.path().join("partial")).unwrap(),
        b"prefix"
    );
}
#[tokio::test]
async fn local_failures_and_write_budget_keep_identity_and_legacy_api_is_coarse() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    for budget in [false, true] {
        let input = partial(b"prefix=");
        let mut req = form(&input, false);
        if budget {
            req.expected = None;
            req.body = Body::from_stream(stream::iter([
                Ok(Bytes::from(input)),
                Err(crate::upload_body::GzipError::InputLimit),
            ]));
        } else {
            req.expected = Some(input.len() as u64 + 1);
        }
        let expected = if budget {
            UploadBodyError::InputLimit
        } else {
            UploadBodyError::Transport
        };
        assert!(
            matches!(upload(&f,req,response_limits()).await,Err(UploadHttpError::Request(UploadRequestError::Body(error))) if error == expected)
        );
        quiesce(&f).await;
    }
    let input = [partial(b"prefix\0tail"), b"\r\n--B--\r\n".to_vec()].concat();
    let mut limits = settings();
    limits.batch.total_bytes = 2;
    assert!(matches!(
        f.streams
            .upload_request_response(
                f.session.authorize(Some("token")).await.unwrap(),
                form(&input, false),
                &f.accounts,
                &f.settings,
                limits,
                response_limits()
            )
            .await,
        Err(UploadHttpError::Request(UploadRequestError::Body(
            UploadBodyError::Transfer(FileStreamError::ResourceExhausted)
        )))
    ));
    quiesce(&f).await;
    assert!(matches!(
        f.streams
            .upload_request(
                f.session.authorize(Some("token")).await.unwrap(),
                form(&input, false),
                &f.accounts,
                &f.settings,
                settings()
            )
            .await,
        Err(UploadRequestError::Body(UploadBodyError::Decode(
            DecodeError::Malformed
        )))
    ));
    quiesce(&f).await;
}
#[tokio::test]
async fn qp_response_keeps_original_slot_deadline_and_final_clone() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let input = partial(b"prefix=");
    let response = upload(&f, form(&input, false), response_limits())
        .await
        .unwrap();
    let mut body = response.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = bytes.clone();
    drop(bytes);
    assert!(matches!(
        upload(&f, form(b"", false), response_limits()).await,
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
            form(&input, false),
            Limits {
                wire_bytes: 1,
                ..response_limits()
            }
        )
        .await,
        Err(UploadHttpError::Response(Error::ResponseLimit))
    ));
    quiesce(&f).await;
    assert_eq!(
        std::fs::read(root.path().join("partial")).unwrap(),
        b"prefix"
    );
}

#[tokio::test]
async fn qp_last_response_byte_settles_workers_and_allows_the_next_single_slot_request() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    for compressed in [false, true] {
        let input = partial(b"prefix\0tail\n");
        let response = upload(&f, form(&input, compressed), response_limits())
            .await
            .unwrap();
        assert_eq!(response.status(), 500);
        let length: usize = response.headers()["content-length"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        let mut body = response.into_body();
        let mut received = 0;
        while received < length {
            let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
            received += bytes.len();
            let copy = bytes.clone();
            drop(bytes);
            assert_eq!(f.streams.slots.available_permits(), 0);
            drop(copy);
        }
        // No EOF poll, sleep, retry or quiesce: model a known-length HTTP reply.
        assert_eq!(f.streams.slots.available_permits(), 1);
        assert_eq!(
            std::fs::read(root.path().join("partial")).unwrap(),
            b"prefix"
        );
        drop(f.batch(None, super::limits()).await);
        assert!(body.frame().await.is_none());
    }
}
