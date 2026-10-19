use super::{
    Fixture,
    gzip::{encode, gzip_limits, metadata},
    limits,
    multipart::{CONTENT_TYPE, body, field, quiesce, wire_limits},
    upload::{request, settings},
    upload_errors::{collect, response_limits, upload},
};
use crate::{
    file_transfer::{FileStreamError, UploadBodyError, UploadHttpError, UploadRequestError},
    upload_body::GzipError,
    upload_response::{Error, GzipReadError},
};
use axum::{
    body::{Body, Bytes},
    http::HeaderValue,
};
use futures_util::stream;
use http_body_util::BodyExt;
use sandproof_guest_runtime::multipart::SourceStage;
use std::time::Duration;

fn head(name: &str, qp: bool) -> Vec<u8> {
    let mut bytes = field(name, Some("partial"), b"", qp);
    bytes.truncate(bytes.len() - 2);
    bytes
}
fn damage(form: &[u8], fault: usize) -> (Vec<u8>, &'static str) {
    let mut bytes = encode(form);
    let message = match fault {
        0 => {
            let offset = bytes.len() - 8;
            bytes[offset] ^= 1;
            "gzip: invalid checksum"
        }
        1 => {
            bytes.truncate(bytes.len() - 3);
            "unexpected EOF"
        }
        _ => {
            bytes.extend_from_slice(b"invalid next member");
            "gzip: invalid header"
        }
    };
    (bytes, message)
}

#[tokio::test]
async fn stages_preserve_prior_files_current_prefix_and_metadata_completion() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    for fault in 0..3 {
        for (tail, prefix, complete, wrapper) in [
            (
                b"".as_slice(),
                b"prefix".as_slice(),
                false,
                "error writing file: ",
            ),
            (
                b"\r\n--B",
                b"prefix",
                true,
                "error reading form: multipart: NextPart: ",
            ),
            (
                b"\r\n--B-",
                b"prefix\r\n--B-",
                false,
                "error writing file: ",
            ),
            (
                b"\r\n--B\r\nX: valid",
                b"prefix",
                true,
                "error reading form: ",
            ),
        ] {
            for qp in [false, true] {
                let mut seed = request(b"old", false, "application/octet-stream", Some("partial"));
                seed.headers
                    .insert("x-metadata-note", HeaderValue::from_static("old"));
                drop(upload(&f, seed, response_limits()).await.unwrap());
                quiesce(&f).await;
                let form = [
                    field("file", Some("prior"), b"done", false),
                    head("file", qp),
                    b"prefix".to_vec(),
                    tail.to_vec(),
                ]
                .concat();
                let (data, cause) = damage(&form, fault);
                let response = upload(
                    &f,
                    request(&data, true, CONTENT_TYPE, None),
                    response_limits(),
                )
                .await
                .unwrap();
                assert_eq!(response.status(), 500);
                let value: serde_json::Value =
                    serde_json::from_slice(&collect(response.into_body()).await).unwrap();
                assert_eq!(
                    value,
                    serde_json::json!({"code":500,"message":format!("{wrapper}{cause}")})
                );
                quiesce(&f).await;
                assert_eq!(std::fs::read(root.path().join("partial")).unwrap(), prefix);
                assert_eq!(
                    metadata(&f, &root.path().join("partial"))
                        .get("note")
                        .unwrap(),
                    if complete { "kept" } else { "old" }
                );
                assert_eq!(std::fs::read(root.path().join("prior")).unwrap(), b"done");
                assert_eq!(
                    metadata(&f, &root.path().join("prior"))
                        .get("note")
                        .unwrap(),
                    "kept"
                );
            }
        }
    }
}

#[tokio::test]
async fn ignored_fields_headers_and_qp_keep_original_error_precedence() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    for (form, message) in [
        (
            b"preamble".to_vec(),
            "error reading form: multipart: NextPart: gzip: invalid checksum",
        ),
        (
            b"--B\r\nX: valid".to_vec(),
            "error reading form: gzip: invalid checksum",
        ),
        (
            b"--B\r\nx<: invalid".to_vec(),
            "error reading form: malformed MIME header line: \"x<: invalid\"",
        ),
        (
            [head("ignored", false), b"ignored".to_vec()].concat(),
            "error reading form: multipart: NextPart: gzip: invalid checksum",
        ),
        (
            [head("file", true), b"prefix\0bad".to_vec()].concat(),
            "error writing file: quotedprintable: invalid unescaped byte 0x00 in body",
        ),
    ] {
        let (data, _) = damage(&form, 0);
        let response = upload(
            &f,
            request(&data, true, CONTENT_TYPE, None),
            response_limits(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), 500);
        let value: serde_json::Value =
            serde_json::from_slice(&collect(response.into_body()).await).unwrap();
        assert_eq!(value, serde_json::json!({"code":500,"message":message}));
        quiesce(&f).await;
    }
    assert_eq!(
        std::fs::read(root.path().join("partial")).unwrap(),
        b"prefix"
    );
    assert!(metadata(&f, &root.path().join("partial")).is_empty());
}

#[tokio::test]
async fn gzip_stage_requires_gzip_input_and_keeps_legacy_and_budget_errors() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let form = [head("file", false), b"prefix".to_vec()].concat();
    let (data, _) = damage(&form, 0);
    let result = f
        .streams
        .upload_request(
            f.session.authorize(Some("token")).await.unwrap(),
            request(&data, true, CONTENT_TYPE, None),
            &f.accounts,
            &f.settings,
            settings(),
        )
        .await;
    assert!(matches!(
        result,
        Err(UploadRequestError::Body(UploadBodyError::Gzip(
            GzipError::Checksum
        )))
    ));
    quiesce(&f).await;
    let result = f
        .batch(None, limits())
        .await
        .receive_gzip_multipart(
            CONTENT_TYPE,
            body(&data, 1),
            Some(data.len() as u64),
            gzip_limits(),
            wire_limits(),
        )
        .await;
    assert!(matches!(
        result,
        Err(UploadBodyError::MultipartGzip {
            stage: SourceStage::Body,
            error: GzipReadError::Checksum
        })
    ));
    quiesce(&f).await;
    for error in [GzipError::Checksum, GzipError::Transport] {
        let mut req = request(b"", false, CONTENT_TYPE, None);
        req.expected = None;
        req.body = Body::from_stream(stream::iter([Ok(Bytes::from(form.clone())), Err(error)]));
        let result = upload(&f, req, response_limits()).await;
        assert!(
            matches!(result, Err(UploadHttpError::Request(UploadRequestError::Body(actual))) if actual == UploadBodyError::from(error))
        );
        quiesce(&f).await;
    }
    for length in [true, false] {
        let data = encode(&form);
        let mut req = request(&data, true, CONTENT_TYPE, None);
        let mut cfg = settings();
        if length {
            req.expected = Some(data.len() as u64 + 1);
        } else {
            cfg.gzip.decode.decoded_bytes = 0;
        }
        let result = f
            .streams
            .upload_request_response(
                f.session.authorize(Some("token")).await.unwrap(),
                req,
                &f.accounts,
                &f.settings,
                cfg,
                response_limits(),
            )
            .await;
        assert!(
            matches!(result, Err(UploadHttpError::Request(UploadRequestError::Body(actual)))
            if actual == if length { UploadBodyError::Transport } else { UploadBodyError::Gzip(GzipError::DecodedLimit) })
        );
        quiesce(&f).await;
    }
}

#[tokio::test]
async fn multipart_gzip_response_keeps_original_slot_deadline_and_failed_prefix() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let (data, _) = damage(&[head("file", false), b"prefix".to_vec()].concat(), 0);
    let response = upload(
        &f,
        request(&data, true, CONTENT_TYPE, None),
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
            request(&data, true, CONTENT_TYPE, None),
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
    let mut response = response_limits();
    response.wire_bytes = 1;
    assert!(matches!(
        upload(&f, request(&data, true, CONTENT_TYPE, None), response).await,
        Err(UploadHttpError::Response(Error::ResponseLimit))
    ));
    quiesce(&f).await;
    assert_eq!(
        std::fs::read(root.path().join("partial")).unwrap(),
        b"prefix"
    );
}
