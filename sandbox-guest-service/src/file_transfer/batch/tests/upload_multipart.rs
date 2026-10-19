use super::{
    Fixture,
    gzip::encode,
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
use http_body_util::BodyExt;
use std::{sync::atomic::Ordering, time::Duration};

const NAMES: [&str; 4] = ["first'雪", "z", "a", "b"];
fn duplicate(count: usize, compressed: bool) -> UploadRequest {
    let mut input = Vec::new();
    for name in &NAMES[..count] {
        input.extend(field("file", Some(name), b"done", false));
    }
    input.extend(field("file", Some("sub/../first'雪"), b"overwrite", false));
    input.extend(field("file", Some("never"), b"never", false));
    input.extend(b"--B--\r\n");
    if compressed {
        input = encode(&input);
    }
    request(&input, compressed, CONTENT_TYPE, None)
}

#[tokio::test]
async fn resolved_duplicates_report_completed_paths_in_original_order_without_rewriting() {
    for compressed in [false, true] {
        for count in 1..=4 {
            let root = tempfile::tempdir().unwrap();
            let f = Fixture::new(root.path()).await;
            let response = upload(&f, duplicate(count, compressed), response_limits())
                .await
                .unwrap();
            assert_eq!(response.status(), 400);
            assert_eq!(
                response.headers()["content-type"],
                "application/json; charset=utf-8"
            );
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            let mut message = format!(
                "you cannot upload multiple files to the same path '{}' in one upload request, only the first specified file was uploaded",
                root.path().join(NAMES[0]).display()
            );
            if count > 2 {
                let previous: Vec<_> = NAMES[1..count]
                    .iter()
                    .map(|name| root.path().join(name).display().to_string())
                    .collect();
                message.push_str(", also the following files were uploaded: ");
                message.push_str(&previous.join(", "));
            }
            let bytes = collect(response.into_body()).await;
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
                serde_json::json!({"code":400,"message":message})
            );
            assert_eq!(bytes.last(), Some(&b'\n'));
            quiesce(&f).await;
            for name in &NAMES[..count] {
                assert_eq!(std::fs::read(root.path().join(name)).unwrap(), b"done");
            }
            assert!(!root.path().join("never").exists());
            assert!(!root.path().join("sub").exists());
            assert_eq!(f.calls.load(Ordering::SeqCst), 1);
        }
    }
}

#[tokio::test]
async fn missing_filename_is_rejected_only_for_file_parts_without_query_override() {
    for compressed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(root.path()).await;
        for path in [None, Some(""), Some("chosen")] {
            let input = [
                field("ignored", None, b"discard", false),
                field("file", None, b"done", false),
                b"--B--\r\n".to_vec(),
            ]
            .concat();
            let input = if compressed { encode(&input) } else { input };
            let response = upload(
                &f,
                request(&input, compressed, CONTENT_TYPE, path),
                response_limits(),
            )
            .await
            .unwrap();
            assert_eq!(response.status(), if path.is_none() { 400 } else { 200 });
            let actual: serde_json::Value =
                serde_json::from_slice(&collect(response.into_body()).await).unwrap();
            quiesce(&f).await;
            if path.is_none() {
                assert_eq!(
                    actual,
                    serde_json::json!({"code":400,"message":"error getting multipart custom part file name: filename not found in Content-Disposition header"})
                );
                assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
            } else {
                let name = if path == Some("") {
                    "default"
                } else {
                    "chosen"
                };
                assert_eq!(std::fs::read(root.path().join(name)).unwrap(), b"done");
            }
        }
    }
}

#[tokio::test]
async fn duplicate_response_keeps_original_slot_and_expiry_after_file_authorization_ends() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let response = upload(&f, duplicate(3, false), response_limits())
        .await
        .unwrap();
    let mut body = response.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = bytes.clone();
    drop(bytes);
    assert!(matches!(
        upload(&f, duplicate(3, false), response_limits()).await,
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
    assert_eq!(std::fs::read(root.path().join(NAMES[0])).unwrap(), b"done");
    assert!(!root.path().join("never").exists());
}

#[tokio::test]
async fn response_budget_failure_retains_files_and_file_limit_precedes_duplicate() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    assert!(matches!(
        upload(
            &f,
            duplicate(1, false),
            Limits {
                wire_bytes: 1,
                ..response_limits()
            }
        )
        .await,
        Err(UploadHttpError::Response(Error::ResponseLimit))
    ));
    quiesce(&f).await;
    let mut limits = settings();
    limits.batch.files = 1;
    assert!(matches!(
        f.streams
            .upload_request_response(
                f.session.authorize(Some("token")).await.unwrap(),
                duplicate(1, false),
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
    assert_eq!(std::fs::read(root.path().join(NAMES[0])).unwrap(), b"done");
    assert!(!root.path().join("never").exists());
}

#[tokio::test]
async fn legacy_entry_retains_detailed_partial_observations() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(root.path()).await;
    let result = f
        .streams
        .upload_request(
            f.session.authorize(Some("token")).await.unwrap(),
            duplicate(3, false),
            &f.accounts,
            &f.settings,
            settings(),
        )
        .await;
    assert!(
        matches!(result, Err(UploadRequestError::Body(UploadBodyError::Transfer(FileStreamError::DuplicatePath(error))))
        if error.path() == root.path().join(NAMES[0]).to_str().unwrap()
            && error.previous() == [root.path().join("z").display().to_string(), root.path().join("a").display().to_string()])
    );
    quiesce(&f).await;
    let input = [
        field("file", Some("before-missing"), b"done", false),
        field("file", None, b"never", false),
        b"--B--\r\n".to_vec(),
    ]
    .concat();
    let result = f
        .streams
        .upload_request(
            f.session.authorize(Some("token")).await.unwrap(),
            request(&input, false, CONTENT_TYPE, None),
            &f.accounts,
            &f.settings,
            settings(),
        )
        .await;
    assert!(matches!(
        result,
        Err(UploadRequestError::Body(UploadBodyError::MissingFilename))
    ));
    quiesce(&f).await;
    assert_eq!(
        std::fs::read(root.path().join("before-missing")).unwrap(),
        b"done"
    );
    assert!(!root.path().join("default").exists());
}
