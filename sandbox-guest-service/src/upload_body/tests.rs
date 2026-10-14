use super::*;
use futures_util::{FutureExt, Stream, stream};
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use sandproof_guest_runtime::gzip::{EncodeLimits, Encoder};
use std::sync::Arc;
use std::{
    collections::VecDeque,
    pin::Pin,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

fn limits() -> GzipLimits {
    GzipLimits {
        http: HttpBodyLimits {
            total_bytes: 8192,
            frame_bytes: 4096,
            frames: 8192,
            trailer_bytes: 256,
            trailer_fields: 8,
        },
        decode: DecodeLimits {
            chunk_bytes: 3,
            encoded_bytes: 8192,
            decoded_bytes: 8192,
            header_bytes: 1024,
            members: 16,
        },
    }
}
fn encode(input: &[u8]) -> Vec<u8> {
    let mut codec = Encoder::new(
        input,
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
fn body(bytes: &[u8], chunk: usize) -> Body {
    let chunks: Vec<_> = bytes
        .chunks(chunk)
        .map(|bytes| Ok::<_, GzipError>(Bytes::copy_from_slice(bytes)))
        .collect();
    Body::from_stream(stream::iter(chunks))
}
pub(super) fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
async fn drain(mut input: GzipBody) -> (Vec<u8>, Option<GzipError>) {
    let mut output = Vec::new();
    loop {
        match input.next().await {
            Ok(Some(bytes)) => {
                assert!(bytes.as_ref().len() <= 3);
                output.extend(bytes.as_ref());
            }
            Ok(None) => return (output, None),
            Err(error) => return (output, Some(error)),
        }
    }
}
pub(super) async fn signal<T>(receiver: oneshot::Receiver<T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), receiver)
        .await
        .expect("barrier timed out")
        .unwrap()
}
struct OwnerDrop(Option<oneshot::Sender<()>>);
impl Drop for OwnerDrop {
    fn drop(&mut self) {
        if let Some(send) = self.0.take() {
            let _ = send.send(());
        }
    }
}
pub(super) fn owner() -> (Owner, oneshot::Receiver<()>) {
    let (send, receive) = oneshot::channel();
    (Arc::new(OwnerDrop(Some(send))), receive)
}

struct Controlled {
    frames: VecDeque<Result<Bytes, GzipError>>,
    next: mpsc::Receiver<Result<Bytes, GzipError>>,
    waiting: Option<oneshot::Sender<()>>,
    polls: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
}
impl Stream for Controlled {
    type Item = Result<Bytes, GzipError>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.polls.fetch_add(1, Ordering::SeqCst);
        if let Some(bytes) = this.frames.pop_front() {
            return Poll::Ready(Some(bytes));
        }
        let result = this.next.poll_recv(cx);
        if result.is_pending()
            && let Some(waiting) = this.waiting.take()
        {
            let _ = waiting.send(());
        }
        result
    }
}
impl Drop for Controlled {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}
type ControlledInput = (
    Body,
    mpsc::Sender<Result<Bytes, GzipError>>,
    oneshot::Receiver<()>,
    Arc<AtomicUsize>,
    Arc<AtomicBool>,
);
fn controlled(frames: Vec<Vec<u8>>) -> ControlledInput {
    let (send, receive) = mpsc::channel(1);
    let (waiting, ready) = oneshot::channel();
    let polls = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let source = Controlled {
        frames: frames.into_iter().map(|b| Ok(Bytes::from(b))).collect(),
        next: receive,
        waiting: Some(waiting),
        polls: Arc::clone(&polls),
        dropped: Arc::clone(&dropped),
    };
    (Body::from_stream(source), send, ready, polls, dropped)
}

#[tokio::test]
async fn preflight_reads_only_the_header_then_decodes_members_on_demand() {
    let bytes = [encode(b"first"), encode(b""), encode(b"\0\xfflast")].concat();
    let (source, send, _waiting, polls, _dropped) =
        controlled(vec![bytes[..10].to_vec(), bytes[10..].to_vec()]);
    drop(send);
    let input = GzipBody::open(
        source,
        Some(bytes.len() as u64),
        limits(),
        deadline(),
        Arc::new(()),
    )
    .await
    .unwrap();
    assert_eq!(
        polls.load(Ordering::SeqCst),
        1,
        "preflight must not decode body data"
    );
    let (output, error) = drain(input).await;
    assert_eq!(error, None);
    assert_eq!(output, b"first\0\xfflast");
    for input in [
        Vec::new(),
        b"invalid gzip header".to_vec(),
        bytes[..9].to_vec(),
    ] {
        let (guard, released) = owner();
        assert!(
            GzipBody::open(body(&input, 1), None, limits(), deadline(), guard)
                .await
                .is_err()
        );
        signal(released).await;
    }
}

#[tokio::test]
async fn cloned_bytes_hold_backpressure_and_original_guard_after_body_cancellation() {
    let bytes = encode(b"more than one output chunk");
    let (guard, mut released) = owner();
    let mut input = GzipBody::open(body(&bytes, 1), None, limits(), deadline(), guard)
        .await
        .unwrap();
    let first = Bytes::from_owner(input.next().await.unwrap().unwrap());
    let clone = first.clone();
    let mut next = Box::pin(input.next());
    assert!(next.as_mut().now_or_never().is_none());
    drop(first);
    assert!(
        next.as_mut().now_or_never().is_none(),
        "last Bytes owner controls release"
    );
    drop(next); // Cancel the pending request without dropping the old bytes.
    drop(input);
    assert!(matches!(
        released.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    drop(clone);
    signal(released).await;

    // Successful resumption also requires release, not a timer or another poll.
    let mut input = GzipBody::open(body(&bytes, 1), None, limits(), deadline(), Arc::new(()))
        .await
        .unwrap();
    let first = input.next().await.unwrap().unwrap();
    let mut next = Box::pin(input.next());
    assert!(next.as_mut().now_or_never().is_none());
    drop(first);
    assert!(next.await.unwrap().is_some());
}

#[tokio::test]
async fn real_http_eof_and_transport_failure_remain_distinct_after_valid_trailer() {
    for failure in [false, true] {
        let bytes = encode(b"already decoded");
        let (source, send, waiting, _, dropped) = controlled(vec![bytes.clone()]);
        let (guard, released) = owner();
        let input = GzipBody::open(
            source,
            Some(bytes.len() as u64),
            limits(),
            deadline(),
            guard,
        )
        .await
        .unwrap();
        let task = tokio::spawn(drain(input));
        signal(waiting).await;
        assert!(!task.is_finished());
        if failure {
            send.send(Err(GzipError::Transport)).await.unwrap();
        }
        drop(send);
        let (output, error) = task.await.unwrap();
        assert_eq!(output, b"already decoded");
        assert_eq!(error, failure.then_some(GzipError::Transport));
        signal(released).await;
        assert!(dropped.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn cancellation_wakes_header_and_body_reads_and_poisoned_handle_cannot_resume() {
    let bytes = encode(b"data");
    let (source, _send, waiting, _, dropped) = controlled(vec![bytes[..3].to_vec()]);
    let (guard, released) = owner();
    let mut opening = Box::pin(GzipBody::open(source, None, limits(), deadline(), guard));
    assert!(opening.as_mut().now_or_never().is_none());
    signal(waiting).await;
    drop(opening);
    signal(released).await;
    assert!(dropped.load(Ordering::SeqCst));

    let (source, _send, waiting, _, dropped) = controlled(vec![bytes[..10].to_vec()]);
    let (guard, released) = owner();
    let mut input = GzipBody::open(source, None, limits(), deadline(), guard)
        .await
        .unwrap();
    let mut pending = Box::pin(input.next());
    assert!(pending.as_mut().now_or_never().is_none());
    signal(waiting).await;
    drop(pending);
    signal(released).await;
    assert!(dropped.load(Ordering::SeqCst));
    assert!(matches!(input.next().await, Err(GzipError::Unavailable)));
}

#[tokio::test]
async fn original_deadline_applies_to_header_and_body_without_restarting() {
    for count in [3, 10] {
        let bytes = encode(b"data");
        let (source, _send, waiting, _, dropped) = controlled(vec![bytes[..count].to_vec()]);
        let (guard, released) = owner();
        let end = deadline();
        let task = tokio::spawn(async move {
            match GzipBody::open(source, None, limits(), end, guard).await {
                Ok(mut input) => input.next().await.map(|_| ()),
                Err(error) => Err(error),
            }
        });
        signal(waiting).await;
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(31)).await;
        assert_eq!(task.await.unwrap(), Err(GzipError::DeadlineExceeded));
        tokio::time::resume();
        signal(released).await;
        assert!(dropped.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn cancellation_does_not_release_guard_during_a_synchronous_source_poll() {
    struct Blocking {
        entered: Option<oneshot::Sender<()>>,
        release: std::sync::mpsc::Receiver<()>,
        header: Vec<u8>,
        dropped: Arc<AtomicBool>,
    }
    impl Stream for Blocking {
        type Item = Result<Bytes, GzipError>;
        fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            if let Some(entered) = self.entered.take() {
                let _ = entered.send(());
                self.release.recv().unwrap();
                return Poll::Ready(Some(Ok(Bytes::from(std::mem::take(&mut self.header)))));
            }
            Poll::Ready(None)
        }
    }
    impl Drop for Blocking {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }
    let (entered, in_poll) = oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let dropped = Arc::new(AtomicBool::new(false));
    let body = Body::from_stream(Blocking {
        entered: Some(entered),
        release: wait,
        header: encode(b"data")[..10].to_vec(),
        dropped: Arc::clone(&dropped),
    });
    let (guard, mut released) = owner();
    let mut opening = Box::pin(GzipBody::open(body, None, limits(), deadline(), guard));
    assert!(opening.as_mut().now_or_never().is_none());
    signal(in_poll).await;
    drop(opening);
    assert!(matches!(
        released.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(!dropped.load(Ordering::SeqCst));
    release.send(()).unwrap();
    signal(released).await;
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn encoded_declared_actual_and_decoded_limits_are_independent() {
    let bytes = encode(b"content");
    let mut budget = limits();
    budget.http.total_bytes = bytes.len() as u64 - 1;
    assert!(matches!(
        GzipBody::open(
            body(&bytes, 1),
            Some(bytes.len() as u64),
            budget,
            deadline(),
            Arc::new(())
        )
        .await,
        Err(GzipError::InputLimit)
    ));
    let input = GzipBody::open(body(&bytes, 1), None, budget, deadline(), Arc::new(()))
        .await
        .unwrap();
    assert_eq!(drain(input).await.1, Some(GzipError::InputLimit));
    for expected in [bytes.len() as u64 - 1, bytes.len() as u64 + 1] {
        let input = GzipBody::open(
            body(&bytes, 1),
            Some(expected),
            limits(),
            deadline(),
            Arc::new(()),
        )
        .await
        .unwrap();
        assert_eq!(drain(input).await.1, Some(GzipError::Transport));
    }
    let mut budget = limits();
    budget.decode.decoded_bytes = 5;
    let input = GzipBody::open(
        body(&bytes, 1),
        Some(bytes.len() as u64),
        budget,
        deadline(),
        Arc::new(()),
    )
    .await
    .unwrap();
    assert_eq!(
        drain(input).await,
        (b"conte".to_vec(), Some(GzipError::DecodedLimit))
    );
    let mut bad = bytes.clone();
    let last = bad.len() - 8;
    bad[last] ^= 1;
    let mut body = GzipBody::open(body(&bad, 1), None, limits(), deadline(), Arc::new(()))
        .await
        .unwrap()
        .into_body();
    let mut output = Vec::new();
    loop {
        match body.frame().await {
            Some(Ok(frame)) => output.extend(frame.into_data().unwrap()),
            Some(Err(error)) => {
                assert_eq!(http::body_error(error), GzipError::Checksum);
                break;
            }
            None => panic!("checksum failure cannot be HTTP EOF"),
        }
    }
    assert_eq!(output, b"content");
}

#[tokio::test]
async fn shared_http_input_bounds_empty_frames_trailers_and_order() {
    let bytes = encode(b"content");
    let mut trailers = axum::http::HeaderMap::new();
    trailers.insert("x-test", "value".parse().unwrap());
    let cases = [
        (
            vec![
                Frame::data(Bytes::from(bytes.clone())),
                Frame::trailers(trailers.clone()),
            ],
            None,
        ),
        (
            vec![
                Frame::data(Bytes::from(bytes.clone())),
                Frame::trailers(trailers.clone()),
                Frame::data(Bytes::new()),
            ],
            Some(GzipError::Transport),
        ),
        (
            vec![
                Frame::data(Bytes::from(bytes.clone())),
                Frame::trailers(trailers.clone()),
                Frame::trailers(trailers.clone()),
            ],
            Some(GzipError::Transport),
        ),
    ];
    for (frames, expected) in cases {
        let body = Body::new(StreamBody::new(stream::iter(
            frames.into_iter().map(Ok::<_, GzipError>),
        )));
        let input = GzipBody::open(body, None, limits(), deadline(), Arc::new(()))
            .await
            .unwrap();
        assert_eq!(drain(input).await.1, expected);
    }
    for (frames, budget) in [
        (
            vec![Frame::data(Bytes::from(bytes.clone()))],
            HttpBodyLimits {
                frame_bytes: 1,
                ..limits().http
            },
        ),
        (
            vec![Frame::data(Bytes::new()), Frame::data(Bytes::new())],
            HttpBodyLimits {
                frames: 1,
                ..limits().http
            },
        ),
        (
            vec![Frame::data(Bytes::from(bytes)), Frame::trailers(trailers)],
            HttpBodyLimits {
                trailer_bytes: 1,
                ..limits().http
            },
        ),
    ] {
        let body = Body::new(StreamBody::new(stream::iter(
            frames.into_iter().map(Ok::<_, GzipError>),
        )));
        match GzipBody::open(
            body,
            None,
            GzipLimits {
                http: budget,
                ..limits()
            },
            deadline(),
            Arc::new(()),
        )
        .await
        {
            Ok(input) => assert_eq!(drain(input).await.1, Some(GzipError::InputLimit)),
            Err(error) => assert_eq!(error, GzipError::InputLimit),
        }
    }
}
