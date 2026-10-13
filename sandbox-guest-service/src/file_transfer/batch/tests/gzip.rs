use super::multipart::{CONTENT_TYPE, body, field, gated_body, quiesce, wire_limits};
use super::{Fixture, limits};
use crate::file_transfer::{
    FileStreamError, GzipError, GzipLimits, HttpBodyLimits, UploadBodyError,
};
use axum::body::Bytes;
use sandproof_guest_runtime::{
    file_protocol::Request,
    filesystem::{self, FileResponse, Ownership},
    gzip::{DecodeLimits, EncodeLimits, Encoder},
};
use std::{collections::BTreeMap, path::Path, sync::atomic::Ordering};

fn gzip_limits() -> GzipLimits {
    GzipLimits {
        http: HttpBodyLimits {
            total_bytes: 8192,
            frame_bytes: 4096,
            frames: 8192,
            trailer_bytes: 256,
            trailer_fields: 8,
        },
        decode: DecodeLimits {
            chunk_bytes: 5,
            encoded_bytes: 8192,
            decoded_bytes: 8192,
            header_bytes: 1024,
            members: 16,
        },
    }
}
fn encode(bytes: &[u8]) -> Vec<u8> {
    let mut codec = Encoder::new(
        bytes,
        EncodeLimits {
            chunk_bytes: 17,
            decoded_bytes: 8192,
            encoded_bytes: 8192,
        },
    )
    .unwrap();
    let mut output = Vec::new();
    while let Some(bytes) = codec.next_chunk().unwrap() {
        output.extend(bytes);
    }
    output
}
fn metadata(fixture: &Fixture, path: &Path) -> BTreeMap<String, String> {
    let response = filesystem::execute(
        &Request::Stat(path.to_str().unwrap().into()),
        Ownership { uid: 0, gid: 0 },
        fixture.streams.limits.file,
    )
    .unwrap();
    match response {
        FileResponse::Entry(entry) => entry.metadata,
        _ => panic!("stat did not return an entry"),
    }
}

#[tokio::test]
async fn raw_multi_member_upload_and_compressed_multipart_use_one_batch() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let bytes = [encode(b"\0first"), encode(b""), encode(b"\xfflast")].concat();
    let reply = fixture
        .batch(Some("raw"), limits())
        .await
        .receive_gzip_raw(body(&bytes, 1), Some(bytes.len() as u64), gzip_limits())
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(root.path().join("raw")).unwrap(),
        b"\0first\xfflast"
    );
    assert_eq!(reply.results()[0].written, 11);
    assert_eq!(reply.results()[0].metadata.get("note").unwrap(), "batch");
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.streams.slots.available_permits(), 0);
    drop(reply);
    quiesce(&fixture).await;
    let form = [
        field("ignored", None, b"discard", false),
        field("file", Some("a"), b"A=00=ffB", true),
        field("file", Some("empty"), b"", false),
        b"--B--\r\nepilogue".to_vec(),
    ]
    .concat();
    let bytes = encode(&form);
    let reply = fixture
        .batch(None, limits())
        .await
        .receive_gzip_multipart(
            CONTENT_TYPE,
            body(&bytes, 1),
            Some(bytes.len() as u64),
            gzip_limits(),
            wire_limits(),
        )
        .await
        .unwrap();
    assert_eq!(reply.results().len(), 2);
    assert_eq!(std::fs::read(root.path().join("a")).unwrap(), b"A\0\xffB");
    assert_eq!(std::fs::read(root.path().join("empty")).unwrap(), b"");
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn invalid_header_and_known_compressed_limit_never_open_the_target() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    std::fs::write(root.path().join("existing"), b"unchanged").unwrap();
    for path in ["existing", "new/parent/file"] {
        assert!(matches!(
            fixture
                .batch(Some(path), limits())
                .await
                .receive_gzip_raw(body(b"invalid gzip header", 1), None, gzip_limits())
                .await,
            Err(UploadBodyError::GzipPreflight(GzipError::Header))
        ));
        quiesce(&fixture).await;
    }
    assert_eq!(
        std::fs::read(root.path().join("existing")).unwrap(),
        b"unchanged"
    );
    assert!(!root.path().join("new").exists());
    let bytes = encode(b"data");
    let mut budget = gzip_limits();
    budget.http.total_bytes = 10;
    assert!(matches!(
        fixture
            .batch(Some("new/parent/file"), limits())
            .await
            .receive_gzip_raw(body(&bytes, 1), Some(bytes.len() as u64), budget)
            .await,
        Err(UploadBodyError::GzipPreflight(GzipError::InputLimit))
    ));
    quiesce(&fixture).await;
    assert!(!root.path().join("new").exists());
    assert!(matches!(
        fixture
            .batch(None, limits())
            .await
            .receive_gzip_raw(body(&bytes, 1), None, gzip_limits())
            .await,
        Err(UploadBodyError::Transfer(FileStreamError::InvalidArgument))
    ));
}

#[tokio::test]
async fn raw_decode_failure_preserves_written_prefix_and_previous_metadata() {
    for checksum in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let fixture = Fixture::new(root.path()).await;
        let mut seed = fixture.batch(Some("file"), limits()).await;
        seed.metadata.insert("note".into(), "old".into());
        let mut file = seed.start_file("", Some(3)).await.unwrap();
        file.write(b"old").await.unwrap();
        file.finish().await.unwrap();
        drop(seed.finish().unwrap());
        let mut bytes = encode(b"new-prefix");
        let trailer = bytes.len() - 8;
        if checksum {
            bytes[trailer] ^= 1;
        } else {
            bytes.truncate(trailer);
        }
        let result = fixture
            .batch(Some("file"), limits())
            .await
            .receive_gzip_raw(body(&bytes, 1), None, gzip_limits())
            .await;
        assert!(matches!(
            result,
            Err(UploadBodyError::Gzip(
                GzipError::Checksum | GzipError::Truncated
            ))
        ));
        quiesce(&fixture).await;
        assert_eq!(
            std::fs::read(root.path().join("file")).unwrap(),
            b"new-prefix"
        );
        assert_eq!(
            metadata(&fixture, &root.path().join("file"))
                .get("note")
                .unwrap(),
            "old"
        );
    }
}

#[tokio::test]
async fn gzip_failure_after_mime_end_keeps_completed_files_but_never_batch_success() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let form = [
        field("file", Some("first"), b"first", false),
        field("file", Some("second"), b"second", false),
        b"--B--\r\n".to_vec(),
    ]
    .concat();
    let mut bytes = encode(&form);
    let trailer = bytes.len() - 8;
    bytes[trailer] ^= 1;
    let result = fixture
        .batch(None, limits())
        .await
        .receive_gzip_multipart(
            CONTENT_TYPE,
            body(&bytes, 1),
            None,
            gzip_limits(),
            wire_limits(),
        )
        .await;
    assert!(matches!(
        result,
        Err(UploadBodyError::Gzip(GzipError::Checksum))
    ));
    quiesce(&fixture).await;
    for name in ["first", "second"] {
        assert_eq!(
            std::fs::read(root.path().join(name)).unwrap(),
            name.as_bytes()
        );
        assert_eq!(
            metadata(&fixture, &root.path().join(name))
                .get("note")
                .unwrap(),
            "batch"
        );
    }
}

#[tokio::test]
async fn failed_next_gzip_member_preserves_prior_file_and_partial_current_file() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let mut partial = field("file", Some("partial"), b"prefixX", false);
    partial.truncate(partial.len() - 2);
    let form = [field("file", Some("first"), b"complete", false), partial].concat();
    let bytes = [encode(&form), b"invalid next member".to_vec()].concat();
    let result = fixture
        .batch(None, limits())
        .await
        .receive_gzip_multipart(
            CONTENT_TYPE,
            body(&bytes, 1),
            None,
            gzip_limits(),
            wire_limits(),
        )
        .await;
    assert!(matches!(
        result,
        Err(UploadBodyError::Gzip(GzipError::Header))
    ));
    quiesce(&fixture).await;
    assert_eq!(
        std::fs::read(root.path().join("first")).unwrap(),
        b"complete"
    );
    assert_eq!(
        std::fs::read(root.path().join("partial")).unwrap(),
        b"prefixX"
    );
    assert!(metadata(&fixture, &root.path().join("partial")).is_empty());
}

#[tokio::test]
async fn raw_completion_keeps_auth_capacity_and_metadata_pending_until_http_eof() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let (send, body, notice) = gated_body();
    send.send(Ok(Bytes::from(encode(b"written"))))
        .await
        .unwrap();
    let batch = fixture.batch(Some("file"), limits()).await;
    let task = tokio::spawn(batch.receive_gzip_raw(body, None, gzip_limits()));
    notice.await.unwrap();
    assert_eq!(std::fs::read(root.path().join("file")).unwrap(), b"written");
    assert!(metadata(&fixture, &root.path().join("file")).is_empty());
    assert_eq!(fixture.streams.slots.available_permits(), 0);
    let init = fixture
        .session
        .initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#);
    tokio::pin!(init);
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(init.as_mut().poll(cx).is_pending()))
            .await
    );
    drop(send);
    let reply = task.await.unwrap().unwrap();
    init.await.unwrap();
    assert_eq!(reply.results()[0].metadata.get("note").unwrap(), "batch");
    assert_eq!(fixture.streams.slots.available_permits(), 0);
    drop(reply);
    quiesce(&fixture).await;
}

#[tokio::test]
async fn cancellation_deadline_and_decoded_budget_leave_partial_file_without_finish() {
    for cancelled in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let fixture = Fixture::new(root.path()).await;
        let (send, body, notice) = gated_body();
        send.send(Ok(Bytes::from(encode(b"prefix")))).await.unwrap();
        let task = tokio::spawn(
            fixture
                .batch(Some("file"), limits())
                .await
                .receive_gzip_raw(body, None, gzip_limits()),
        );
        notice.await.unwrap();
        if cancelled {
            task.abort();
            assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        } else {
            tokio::time::pause();
            tokio::time::advance(std::time::Duration::from_secs(31)).await;
            assert!(matches!(
                task.await.unwrap(),
                Err(UploadBodyError::Transfer(FileStreamError::DeadlineExceeded))
            ));
            tokio::time::resume();
        }
        drop(send);
        quiesce(&fixture).await;
        assert_eq!(std::fs::read(root.path().join("file")).unwrap(), b"prefix");
        assert!(metadata(&fixture, &root.path().join("file")).is_empty());
    }
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let mut budget = gzip_limits();
    budget.decode.decoded_bytes = 5;
    assert!(matches!(
        fixture
            .batch(Some("file"), limits())
            .await
            .receive_gzip_raw(body(&encode(b"1234567"), 1), None, budget)
            .await,
        Err(UploadBodyError::Gzip(GzipError::DecodedLimit))
    ));
    quiesce(&fixture).await;
    assert_eq!(std::fs::read(root.path().join("file")).unwrap(), b"12345");
    assert!(metadata(&fixture, &root.path().join("file")).is_empty());
}
