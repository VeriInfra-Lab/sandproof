use super::multipart::quiesce;
use super::*;
use crate::upload_response::{Error, Limits};
use http_body_util::BodyExt;

#[tokio::test]
async fn successful_reply_holds_original_slot_through_response_byte_clones() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let mut batch = fixture.batch(Some("file"), limits()).await;
    let mut part = batch.start_file("", Some(3)).await.unwrap();
    part.write(b"abc").await.unwrap();
    part.finish().await.unwrap();
    let reply = batch.finish().unwrap();
    // File I/O is complete; response retention keeps capacity, not old auth.
    fixture
        .session
        .initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#)
        .await
        .unwrap();
    let response = reply
        .into_http_response(Limits {
            wire_bytes: 4096,
            chunk_bytes: 7,
        })
        .await
        .unwrap();
    let mut body = response.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = bytes.clone();
    drop(bytes);
    drop(body);
    assert_eq!(fixture.streams.slots.available_permits(), 0);
    assert_eq!(std::fs::read(root.path().join("file")).unwrap(), b"abc");
    drop(copy);
    quiesce(&fixture).await;
}
#[tokio::test]
async fn failed_response_budget_preserves_completed_file_and_metadata() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let mut batch = fixture.batch(Some("file"), limits()).await;
    let mut part = batch.start_file("", Some(3)).await.unwrap();
    part.write(b"abc").await.unwrap();
    part.finish().await.unwrap();
    let reply = batch.finish().unwrap();
    let error = reply
        .into_http_response(Limits {
            wire_bytes: 2,
            chunk_bytes: 1,
        })
        .await
        .err()
        .unwrap();
    assert_eq!(error, Error::ResponseLimit);
    quiesce(&fixture).await;
    assert_eq!(std::fs::read(root.path().join("file")).unwrap(), b"abc");
    assert_eq!(
        super::gzip::metadata(&fixture, &root.path().join("file"))
            .get("note")
            .unwrap(),
        "batch"
    );
    let response = fixture
        .batch(None, limits())
        .await
        .finish()
        .unwrap()
        .into_http_response(Limits {
            wire_bytes: 2,
            chunk_bytes: 2,
        })
        .await
        .unwrap();
    let mut body = response.into_body();
    assert_eq!(
        body.frame().await.unwrap().unwrap().into_data().unwrap(),
        "[]"
    );
    assert!(body.frame().await.is_none());
    quiesce(&fixture).await;
}
