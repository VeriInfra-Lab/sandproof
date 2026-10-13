use super::{
    Fixture,
    gzip::{encode, metadata},
    multipart::{CONTENT_TYPE, field, quiesce},
    upload::{request, settings},
    upload_errors::{collect, response_limits, upload},
};
use crate::{
    file_transfer::{FileStreamError, UploadBodyError, UploadHttpError, UploadRequestError},
    upload_body::GzipError,
    upload_response::{Error, Limits},
};
use axum::{
    body::{Body, Bytes},
    http::HeaderValue,
};
use http_body_util::BodyExt;
use std::time::Duration;

fn damaged() -> Vec<u8> {
    let mut bytes = encode(b"written-prefix");
    let trailer = bytes.len() - 8;
    bytes[trailer] ^= 1;
    bytes
}

#[tokio::test]
async fn body_errors_send_write_stage_diagnostics_keep_prefix_and_do_not_finish_metadata() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let mut truncated = encode(b"written-prefix");
    truncated.truncate(truncated.len() - 8);
    for (input, message) in [
        (damaged(), "gzip: invalid checksum"),
        (truncated, "unexpected EOF"),
        (
            [encode(b"written-prefix"), b"invalid next member".to_vec()].concat(),
            "gzip: invalid header",
        ),
    ] {
        let mut seed = request(b"old", false, "application/octet-stream", Some("file"));
        seed.headers
            .insert("x-metadata-note", HeaderValue::from_static("old"));
        drop(upload(&f, seed, response_limits()).await.unwrap());
        quiesce(&f).await;
        let response = upload(
            &f,
            request(&input, true, "application/octet-stream", Some("file")),
            response_limits(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), 500);
        let actual: serde_json::Value =
            serde_json::from_slice(&collect(response.into_body()).await).unwrap();
        assert_eq!(
            actual,
            serde_json::json!({"code":500,"message":format!("error writing file: {message}")})
        );
        quiesce(&f).await;
        assert_eq!(
            std::fs::read(root.path().join("file")).unwrap(),
            b"written-prefix"
        );
        assert_eq!(
            metadata(&f, &root.path().join("file")).get("note").unwrap(),
            "old"
        );
    }
}

#[tokio::test]
async fn multipart_and_identity_sources_do_not_receive_raw_gzip_write_wrapper() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let form = [
        field("file", Some("form"), b"done", false),
        b"--B--\r\n".to_vec(),
    ]
    .concat();
    let mut data = encode(&form);
    let trailer = data.len() - 8;
    data[trailer] ^= 1;
    let result = upload(
        &f,
        request(&data, true, CONTENT_TYPE, None),
        response_limits(),
    )
    .await;
    assert!(matches!(
        result,
        Err(UploadHttpError::Request(UploadRequestError::Body(
            UploadBodyError::Gzip(GzipError::Checksum)
        )))
    ));
    quiesce(&f).await;
    assert_eq!(std::fs::read(root.path().join("form")).unwrap(), b"done");
    assert_eq!(
        metadata(&f, &root.path().join("form")).get("note").unwrap(),
        "kept"
    );
    let mut req = request(b"", false, "application/octet-stream", Some("identity"));
    req.body = Body::from_stream(futures_util::stream::iter([Err::<Bytes, _>(
        GzipError::Checksum,
    )]));
    assert!(matches!(
        upload(&f, req, response_limits()).await,
        Err(UploadHttpError::Request(UploadRequestError::Body(
            UploadBodyError::Gzip(GzipError::Checksum)
        )))
    ));
    quiesce(&f).await;
}

#[tokio::test]
async fn transport_and_output_budgets_stay_explicit_and_legacy_error_is_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    for extra_length in [true, false] {
        let data = encode(b"written-prefix");
        let mut req = request(&data, true, "application/octet-stream", Some("file"));
        let mut limits = settings();
        if extra_length {
            req.expected = Some(data.len() as u64 + 1);
        } else {
            limits.gzip.decode.decoded_bytes = 0;
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
            matches!(result,Err(UploadHttpError::Request(UploadRequestError::Body(error)))
            if error==if extra_length {UploadBodyError::Transport} else {UploadBodyError::Gzip(GzipError::DecodedLimit)})
        );
        quiesce(&f).await;
    }
    let result = f
        .streams
        .upload_request(
            f.session.authorize(Some("token")).await.unwrap(),
            request(&damaged(), true, "application/octet-stream", Some("file")),
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
    assert_eq!(
        std::fs::read(root.path().join("file")).unwrap(),
        b"written-prefix"
    );
}

#[tokio::test]
async fn failed_body_response_retains_original_slot_and_expiry_without_rolling_back() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let response = upload(
        &f,
        request(&damaged(), true, "application/octet-stream", Some("file")),
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
            request(b"never", false, "application/octet-stream", Some("never")),
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
    assert!(matches!(
        upload(
            &f,
            request(&damaged(), true, "application/octet-stream", Some("file")),
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
        std::fs::read(root.path().join("file")).unwrap(),
        b"written-prefix"
    );
    assert!(!root.path().join("never").exists());
}
