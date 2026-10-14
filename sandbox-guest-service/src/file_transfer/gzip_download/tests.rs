use super::*;
use crate::file_transfer::tests::{context, pool, session};
use futures_util::FutureExt;
use http_body_util::BodyExt;
use sandproof_guest_runtime::gzip::{DecodeLimits, Decoder};
use std::{fs, io::Write, time::Duration};

async fn collect(mut download: GzipDownload) -> Result<Vec<u8>, FileStreamError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = download.next().await? {
        bytes.extend(chunk.as_ref());
    }
    Ok(bytes)
}
fn decode(bytes: &[u8]) -> Vec<u8> {
    let mut codec = Decoder::new(
        bytes,
        DecodeLimits {
            chunk_bytes: 3,
            encoded_bytes: 2048,
            decoded_bytes: 1024,
            header_bytes: 256,
            members: 1,
        },
    )
    .unwrap();
    let mut output = Vec::new();
    while let Some(chunk) = codec.next_chunk().unwrap() {
        output.extend(chunk);
    }
    output
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
async fn gzip_download_uses_real_file_empty_binary_and_actual_eof_after_growth_or_shrink() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    let session = session().await;
    let pool = pool();
    for data in [b"".as_slice(), b"\0\xffbinary", b"abcdef"] {
        fs::write(&path, data).unwrap();
        let download = pool
            .download_gzip(
                session.authorize(Some("token")).await.unwrap(),
                context(&path, root.path()),
                2048,
            )
            .await
            .unwrap();
        assert_eq!(decode(&collect(download).await.unwrap()), data);
        quiesce(&pool).await;
    }
    for shrink in [false, true] {
        fs::write(&path, b"original").unwrap();
        let download = pool
            .download_gzip(
                session.authorize(Some("token")).await.unwrap(),
                context(&path, root.path()),
                2048,
            )
            .await
            .unwrap();
        // Opening does not pre-read: mutate before the first demand.
        if shrink {
            fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(3)
                .unwrap();
        } else {
            fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(b"-tail")
                .unwrap();
        }
        assert_eq!(
            decode(&collect(download).await.unwrap()),
            if shrink {
                b"ori".as_slice()
            } else {
                b"original-tail"
            }
        );
        quiesce(&pool).await;
    }
    fs::write(&path, b"held").unwrap();
    let download = pool
        .download_gzip(
            session.authorize(Some("token")).await.unwrap(),
            context(&path, root.path()),
            2048,
        )
        .await
        .unwrap();
    fs::rename(&path, root.path().join("old")).unwrap();
    fs::write(&path, b"replacement").unwrap();
    assert_eq!(decode(&collect(download).await.unwrap()), b"held");
}

#[tokio::test]
async fn gzip_response_bytes_retain_original_auth_and_capacity_after_handle_drop() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    fs::write(&path, b"content").unwrap();
    let session = session().await;
    let pool = pool();
    let download = pool
        .download_gzip(
            session.authorize(Some("token")).await.unwrap(),
            context(&path, root.path()),
            2048,
        )
        .await
        .unwrap();
    let mut body = download.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = bytes.clone();
    drop(body);
    drop(bytes);
    assert_eq!(pool.slots.available_permits(), 0);
    let mut init = Box::pin(session.initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#));
    assert!(init.as_mut().now_or_never().is_none());
    drop(copy);
    quiesce(&pool).await;
    init.await.unwrap();
}

#[tokio::test]
async fn gzip_open_errors_and_raw_or_encoded_limits_cannot_succeed() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    let session = session().await;
    let pool = pool();
    for (path, expected) in [
        (root.path(), TransferIoError::InvalidArgument),
        (path.as_path(), TransferIoError::NotFound),
    ] {
        assert!(
            matches!(pool.download_gzip(session.authorize(Some("token")).await.unwrap(), context(path, root.path()), 2048).await,
            Err(FileStreamError::Operation(actual)) if actual == expected)
        );
        quiesce(&pool).await;
    }
    for (data, budget) in [(vec![42; 1025], 2048), (vec![], 1)] {
        fs::write(&path, data).unwrap();
        let download = pool
            .download_gzip(
                session.authorize(Some("token")).await.unwrap(),
                context(&path, root.path()),
                budget,
            )
            .await
            .unwrap();
        assert_eq!(
            collect(download).await,
            Err(FileStreamError::ResourceExhausted)
        );
        quiesce(&pool).await;
    }
}

#[tokio::test]
async fn gzip_expiry_after_open_uses_original_deadline_and_retained_handle_keeps_slot() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    fs::write(&path, b"content").unwrap();
    let session = session().await;
    let pool = pool();
    let mut download = pool
        .download_gzip(
            session.authorize(Some("token")).await.unwrap(),
            context(&path, root.path()),
            2048,
        )
        .await
        .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    assert!(matches!(
        download.next().await,
        Err(FileStreamError::DeadlineExceeded)
    ));
    assert_eq!(pool.slots.available_permits(), 0);
    drop(download);
    tokio::time::resume();
    quiesce(&pool).await;
}
