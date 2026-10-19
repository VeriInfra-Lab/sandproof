use super::*;
use crate::upload_body::{
    http,
    tests::{deadline, owner, signal},
};
use axum::body::Bytes;
use futures_util::FutureExt;
use http_body_util::BodyExt;
use sandproof_guest_runtime::gzip::{DecodeLimits, Decoder};
use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::oneshot;

fn limits() -> EncodeLimits {
    EncodeLimits {
        chunk_bytes: 7,
        decoded_bytes: 1 << 20,
        encoded_bytes: 2 << 20,
    }
}
async fn drain(mut body: GzipBody) -> (Vec<u8>, Option<GzipError>) {
    let mut bytes = Vec::new();
    loop {
        match body.next().await {
            Ok(Some(chunk)) => {
                assert!(chunk.as_ref().len() <= 7);
                bytes.extend(chunk.as_ref());
            }
            Ok(None) => {
                assert!(body.next().await.unwrap().is_none());
                return (bytes, None);
            }
            Err(error) => {
                assert!(matches!(body.next().await, Err(GzipError::Unavailable)));
                return (bytes, Some(error));
            }
        }
    }
}
fn decode(bytes: &[u8]) -> Vec<u8> {
    let mut codec = Decoder::new(
        bytes,
        DecodeLimits {
            chunk_bytes: 4096,
            encoded_bytes: 2 << 20,
            decoded_bytes: 1 << 20,
            header_bytes: 1024,
            members: 1,
        },
    )
    .unwrap();
    let mut result = Vec::new();
    while let Some(chunk) = codec.next_chunk().unwrap() {
        result.extend(chunk);
    }
    result
}

struct Counted {
    data: Cursor<Vec<u8>>,
    polls: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
}
impl Read for Counted {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        assert!(
            output.len() <= 7,
            "source buffer must use the configured chunk bound"
        );
        self.polls.fetch_add(1, Ordering::SeqCst);
        self.data.read(output)
    }
}
impl Drop for Counted {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn encoding_opens_without_reading_and_roundtrips_empty_binary_and_large_sources() {
    for data in [
        vec![],
        b"\0\xffbinary".to_vec(),
        (0..131_071)
            .map(|n| u8::try_from((n * 31) % 256).unwrap())
            .collect(),
    ] {
        let expected = data.clone();
        let polls = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicBool::new(false));
        let source = Counted {
            data: Cursor::new(data),
            polls: Arc::clone(&polls),
            dropped: Arc::clone(&dropped),
        };
        let (guard, released) = owner();
        let body = GzipBody::encode(move || Ok(source), limits(), deadline(), guard)
            .await
            .unwrap();
        assert_eq!(polls.load(Ordering::SeqCst), 0);
        let (bytes, error) = drain(body).await;
        assert_eq!(error, None);
        assert_eq!(decode(&bytes), expected);
        signal(released).await;
        assert!(dropped.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn compressed_bytes_clones_hold_backpressure_authorization_and_capacity() {
    let polls = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let source = Counted {
        data: Cursor::new(vec![42; 8192]),
        polls: Arc::clone(&polls),
        dropped: Arc::clone(&dropped),
    };
    let (guard, mut released) = owner();
    let mut body = GzipBody::encode(move || Ok(source), limits(), deadline(), guard)
        .await
        .unwrap();
    let bytes = Bytes::from_owner(body.next().await.unwrap().unwrap());
    let clone = bytes.clone();
    let count = polls.load(Ordering::SeqCst);
    let mut next = Box::pin(body.next());
    assert!(next.as_mut().now_or_never().is_none());
    drop(bytes);
    assert!(next.as_mut().now_or_never().is_none());
    assert_eq!(polls.load(Ordering::SeqCst), count);
    drop(next);
    assert!(matches!(body.next().await, Err(GzipError::Unavailable)));
    drop(body);
    assert!(matches!(
        released.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    drop(clone);
    signal(released).await;
    assert!(dropped.load(Ordering::SeqCst));
}

struct Blocking {
    entered: Option<oneshot::Sender<()>>,
    release: std::sync::mpsc::Receiver<()>,
    dropped: Arc<AtomicBool>,
}
impl Read for Blocking {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
            self.release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        Ok(0)
    }
}
impl Drop for Blocking {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn cancellation_and_deadline_keep_guard_until_blocked_source_read_actually_returns() {
    for expire in [false, true] {
        let (entered, waiting) = oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let dropped = Arc::new(AtomicBool::new(false));
        let source = Blocking {
            entered: Some(entered),
            release: wait,
            dropped: Arc::clone(&dropped),
        };
        let (guard, mut released) = owner();
        let body = GzipBody::encode(move || Ok(source), limits(), deadline(), guard)
            .await
            .unwrap();
        let task = tokio::spawn(drain(body));
        signal(waiting).await;
        if expire {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(31)).await;
            assert_eq!(task.await.unwrap().1, Some(GzipError::DeadlineExceeded));
            tokio::time::resume();
        } else {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        }
        assert!(!dropped.load(Ordering::SeqCst));
        assert!(matches!(
            released.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        release.send(()).unwrap();
        signal(released).await;
        assert!(dropped.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn cancelled_or_expired_open_wait_does_not_release_the_running_opener() {
    for expire in [false, true] {
        let (entered, waiting) = oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let (guard, mut released) = owner();
        let polls = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicBool::new(false));
        let source = Counted {
            data: Cursor::new(b"source".to_vec()),
            polls: Arc::clone(&polls),
            dropped: Arc::clone(&dropped),
        };
        let mut opening = Box::pin(GzipBody::encode(
            move || {
                let _ = entered.send(());
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(source)
            },
            limits(),
            deadline(),
            guard,
        ));
        assert!(opening.as_mut().now_or_never().is_none());
        signal(waiting).await;
        if expire {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(31)).await;
            assert!(matches!(
                opening.as_mut().await,
                Err(GzipError::DeadlineExceeded)
            ));
            tokio::time::resume();
        }
        drop(opening);
        assert!(matches!(
            released.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(!dropped.load(Ordering::SeqCst));
        release.send(()).unwrap();
        signal(released).await;
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(polls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn exact_raw_and_compressed_limits_still_require_actual_eof() {
    let data = b"exact limit";
    let run = async |budget| {
        drain(
            GzipBody::encode(|| Ok(data.as_slice()), budget, deadline(), Arc::new(()))
                .await
                .unwrap(),
        )
        .await
    };
    let (bytes, error) = run(limits()).await;
    assert_eq!(error, None);
    let exact = EncodeLimits {
        decoded_bytes: data.len() as u64,
        encoded_bytes: bytes.len() as u64,
        ..limits()
    };
    assert_eq!(run(exact).await, (bytes, None));
    assert_eq!(
        run(EncodeLimits {
            decoded_bytes: exact.decoded_bytes - 1,
            ..exact
        })
        .await
        .1,
        Some(GzipError::DecodedLimit)
    );
    assert_eq!(
        run(EncodeLimits {
            encoded_bytes: exact.encoded_bytes - 1,
            ..exact
        })
        .await
        .1,
        Some(GzipError::InputLimit)
    );
    assert!(matches!(
        GzipBody::encode(
            || -> Result<&[u8], _> { panic!("invalid configuration must not open") },
            EncodeLimits {
                chunk_bytes: 0,
                ..limits()
            },
            deadline(),
            Arc::new(())
        )
        .await,
        Err(GzipError::InvalidConfiguration)
    ));
}

#[tokio::test]
async fn read_error_is_not_a_valid_gzip_trailer_or_http_eof() {
    struct Failure;
    impl Read for Failure {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("controlled read failure"))
        }
    }
    let body = GzipBody::encode(
        || Ok(b"prefix".as_slice().chain(Failure)),
        limits(),
        deadline(),
        Arc::new(()),
    )
    .await
    .unwrap();
    let mut body = body.into_body();
    loop {
        match body.frame().await {
            Some(Ok(_)) => {}
            Some(Err(error)) => {
                assert_eq!(http::body_error(error), GzipError::Transport);
                break;
            }
            None => panic!("source error became HTTP success"),
        }
    }
}
