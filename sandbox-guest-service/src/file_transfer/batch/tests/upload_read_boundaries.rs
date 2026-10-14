use super::{
    Fixture,
    gzip::metadata,
    multipart::{CONTENT_TYPE, body, field, quiesce},
    upload::{request, settings},
    upload_errors::{collect, response_limits},
};
use crate::file_transfer::{FileStreams, UploadHttpError, UploadLimits, UploadRequest};
use axum::{
    body::{Body, Bytes},
    http::HeaderValue,
};
use futures_util::{StreamExt, stream};
use std::{path::Path, task::Poll, time::Duration};

async fn fixture(root: &Path) -> Fixture {
    let mut f = Fixture::new(root).await;
    let mut limits = f.streams.limits;
    limits.total_bytes = 16384;
    f.streams = FileStreams::new(1, limits, Duration::from_secs(30)).unwrap();
    f
}
fn limits(chunk: usize) -> UploadLimits {
    let mut limits = settings();
    limits.batch.total_bytes = 16384;
    limits.multipart.wire.total_bytes = 16384;
    limits.multipart.wire.line_bytes = 4096;
    limits.multipart.wire.chunk_bytes = chunk;
    limits.multipart.frames = 16384;
    limits
}
async fn upload(
    f: &Fixture,
    req: UploadRequest,
    chunk: usize,
) -> Result<axum::response::Response, UploadHttpError> {
    f.streams
        .upload_request_response(
            f.session.authorize(Some("token")).await.unwrap(),
            req,
            &f.accounts,
            &f.settings,
            limits(chunk),
            response_limits(),
        )
        .await
}
fn head() -> Vec<u8> {
    let mut head = field("file", Some("partial"), b"", true);
    head.truncate(head.len() - 2);
    head
}
#[tokio::test]
async fn actual_source_reads_control_full_line_completion_and_metadata() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path()).await;
    let head = head();
    for offset in [head.len(), 4096] {
        for source in [1, 4096] {
            for chunk in [7, 128] {
                let mut seed = request(b"old", false, "application/octet-stream", Some("partial"));
                seed.headers
                    .insert("x-metadata-note", HeaderValue::from_static("old"));
                drop(upload(&f, seed, chunk).await.unwrap());
                quiesce(&f).await;
                let bytes = [
                    if offset == head.len() {
                        vec![]
                    } else {
                        [vec![b'p'; offset - head.len() - 2], b"\r\n".to_vec()].concat()
                    },
                    head.clone(),
                    vec![b'x'; 4096],
                    b"\r\n--B--\r\n".to_vec(),
                ]
                .concat();
                let mut req = request(&bytes, false, CONTENT_TYPE, None);
                req.body = body(&bytes, source);
                let response = upload(&f, req, chunk).await.unwrap();
                let complete = source == 4096 && offset == head.len();
                assert_eq!(response.status(), if complete { 200 } else { 500 });
                let value: serde_json::Value =
                    serde_json::from_slice(&collect(response.into_body()).await).unwrap();
                if complete {
                    assert_eq!(value.as_array().unwrap().len(), 1);
                } else {
                    assert_eq!(
                        value,
                        serde_json::json!({"code":500,"message":"error writing file: bufio: buffer full"})
                    );
                }
                quiesce(&f).await;
                assert_eq!(
                    std::fs::read(root.path().join("partial")).unwrap(),
                    vec![b'x'; 4096]
                );
                assert_eq!(
                    metadata(&f, &root.path().join("partial"))
                        .get("note")
                        .unwrap(),
                    if complete { "kept" } else { "old" }
                );
            }
        }
    }
}
#[tokio::test]
async fn full_line_fails_before_polling_the_next_source_read() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path()).await;
    let mut req = request(b"", false, CONTENT_TYPE, None);
    req.expected = None;
    let reads = stream::iter([
        Ok::<_, std::io::Error>(Bytes::from(head())),
        Ok(Bytes::from(vec![b'x'; 4096])),
    ]);
    let tail = stream::poll_fn(|_| -> Poll<Option<Result<Bytes, std::io::Error>>> {
        panic!("read after full QP physical line")
    });
    req.body = Body::from_stream(reads.chain(tail));
    let response = upload(&f, req, 7).await.unwrap();
    assert_eq!(response.status(), 500);
    let value: serde_json::Value =
        serde_json::from_slice(&collect(response.into_body()).await).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"code":500,"message":"error writing file: bufio: buffer full"})
    );
    quiesce(&f).await;
    assert_eq!(
        std::fs::read(root.path().join("partial")).unwrap(),
        vec![b'x'; 4096]
    );
}
