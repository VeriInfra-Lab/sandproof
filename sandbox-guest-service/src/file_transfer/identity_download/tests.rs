use super::*;
use crate::file_transfer::tests::{context, pool, session};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use futures_util::FutureExt;
use http_body_util::BodyExt;
use sandproof_guest_runtime::file_download::{self as wire, date::Utc};
use std::{fs, io::Write, time::Duration};
fn limits() -> Limits {
    Limits {
        request: wire::Limits {
            header_bytes: 8192,
            items: 32,
        },
        response_header_bytes: 8192,
        wire_bytes: 8192,
    }
}
fn request(range: &'static str) -> IdentityRequest {
    let mut headers = HeaderMap::new();
    headers.insert("range", HeaderValue::from_static(range));
    IdentityRequest {
        method: Method::GET,
        headers,
        response: HeaderMap::new(),
        content_type: HeaderValue::from_static("application/octet-stream").into(),
        disposition_path: None,
        boundary: Some("a".repeat(60)),
    }
}
async fn collect(mut download: IdentityDownload) -> Result<Vec<u8>, DownloadError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = download.next().await? {
        bytes.extend(chunk.as_ref());
    }
    Ok(bytes)
}
async fn quiesce(pool: &FileStreams) {
    drop(
        tokio::time::timeout(Duration::from_secs(5), pool.slots.acquire())
            .await
            .unwrap()
            .unwrap(),
    );
}

#[tokio::test]
async fn actual_descriptor_ranges_empty_binary_and_conditional_responses() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    let session = session().await;
    let pool = pool();
    for (data, range, expected) in [
        (b"".as_slice(), "", b"".as_slice()),
        (b"\0\xff123", "bytes=0-1", b"\0\xff"),
        (b"abcdef", "bytes=-2", b"ef"),
    ] {
        fs::write(&path, data).unwrap();
        let download = pool
            .download_identity(
                session.authorize(Some("token")).await.unwrap(),
                context(&path, root.path()),
                request(range),
                limits(),
                Utc,
            )
            .await
            .unwrap();
        assert_eq!(collect(download).await.unwrap(), expected);
        quiesce(&pool).await;
    }
    let mut req = request("bytes=bad");
    req.headers
        .insert("if-none-match", HeaderValue::from_static("*"));
    let download = pool
        .download_identity(
            session.authorize(Some("token")).await.unwrap(),
            context(&path, root.path()),
            req,
            limits(),
            Utc,
        )
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::NOT_MODIFIED);
    assert!(collect(download).await.unwrap().is_empty());
    quiesce(&pool).await;
    let mut req = request("bytes=1-2");
    req.method = Method::HEAD;
    let download = pool
        .download_identity(
            session.authorize(Some("token")).await.unwrap(),
            context(&path, root.path()),
            req,
            limits(),
            Utc,
        )
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(download.headers()["content-length"], "2");
    assert!(collect(download).await.unwrap().is_empty());
}

#[tokio::test]
async fn descriptor_replacement_growth_and_shrink_do_not_change_planned_identity() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    let session = session().await;
    let pool = pool();
    for action in ["replace", "grow", "shrink"] {
        fs::write(&path, b"01234").unwrap();
        let download = pool
            .download_identity(
                session.authorize(Some("token")).await.unwrap(),
                context(&path, root.path()),
                request(""),
                limits(),
                Utc,
            )
            .await
            .unwrap();
        match action {
            "replace" => {
                fs::rename(&path, root.path().join("old")).unwrap();
                fs::write(&path, b"other").unwrap();
            }
            "grow" => {
                fs::OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .unwrap()
                    .write_all(b"tail")
                    .unwrap();
            }
            _ => fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(2)
                .unwrap(),
        }
        let result = collect(download).await;
        if action == "shrink" {
            assert_eq!(result, Err(DownloadError::Incomplete));
        } else {
            assert_eq!(result.unwrap(), b"01234");
        }
        quiesce(&pool).await;
    }
    fs::write(&path, b"01234").unwrap();
    let download = pool
        .download_identity(
            session.authorize(Some("token")).await.unwrap(),
            context(&path, root.path()),
            request("bytes=0-0,3-4"),
            limits(),
            Utc,
        )
        .await
        .unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(4)
        .unwrap();
    assert_eq!(collect(download).await, Err(DownloadError::Incomplete));
}

#[tokio::test]
async fn response_bytes_keep_original_auth_and_capacity_after_body_drop() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    fs::write(&path, b"01234").unwrap();
    let session = session().await;
    let pool = pool();
    let download = pool
        .download_identity(
            session.authorize(Some("token")).await.unwrap(),
            context(&path, root.path()),
            request("bytes=0-0,3-4"),
            limits(),
            Utc,
        )
        .await
        .unwrap();
    let mut body = download.into_response().into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let clone = bytes.clone();
    drop(body);
    drop(bytes);
    assert_eq!(pool.slots.available_permits(), 0);
    let mut init = Box::pin(session.initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#));
    assert!(init.as_mut().now_or_never().is_none());
    drop(clone);
    quiesce(&pool).await;
    init.await.unwrap();
}

#[tokio::test]
async fn selected_file_budget_and_open_errors_preserve_admission() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    let session = session().await;
    let pool = pool();
    for (target, expected) in [
        (root.path(), DownloadError::SourceInvalid),
        (path.as_path(), DownloadError::SourceNotFound),
    ] {
        assert!(
            matches!(pool.download_identity(session.authorize(Some("token")).await.unwrap(), context(target, root.path()), request(""), limits(), Utc).await, Err(IdentityDownloadError::Download(error)) if error == expected)
        );
        quiesce(&pool).await;
    }
    fs::write(&path, vec![42; 1025]).unwrap();
    assert!(matches!(
        pool.download_identity(
            session.authorize(Some("token")).await.unwrap(),
            context(&path, root.path()),
            request(""),
            limits(),
            Utc
        )
        .await,
        Err(IdentityDownloadError::Download(
            DownloadError::ResourceExhausted
        ))
    ));
    quiesce(&pool).await;
    let download = pool
        .download_identity(
            session.authorize(Some("token")).await.unwrap(),
            context(&path, root.path()),
            request("bytes=0-1"),
            limits(),
            Utc,
        )
        .await
        .unwrap();
    assert_eq!(collect(download).await.unwrap(), [42, 42]);
}

#[tokio::test]
async fn mime_sniffs_original_descriptor_and_formats_resolved_filename() {
    use crate::file_download::ContentType;
    use sandproof_guest_runtime::file_mime::{Catalog, CatalogLimits};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("雪.jpg");
    fs::write(&path, b"<!DOCTYPE HTML><body>hello</body>").unwrap();
    let session = session().await;
    let pool = pool();
    let catalog = Arc::new(
        Catalog::from_sources(
            [None; 2],
            [None; 4],
            CatalogLimits {
                input_bytes: 8192,
                line_bytes: 1024,
                entries: 256,
                retained_bytes: 8192,
                value_bytes: 1024,
            },
        )
        .unwrap(),
    );
    for (original_path, expected_type) in [
        ("query.CSS", "text/css; charset=utf-8"),
        ("query.unknown", "text/html; charset=utf-8"),
    ] {
        let mut req = request("bytes=0-3");
        req.content_type = ContentType::Detect {
            original_path: original_path.into(),
            catalog: Arc::clone(&catalog),
        };
        let download = pool
            .download_identity(
                session.authorize(Some("token")).await.unwrap(),
                context(&path, root.path()),
                req,
                limits(),
                Utc,
            )
            .await
            .unwrap();
        assert_eq!(download.headers()["content-type"], expected_type);
        assert_eq!(
            download.headers()["content-disposition"],
            "inline; filename*=utf-8''%E9%9B%AA.jpg"
        );
        assert_eq!(collect(download).await.unwrap(), b"<!DO");
        quiesce(&pool).await;
    }
}
