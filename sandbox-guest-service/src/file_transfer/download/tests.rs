use super::*;
use crate::file_transfer::tests::{context, pool, session};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use sandproof_guest_runtime::{
    file_download::{self as wire, date::Utc},
    file_encoding,
};
use std::fs;
mod request;
fn request(range: &'static str, encoding: &'static str) -> IdentityRequest {
    let mut headers = HeaderMap::new();
    headers.insert("range", HeaderValue::from_static(range));
    headers.insert("accept-encoding", HeaderValue::from_static(encoding));
    IdentityRequest {
        method: Method::GET,
        headers,
        response: HeaderMap::new(),
        content_type: HeaderValue::from_static("text/plain; charset=utf-8").into(),
        disposition_path: None,
        boundary: Some("a".repeat(60)),
    }
}
fn limits() -> DownloadLimits {
    DownloadLimits {
        response: Limits {
            request: wire::Limits {
                header_bytes: 8192,
                items: 32,
            },
            response_header_bytes: 8192,
            wire_bytes: 8192,
        },
        encoding: file_encoding::Limits {
            header_bytes: 8192,
            items: 32,
        },
        compressed_bytes: 8192,
    }
}
#[tokio::test]
async fn filesystem_rejection_precedes_encoding_and_range_fallback_reads_real_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    let session = session().await;
    let pool = pool();
    for (target, expected) in [
        (path.as_path(), StatusCode::NOT_FOUND),
        (root.path(), StatusCode::BAD_REQUEST),
    ] {
        let mut download = pool
            .download_negotiated(
                session.authorize(Some("token")).await.unwrap(),
                context(target, root.path()),
                request("", "identity;q=0"),
                limits(),
                Utc,
            )
            .await
            .unwrap();
        assert_eq!(download.status(), expected);
        assert_eq!(
            download.headers()["content-type"],
            "application/json; charset=utf-8"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = download.next().await.unwrap() {
            bytes.extend_from_slice(chunk.as_ref());
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["code"], expected.as_u16());
        assert_eq!(pool.slots.available_permits(), 0);
        drop(download);
        drop(pool.slots.acquire().await.unwrap());
    }
    fs::write(&path, b"abcdef").unwrap();
    let mut download = pool
        .download_negotiated(
            session.authorize(Some("token")).await.unwrap(),
            context(&path, root.path()),
            request("bytes=1-3", "gzip"),
            limits(),
            Utc,
        )
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(download.headers()["vary"], "Accept-Encoding");
    assert!(!download.headers().contains_key("content-encoding"));
    let mut body = Vec::<u8>::new();
    while let Some(chunk) = download.next().await.unwrap() {
        body.extend(chunk.as_ref());
    }
    assert_eq!(body, b"bcd");
    assert_eq!(pool.slots.available_permits(), 0);
    drop(download);
    drop(pool.slots.acquire().await.unwrap());
}

#[tokio::test]
async fn real_stat_errno_json_does_not_treat_not_directory_as_missing() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("<quoted>&file");
    fs::write(&file, b"data").unwrap();
    let cycle = root.path().join("loop");
    std::os::unix::fs::symlink(&cycle, &cycle).unwrap();
    let session = session().await;
    let pool = pool();
    for (path, text) in [
        (file.join("child"), "not a directory"),
        (cycle, "too many levels of symbolic links"),
    ] {
        for head in [false, true] {
            let mut req = request("", "identity;q=0");
            if head {
                req.method = Method::HEAD;
            }
            let mut download = pool
                .download_negotiated(
                    session.authorize(Some("token")).await.unwrap(),
                    context(&path, root.path()),
                    req,
                    limits(),
                    Utc,
                )
                .await
                .unwrap();
            assert_eq!(download.status(), StatusCode::INTERNAL_SERVER_ERROR);
            assert!(!download.headers().contains_key("vary"));
            let mut bytes = Vec::new();
            while let Some(chunk) = download.next().await.unwrap() {
                bytes.extend_from_slice(chunk.as_ref());
            }
            if head {
                assert!(bytes.is_empty());
            } else {
                let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(value["code"], 500);
                assert_eq!(
                    value["message"],
                    format!(
                        "error checking if path exists '{}': stat {}: {text}",
                        path.display(),
                        path.display()
                    )
                );
            }
            assert_eq!(pool.slots.available_permits(), 0);
            drop(download);
            drop(pool.slots.acquire().await.unwrap());
        }
    }
}
