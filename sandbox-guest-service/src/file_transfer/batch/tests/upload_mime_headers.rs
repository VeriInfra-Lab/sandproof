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
use axum::body::{Body, Bytes};
use futures_util::{StreamExt, stream};
use http_body_util::BodyExt;
use sandproof_guest_runtime::multipart::Error as DecodeError;
use std::{task::Poll, time::Duration};

fn form(bytes: &[u8], compressed: bool) -> UploadRequest {
    request(
        &if compressed {
            encode(bytes)
        } else {
            bytes.to_vec()
        },
        compressed,
        CONTENT_TYPE,
        None,
    )
}
fn bad_header(header: &[u8]) -> Vec<u8> {
    [b"--B\r\n".as_slice(), header,
        b"\r\nContent-Disposition: form-data; name=\"file\"; filename=\"never\"\r\n\r\nnever\r\n--B--\r\n"].concat()
}

#[tokio::test]
async fn header_failures_keep_prior_file_metadata_and_never_open_next_file() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    for compressed in [false, true] {
        for (header, message) in [
            (
                b" Bad: x".to_vec(),
                "error reading form: malformed MIME header initial line: \" Bad: x\"",
            ),
            (
                b"NoColon \t".to_vec(),
                "error reading form: malformed MIME header: missing colon: \"NoColon \\t\"",
            ),
            (
                b"x-key: good\r\n bad\0".to_vec(),
                "error reading form: malformed MIME header line: \"X-Key: good bad\\x00\"",
            ),
            (
                [b" ".as_slice(), &[b'x'; 80]].concat(),
                "error reading form: multipart: message too large",
            ),
        ] {
            let input = [
                field("file", Some("first"), b"done", false),
                bad_header(&header),
            ]
            .concat();
            let response = upload(&f, form(&input, compressed), response_limits())
                .await
                .unwrap();
            assert_eq!(response.status(), 500);
            let actual: serde_json::Value =
                serde_json::from_slice(&collect(response.into_body()).await).unwrap();
            assert_eq!(actual, serde_json::json!({"code":500,"message":message}));
            quiesce(&f).await;
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
async fn physical_error_stops_polling_and_actual_source_failure_finishes_header_validation() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let mut req = form(b"", false);
    req.expected = None;
    let first = stream::iter([Ok::<_, std::io::Error>(Bytes::from_static(
        b"--B\r\nNoColon\r\n",
    ))]);
    let tail = stream::poll_fn(|_| -> Poll<Option<Result<Bytes, std::io::Error>>> {
        panic!("read past a complete missing-colon error")
    });
    req.body = Body::from_stream(first.chain(tail));
    let response = upload(&f, req, response_limits()).await.unwrap();
    assert_eq!(response.status(), 500);
    drop(response);
    quiesce(&f).await;
    let mut req = form(b"", false);
    req.expected = None;
    req.body = Body::from_stream(stream::iter([
        Ok(Bytes::from_static(b"--B\r\nx<: unfinished\r\n")),
        Err(std::io::Error::other(
            "source failure before next header byte",
        )),
    ]));
    let response = upload(&f, req, response_limits()).await.unwrap();
    let actual: serde_json::Value =
        serde_json::from_slice(&collect(response.into_body()).await).unwrap();
    assert_eq!(
        actual,
        serde_json::json!({"code":500,"message":"error reading form: malformed MIME header line: \"x<: unfinished\""})
    );
    quiesce(&f).await;
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn source_budgets_unsupported_text_and_valid_header_eof_remain_component_failures() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    for (input, expected) in [
        (b"--B\r\nX: valid".to_vec(), DecodeError::TruncatedHeaders),
        (b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"\xff\"\r\n\r\npayload\r\n--B--\r\n".to_vec(), DecodeError::Malformed),
    ] {
        assert!(matches!(upload(&f, form(&input, false), response_limits()).await,
            Err(UploadHttpError::Request(UploadRequestError::Body(UploadBodyError::Decode(error)))) if error == expected));
        quiesce(&f).await;
    }
    for budget_error in [false, true] {
        let prefix = b"--B\r\nx<: unfinished\r\n";
        let mut req = form(prefix, false);
        if budget_error {
            req.expected = None;
            req.body = Body::from_stream(stream::iter([
                Ok(Bytes::from_static(prefix)),
                Err(crate::upload_body::GzipError::InputLimit),
            ]));
        } else {
            req.expected = Some(prefix.len() as u64 + 1);
        }
        let expected = if budget_error {
            UploadBodyError::InputLimit
        } else {
            UploadBodyError::Transport
        };
        assert!(matches!(upload(&f, req, response_limits()).await,
            Err(UploadHttpError::Request(UploadRequestError::Body(error))) if error == expected));
        quiesce(&f).await;
    }
    for count_budget in [false, true] {
        let mut limits = settings();
        let input = if count_budget {
            limits.multipart.wire.header_count = 1;
            bad_header(b"X: good")
        } else {
            limits.multipart.wire.header_bytes = 8;
            [b"--B\r\n".as_slice(), &[b'x'; 40]].concat()
        };
        let result = f
            .streams
            .upload_request_response(
                f.session.authorize(Some("token")).await.unwrap(),
                form(&input, false),
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
    }
    let result = f
        .streams
        .upload_request(
            f.session.authorize(Some("token")).await.unwrap(),
            form(b"--B\r\nNoColon", false),
            &f.accounts,
            &f.settings,
            settings(),
        )
        .await;
    assert!(matches!(
        result,
        Err(UploadRequestError::Body(UploadBodyError::Decode(
            DecodeError::Malformed
        )))
    ));
    quiesce(&f).await;
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn header_error_response_retains_original_slot_deadline_and_final_clone() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let input = bad_header(b"NoColon");
    let response = upload(&f, form(&input, false), response_limits())
        .await
        .unwrap();
    let mut body = response.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = bytes.clone();
    drop(bytes);
    assert!(matches!(
        upload(&f, form(&input, false), response_limits()).await,
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
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
