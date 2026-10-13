use super::{
    Fixture,
    gzip::{encode, metadata},
    limits,
    multipart::{CONTENT_TYPE, body, field, gated_body, quiesce, wire_limits},
    upload::{request, settings},
    upload_errors::{collect, response_limits},
};
use crate::file_transfer::{
    FileStreamError, MultipartError, UploadBodyError, UploadHttpError, UploadRequestError,
};
use axum::{body::Bytes, http::HeaderValue};

fn form(data: &[u8]) -> Vec<u8> {
    [
        field("file", Some("prior"), b"done", false),
        field("ignored", None, data, true),
        field("file", Some("next"), b"next", false),
        b"--B--\r\n".to_vec(),
    ]
    .concat()
}

#[tokio::test]
async fn ignored_qp_prefetch_controls_following_file_without_rejecting_all_bad_fields() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    for compressed in [false, true] {
        for source in [1, 4096] {
            for chunk in [1, 128] {
                for tail in [false, true] {
                    let mut cfg = settings();
                    cfg.gzip.decode.chunk_bytes = source;
                    cfg.multipart.wire.chunk_bytes = chunk;
                    let mut seed = request(b"old", false, "application/octet-stream", Some("next"));
                    seed.headers
                        .insert("x-metadata-note", HeaderValue::from_static("old"));
                    drop(
                        super::upload_errors::upload(&f, seed, response_limits())
                            .await
                            .unwrap(),
                    );
                    quiesce(&f).await;
                    let data = form(if tail { b"bad\0\nextra\n" } else { b"bad\0\n" });
                    let bytes = if compressed { encode(&data) } else { data };
                    let mut req = request(&bytes, compressed, CONTENT_TYPE, None);
                    req.body = body(&bytes, source);
                    let response = f
                        .streams
                        .upload_request_response(
                            f.session.authorize(Some("token")).await.unwrap(),
                            req,
                            &f.accounts,
                            &f.settings,
                            cfg,
                            response_limits(),
                        )
                        .await
                        .unwrap();
                    let success = !tail || source == 4096;
                    assert_eq!(response.status(), if success { 200 } else { 500 });
                    let value: serde_json::Value =
                        serde_json::from_slice(&collect(response.into_body()).await).unwrap();
                    if success {
                        assert_eq!(value.as_array().unwrap().len(), 2);
                    } else {
                        assert_eq!(
                            value,
                            serde_json::json!({"code":500,"message":"error reading form: multipart: unexpected line in Next(): \"extra\\n\""})
                        );
                    }
                    quiesce(&f).await;
                    assert_eq!(std::fs::read(root.path().join("prior")).unwrap(), b"done");
                    assert_eq!(
                        metadata(&f, &root.path().join("prior"))
                            .get("note")
                            .unwrap(),
                        "kept"
                    );
                    assert_eq!(
                        std::fs::read(root.path().join("next")).unwrap(),
                        if success { b"next".as_slice() } else { b"old" }
                    );
                    assert_eq!(
                        metadata(&f, &root.path().join("next")).get("note").unwrap(),
                        if success { "kept" } else { "old" }
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn ignored_qp_keeps_input_and_response_budgets_explicit() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let data = form(b"bad\0\nextra\n");
    let mut cfg = settings();
    cfg.multipart.wire.total_bytes = 2;
    let result = f
        .streams
        .upload_request_response(
            f.session.authorize(Some("token")).await.unwrap(),
            request(&data, false, CONTENT_TYPE, None),
            &f.accounts,
            &f.settings,
            cfg,
            response_limits(),
        )
        .await;
    assert!(matches!(
        result,
        Err(UploadHttpError::Request(UploadRequestError::Body(
            UploadBodyError::InputLimit
        )))
    ));
    quiesce(&f).await;
    assert!(!root.path().join("prior").exists());
    let mut budget = response_limits();
    budget.wire_bytes = 1;
    let result =
        super::upload_errors::upload(&f, request(&data, false, CONTENT_TYPE, None), budget).await;
    assert!(matches!(
        result,
        Err(UploadHttpError::Response(
            crate::upload_response::Error::ResponseLimit
        ))
    ));
    quiesce(&f).await;
    assert_eq!(std::fs::read(root.path().join("prior")).unwrap(), b"done");
    assert!(!root.path().join("next").exists());
    let result = f
        .streams
        .upload_request(
            f.session.authorize(Some("token")).await.unwrap(),
            request(&data, false, CONTENT_TYPE, None),
            &f.accounts,
            &f.settings,
            settings(),
        )
        .await;
    assert!(matches!(
        result,
        Err(UploadRequestError::Body(UploadBodyError::Decode(
            sandproof_guest_runtime::multipart::Error::Malformed
        )))
    ));
    quiesce(&f).await;
}

#[tokio::test]
async fn waiting_inside_ignored_qp_preserves_original_authorization_until_cancel_or_expiry() {
    for cancelled in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(root.path()).await;
        let (send, body, notice) = gated_body();
        let mut input = [
            field("file", Some("prior"), b"done", false),
            field("ignored", None, b"pending", true),
        ]
        .concat();
        input.truncate(input.len() - 2);
        send.send(Ok(Bytes::from(input))).await.unwrap();
        let task = tokio::spawn(f.batch(None, limits()).await.receive_multipart(
            CONTENT_TYPE,
            body,
            None,
            wire_limits(),
        ));
        notice.await.unwrap();
        assert!(!task.is_finished());
        assert_eq!(f.streams.slots.available_permits(), 0);
        let init = f
            .session
            .initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#);
        tokio::pin!(init);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(init.as_mut().poll(cx).is_pending()))
                .await
        );
        if cancelled {
            task.abort();
            assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        } else {
            tokio::time::pause();
            tokio::time::advance(std::time::Duration::from_secs(31)).await;
            assert!(matches!(
                task.await.unwrap(),
                Err(MultipartError::Transfer(FileStreamError::DeadlineExceeded))
            ));
            tokio::time::resume();
        }
        drop(send);
        init.await.unwrap();
        quiesce(&f).await;
        assert_eq!(std::fs::read(root.path().join("prior")).unwrap(), b"done");
        assert_eq!(
            metadata(&f, &root.path().join("prior"))
                .get("note")
                .unwrap(),
            "batch"
        );
        assert!(!root.path().join("next").exists());
    }
}
