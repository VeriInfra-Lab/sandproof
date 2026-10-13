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
use http_body_util::BodyExt;
use sandproof_guest_runtime::multipart::Error as DecodeError;
use std::time::Duration;

fn partial(name: &str, data: &[u8], encoded: bool) -> Vec<u8> {
    let mut input = field(name, Some("partial"), data, encoded);
    input.truncate(input.len() - 2);
    input
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
async fn check(f: &Fixture, req: UploadRequest, message: &str) {
    let response = upload(f, req, response_limits()).await.unwrap();
    assert_eq!(response.status(), 500);
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let actual: serde_json::Value =
        serde_json::from_slice(&collect(response.into_body()).await).unwrap();
    assert_eq!(actual, serde_json::json!({"code":500, "message":message}));
    quiesce(f).await;
}

#[tokio::test]
async fn truncated_file_keeps_decoded_prefix_and_old_metadata_after_completed_file() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    for compressed in [false, true] {
        for qp in [false, true] {
            let mut seed = request(b"old", false, "application/octet-stream", Some("partial"));
            seed.headers
                .insert("x-metadata-note", HeaderValue::from_static("old"));
            drop(upload(&f, seed, response_limits()).await.unwrap());
            quiesce(&f).await;
            let input = [
                field("file", Some("first"), b"done", false),
                partial("file", if qp { b"prefix=00" } else { b"prefix\0" }, qp),
            ]
            .concat();
            check(
                &f,
                form(&input, compressed),
                "error writing file: unexpected EOF",
            )
            .await;
            assert_eq!(
                std::fs::read(root.path().join("partial")).unwrap(),
                b"prefix\0"
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
        }
    }
}

#[tokio::test]
async fn next_part_failures_follow_completed_file_or_ignored_field() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    for compressed in [false, true] {
        for input in [b"preamble".to_vec(), partial("ignored", b"discard", false)] {
            check(
                &f,
                form(&input, compressed),
                "error reading form: multipart: NextPart: EOF",
            )
            .await;
        }
        assert!(!root.path().join("partial").exists());
        for separated in [false, true] {
            let prefix = if separated {
                b"done\r\n".as_slice()
            } else {
                b""
            };
            let input = [
                partial("file", prefix, false),
                b"--B \"\xff\r\n".to_vec(),
                field("file", Some("never"), b"never", false),
                b"--B--\r\n".to_vec(),
            ]
            .concat();
            let message = if separated {
                "error reading form: multipart: expecting a new Part; got line \"--B \\\"\\xff\\r\\n\""
            } else {
                "error reading form: multipart: unexpected line in Next(): \"--B \\\"\\xff\\r\\n\""
            };
            check(&f, form(&input, compressed), message).await;
            assert_eq!(
                std::fs::read(root.path().join("partial")).unwrap(),
                if separated { b"done".as_slice() } else { b"" }
            );
            assert_eq!(
                metadata(&f, &root.path().join("partial"))
                    .get("note")
                    .unwrap(),
                "kept"
            );
            assert!(!root.path().join("never").exists());
            std::fs::remove_file(root.path().join("partial")).unwrap();
        }
    }
}

#[tokio::test]
async fn qp_header_transport_and_budget_errors_are_not_labeled_as_framing() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    for (bytes, expected) in [
        (b"--B\r\n".to_vec(), DecodeError::TruncatedHeaders),
        (partial("file", b"prefix=", true), DecodeError::Malformed),
    ] {
        assert!(
            matches!(upload(&f, form(&bytes, false), response_limits()).await,
            Err(UploadHttpError::Request(UploadRequestError::Body(UploadBodyError::Decode(error)))) if error == expected)
        );
        quiesce(&f).await;
    }
    assert_eq!(
        std::fs::read(root.path().join("partial")).unwrap(),
        b"prefix"
    );
    let mut req = form(b"", false);
    req.body = Body::from_stream(futures_util::stream::iter([Err::<Bytes, _>(
        std::io::Error::other("source"),
    )]));
    assert!(matches!(
        upload(&f, req, response_limits()).await,
        Err(UploadHttpError::Request(UploadRequestError::Body(
            UploadBodyError::Transport
        )))
    ));
    quiesce(&f).await;
    let mut limits = settings();
    limits.multipart.wire.line_bytes = 2;
    let result = f
        .streams
        .upload_request_response(
            f.session.authorize(Some("token")).await.unwrap(),
            form(b"long\r\n", false),
            &f.accounts,
            &f.settings,
            limits,
            response_limits(),
        )
        .await;
    assert!(matches!(
        result,
        Err(UploadHttpError::Request(UploadRequestError::Body(
            UploadBodyError::Decode(DecodeError::ResourceExhausted)
        )))
    ));
    quiesce(&f).await;
    for input in [b"preamble".to_vec(), partial("file", b"prefix", false)] {
        let result = f
            .streams
            .upload_request(
                f.session.authorize(Some("token")).await.unwrap(),
                form(&input, false),
                &f.accounts,
                &f.settings,
                settings(),
            )
            .await;
        assert!(matches!(
            result,
            Err(UploadRequestError::Body(UploadBodyError::Decode(
                DecodeError::Truncated
            )))
        ));
        quiesce(&f).await;
    }
}

#[tokio::test]
async fn framing_response_keeps_original_slot_deadline_and_final_clone() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let input = partial("file", b"prefix", false);
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
