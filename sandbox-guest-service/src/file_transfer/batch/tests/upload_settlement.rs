//! A deterministic source destructor, not scheduling sleeps or larger quotas.
use super::{
    Fixture,
    multipart::field,
    upload::request,
    upload_errors::{response_limits, upload},
};
use crate::{file_transfer::UploadHttpError, upload_response::Error};
use axum::body::{Body, Bytes};
use futures_util::Stream;
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::oneshot;

struct BlockingDrop {
    bytes: Option<Bytes>,
    entered: Option<oneshot::Sender<()>>,
    release: std::sync::mpsc::Receiver<()>,
}
impl Stream for BlockingDrop {
    type Item = Result<Bytes, std::io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.bytes.take().map(Ok))
    }
}
impl Drop for BlockingDrop {
    fn drop(&mut self) {
        let _ = self.entered.take().unwrap().send(());
        let _ = self.release.recv();
    }
}

#[tokio::test]
async fn failed_upload_waits_for_actual_decoder_cleanup_under_the_original_deadline() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::new(root.path()).await);
    let input = [
        field("file", Some("partial"), b"prefix\0tail\n", true),
        b"--B--\r\n".to_vec(),
    ]
    .concat();
    let mut request = request(
        &super::gzip::encode(&input),
        true,
        super::multipart::CONTENT_TYPE,
        None,
    );
    let (entered, started) = oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    request.body = Body::from_stream(BlockingDrop {
        bytes: Some(Bytes::from(super::gzip::encode(&input))),
        entered: Some(entered),
        release: blocked,
    });
    let work = Arc::clone(&fixture);
    let task = tokio::spawn(async move { upload(&work, request, response_limits()).await });
    started.await.unwrap();
    assert_eq!(fixture.streams.slots.available_permits(), 0);
    // The decoder's real source destructor still owns both auth and capacity.
    let mut reinit = Box::pin(
        fixture
            .session
            .initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#),
    );
    assert!(std::future::poll_fn(|cx| Poll::Ready(reinit.as_mut().poll(cx).is_pending())).await);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    assert!(matches!(
        task.await.unwrap(),
        Err(UploadHttpError::Response(Error::DeadlineExceeded))
    ));
    assert_eq!(fixture.streams.slots.available_permits(), 0);
    assert!(std::future::poll_fn(|cx| Poll::Ready(reinit.as_mut().poll(cx).is_pending())).await);
    release.send(()).unwrap();
    tokio::time::resume();
    tokio::time::timeout(Duration::from_secs(5), reinit)
        .await
        .unwrap()
        .unwrap();
    super::multipart::quiesce(&fixture).await;
    assert_eq!(
        std::fs::read(root.path().join("partial")).unwrap(),
        b"prefix"
    );
}
