use super::*;
use crate::{
    InitLimits, Metadata,
    file_download::tests::reference::{Reference, hex},
};
use axum::{body::Bytes, http::HeaderValue};
use futures_util::stream;
use http_body_util::StreamBody;
use sha2::{Digest, Sha512};
use std::{
    collections::VecDeque,
    future::{Future, poll_fn},
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll},
};

struct Meta;
impl MetadataSource for Meta {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        Ok(Metadata {
            instance_id: "vm".into(),
            template_id: "tpl".into(),
            access_token_sha512: Sha512::digest(b"token").into(),
        })
    }
}
async fn session() -> Session<Meta> {
    let session = Session::new(
        Meta,
        InitLimits {
            body_bytes: 1024,
            token_bytes: 64,
            state_bytes: 1024,
            env_entries: 16,
            in_flight: 1,
            timeout: Duration::from_mins(1),
        },
    )
    .unwrap();
    session
        .initialize(br#"{"accessToken":"token"}"#)
        .await
        .unwrap();
    session
}
fn limits() -> Limits {
    Limits {
        in_flight: 1,
        timeout: Duration::from_secs(30),
        headers: HeaderLimits {
            bytes: 256,
            fields: 8,
        },
        body: HttpBodyLimits {
            total_bytes: 64,
            frame_bytes: 32,
            frames: 8,
            trailer_bytes: 64,
            trailer_fields: 2,
        },
    }
}
fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-access-token", "token".parse().unwrap());
    headers
}
fn full(pool: &FileRpcRequests) {
    assert!(matches!(
        pool.admit(),
        Err(AdmissionError::ResourceExhausted)
    ));
}
async fn released(pool: &FileRpcRequests) {
    drop(
        tokio::time::timeout(Duration::from_secs(5), pool.slots.acquire())
            .await
            .unwrap()
            .unwrap(),
    );
}
async fn pending<T>(mut future: Pin<&mut impl Future<Output = T>>) {
    assert!(poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx).is_pending())).await);
}

#[test]
fn invalid_configuration_never_constructs_an_admission_pool() {
    let mut cases = vec![limits(); 13];
    cases[0].in_flight = 0;
    cases[1].in_flight = Semaphore::MAX_PERMITS + 1;
    cases[2].timeout = Duration::ZERO;
    cases[3].timeout = Duration::MAX;
    cases[4].headers.bytes = 0;
    cases[5].headers.fields = usize::MAX;
    cases[6].body.total_bytes = 0;
    cases[7].body.total_bytes = u64::MAX;
    cases[8].body.frame_bytes = 0;
    cases[9].body.frames = 0;
    cases[10].body.trailer_bytes = 0;
    cases[11].body.trailer_fields = 0;
    cases[12].body.frame_bytes = usize::MAX;
    for limits in cases {
        assert!(matches!(
            FileRpcRequests::new(limits),
            Err(AdmissionError::InvalidConfiguration)
        ));
    }
}

#[tokio::test]
async fn one_admission_survives_headers_body_decode_and_retained_result() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    let clone = pool.clone();
    let admitted = pool.admit().unwrap();
    let end = admitted.deadline();
    full(&clone);
    let auth = admitted.authenticate(&session, headers()).await.unwrap();
    assert_eq!(auth.deadline(), end);
    assert_eq!(auth.authorization().defaults().user, "root");
    full(&clone);
    let body = Body::from_stream(stream::iter([
        Ok::<_, std::io::Error>(Bytes::from_static(b"{\"path\":")),
        Ok(Bytes::from_static(b"\"/file\"}")),
    ]));
    let decoded = auth.decode("Stat", body).await.ok().unwrap();
    assert_eq!(decoded.request(), &Request::Stat("/file".into()));
    assert_eq!(decoded.headers()["x-access-token"], "token");
    assert_eq!(decoded.deadline(), end);
    assert_eq!(decoded.authorization().defaults().user, "root");
    full(&clone);
    let mut init =
        Box::pin(session.initialize(br#"{"accessToken":"token","defaultUser":"updated"}"#));
    pending(init.as_mut()).await;
    drop(decoded);
    init.await.unwrap();
    released(&pool).await;
}

#[tokio::test]
async fn header_and_auth_rejections_retain_capacity_without_reflecting_secrets() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    let mut large = headers();
    large.insert(
        "ignored",
        HeaderValue::from_bytes(&vec![b'x'; 256]).unwrap(),
    );
    let mut many = headers();
    for _ in 0..8 {
        many.append("ignored", "x".parse().unwrap());
    }
    let mut duplicate = headers();
    duplicate.append("x-access-token", "token".parse().unwrap());
    let mut wrong = headers();
    wrong.insert("x-access-token", "private-secret".parse().unwrap());
    for (headers, expected) in [
        (large, Error::HeaderLimit),
        (many, Error::HeaderLimit),
        (
            duplicate,
            Error::Authorization(ServiceError::Unauthenticated),
        ),
        (wrong, Error::Authorization(ServiceError::Unauthenticated)),
        (
            HeaderMap::new(),
            Error::Authorization(ServiceError::Unauthenticated),
        ),
    ] {
        let failure = pool
            .admit()
            .unwrap()
            .authenticate(&session, headers)
            .await
            .err()
            .unwrap();
        assert_eq!(failure.error(), &expected);
        assert!(!format!("{failure:?}").contains("private-secret"));
        full(&pool);
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        drop(failure);
        released(&pool).await;
    }
}

#[tokio::test]
async fn cancelled_and_expired_token_waits_release_the_reader_but_failure_keeps_admission() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    let held = session.authorize(Some("token")).await.unwrap();
    let mut init =
        Box::pin(session.initialize(br#"{"accessToken":"token","defaultUser":"updated"}"#));
    pending(init.as_mut()).await; // A queued writer prevents later readers.
    let mut waiting = Box::pin(pool.admit().unwrap().authenticate(&session, headers()));
    pending(waiting.as_mut()).await;
    full(&pool);
    drop(waiting);
    let admitted = pool.admit().unwrap();
    let end = admitted.deadline();
    let mut waiting = Box::pin(admitted.authenticate(&session, headers()));
    pending(waiting.as_mut()).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    let failure = waiting.await.err().unwrap();
    assert_eq!(failure.error(), &Error::DeadlineExceeded);
    assert_eq!(failure.deadline(), end);
    tokio::time::resume();
    full(&pool);
    drop(failure);
    drop(held);
    init.await.unwrap();
    let auth = pool
        .admit()
        .unwrap()
        .authenticate(&session, headers())
        .await
        .unwrap();
    assert_eq!(auth.authorization().defaults().user, "updated");
    drop(auth);
}

// A controlled framing layer, independent of the Content-Length header. It
// supplies real frames/size hints and records whether rejection read the body.
struct Frames {
    pending: VecDeque<Result<hyper::body::Frame<Bytes>, std::io::Error>>,
    expected: Option<u64>,
    polls: Arc<AtomicUsize>,
}
impl HttpBody for Frames {
    type Data = Bytes;
    type Error = std::io::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<hyper::body::Frame<Bytes>, Self::Error>>> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Ready(self.pending.pop_front())
    }
    fn size_hint(&self) -> hyper::body::SizeHint {
        let mut hint = hyper::body::SizeHint::new();
        if let Some(expected) = self.expected {
            hint.set_exact(expected);
        }
        hint
    }
}
fn body(frames: Vec<hyper::body::Frame<Bytes>>, expected: Option<u64>) -> (Body, Arc<AtomicUsize>) {
    let polls = Arc::new(AtomicUsize::new(0));
    (
        Body::new(Frames {
            pending: frames.into_iter().map(Ok).collect(),
            expected,
            polls: Arc::clone(&polls),
        }),
        polls,
    )
}
fn data(bytes: &[u8]) -> hyper::body::Frame<Bytes> {
    hyper::body::Frame::data(Bytes::copy_from_slice(bytes))
}

#[tokio::test]
async fn complete_framing_and_all_body_budgets_precede_json_success() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    let mut oversized = HeaderMap::new();
    oversized.insert("extra", HeaderValue::from_bytes(&[b'x'; 64]).unwrap());
    let mut many = HeaderMap::new();
    for _ in 0..3 {
        many.append("extra", "x".parse().unwrap());
    }
    let trailer = || hyper::body::Frame::trailers(HeaderMap::new());
    for (frames, expected, error) in [
        (vec![data(b"{}")], Some(65), Error::BodyLimit),
        (vec![data(b"{}")], Some(3), Error::Transport),
        (vec![data(b"{}")], Some(1), Error::Transport),
        (vec![data(&[b' '; 33])], None, Error::BodyLimit),
        (
            vec![data(&[b' '; 32]), data(&[b' '; 32]), data(b" ")],
            None,
            Error::BodyLimit,
        ),
        (
            vec![data(b"{}"), hyper::body::Frame::trailers(many)],
            None,
            Error::BodyLimit,
        ),
        ((0..9).map(|_| data(b"")).collect(), None, Error::BodyLimit),
        (
            vec![data(b"{}"), trailer(), data(b" ")],
            None,
            Error::Transport,
        ),
        (
            vec![data(b"{}"), trailer(), trailer()],
            None,
            Error::Transport,
        ),
        (
            vec![data(b"{}"), hyper::body::Frame::trailers(oversized)],
            None,
            Error::BodyLimit,
        ),
        (
            vec![data(b"{")],
            None,
            Error::Protocol(ProtocolError::InvalidArgument),
        ),
    ] {
        let (body, polls) = body(frames, expected);
        let auth = pool
            .admit()
            .unwrap()
            .authenticate(&session, headers())
            .await
            .unwrap();
        let failure = auth.decode("Stat", body).await.err().unwrap();
        assert_eq!(failure.error(), &error);
        if expected == Some(65) {
            assert_eq!(polls.load(Ordering::SeqCst), 0);
        }
        full(&pool);
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        drop(failure);
        released(&pool).await;
    }
    let (body, _) = body(vec![data(b"{}"), trailer()], Some(2));
    let mut headers = headers();
    headers.insert("content-length", "999999".parse().unwrap());
    let decoded = pool
        .admit()
        .unwrap()
        .authenticate(&session, headers)
        .await
        .unwrap()
        .decode("Remove", body)
        .await
        .ok()
        .unwrap();
    assert_eq!(decoded.request(), &Request::Remove(String::new()));
    drop(decoded);
    let failed_body = Body::new(StreamBody::new(stream::iter([Err::<
        hyper::body::Frame<Bytes>,
        _,
    >(
        std::io::Error::other("private source detail"),
    )])));
    let failure = pool
        .admit()
        .unwrap()
        .authenticate(&session, self::headers())
        .await
        .unwrap()
        .decode("Stat", failed_body)
        .await
        .err()
        .unwrap();
    assert_eq!(failure.error(), &Error::Transport);
    assert!(!format!("{failure:?}").contains("private source"));
}

#[tokio::test]
async fn connect_timeout_preserves_original_cap_and_rejects_before_polling_expired_or_unknown_work()
{
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    for (value, method, error) in [
        ("0", "Stat", Error::DeadlineExceeded),
        ("-1", "Stat", Error::DeadlineExceeded),
        ("+0", "Stat", Error::DeadlineExceeded),
        (
            "",
            "WatchDir",
            Error::Protocol(ProtocolError::Unimplemented),
        ),
    ] {
        let mut h = headers();
        h.insert("connect-timeout-ms", value.parse().unwrap());
        let (body, polls) = body(vec![data(b"{}")], Some(2));
        let failure = pool
            .admit()
            .unwrap()
            .authenticate(&session, h)
            .await
            .unwrap()
            .decode(method, body)
            .await
            .err()
            .unwrap();
        assert_eq!(failure.error(), &error);
        assert_eq!(polls.load(Ordering::SeqCst), 0);
        full(&pool);
        drop(failure);
    }
    for first in ["", "9999999999"] {
        let mut h = headers();
        h.append("connect-timeout-ms", first.parse().unwrap());
        h.append("connect-timeout-ms", "invalid second".parse().unwrap());
        let admitted = pool.admit().unwrap();
        let end = admitted.deadline();
        let auth = admitted.authenticate(&session, h).await.unwrap();
        let decoded = auth.decode("Stat", Body::from("{}")).await.ok().unwrap();
        assert_eq!(decoded.deadline(), end);
        drop(decoded);
    }
    let mut h = headers();
    h.insert("connect-timeout-ms", "+1000".parse().unwrap());
    let auth = pool
        .admit()
        .unwrap()
        .authenticate(&session, h)
        .await
        .unwrap();
    let original = auth.deadline();
    let decoded = auth.decode("Stat", Body::from("{}")).await.ok().unwrap();
    assert!(decoded.deadline() < original);
}

#[tokio::test]
async fn cancelled_or_expired_body_wait_drops_source_and_auth_without_fabricating_a_request() {
    struct Dropped(Arc<AtomicUsize>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    for expire in [false, true] {
        let polls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&polls);
        let drops = Arc::new(AtomicUsize::new(0));
        let guard = Dropped(Arc::clone(&drops));
        let mut sent = false;
        let source = stream::poll_fn(move |_| {
            let _retained = &guard;
            observed.fetch_add(1, Ordering::SeqCst);
            if !sent {
                sent = true;
                return Poll::Ready(Some(Ok(Bytes::from_static(b"{}"))));
            }
            // Valid complete JSON is insufficient until the HTTP body ends.
            Poll::Pending::<Option<Result<Bytes, std::io::Error>>>
        });
        let auth = pool
            .admit()
            .unwrap()
            .authenticate(&session, headers())
            .await
            .unwrap();
        let end = auth.deadline();
        let mut waiting = Box::pin(auth.decode("Stat", Body::from_stream(source)));
        pending(waiting.as_mut()).await;
        assert!(polls.load(Ordering::SeqCst) >= 2);
        full(&pool);
        if expire {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(31)).await;
            let failure = waiting.await.err().unwrap();
            assert_eq!(failure.error(), &Error::DeadlineExceeded);
            assert_eq!(failure.deadline(), end);
            tokio::time::resume();
            full(&pool);
            drop(failure);
        } else {
            drop(waiting);
        }
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        released(&pool).await;
    }
}

#[test]
fn queued_decoder_keeps_authorization_and_capacity_after_wait_deadline_until_actual_exit() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let session = session().await;
        let pool = FileRpcRequests::new(limits()).unwrap();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        let occupied = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            resumed.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        started.await.unwrap();
        let auth = pool
            .admit()
            .unwrap()
            .authenticate(&session, headers())
            .await
            .unwrap();
        let mut decoding = Box::pin(auth.decode("Stat", Body::from("{}")));
        pending(decoding.as_mut()).await; // Input complete; decoder is queued.
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(31)).await;
        let failure = decoding.await.err().unwrap();
        assert_eq!(failure.error(), &Error::DeadlineExceeded);
        tokio::time::resume();
        drop(failure);
        full(&pool);
        let mut init = Box::pin(session.initialize(br#"{"accessToken":"token"}"#));
        pending(init.as_mut()).await;
        resume.send(()).unwrap();
        occupied.await.unwrap();
        init.await.unwrap();
        released(&pool).await;
    });
}

#[test]
fn fixed_timeout_signs_first_field_and_absolute_deadline_boundaries() {
    let now = Instant::now();
    let original = now + Duration::from_secs(5);
    for (value, expected) in [
        ("", original),
        ("5001", original),
        ("5000", original),
        ("00001", now + Duration::from_millis(1)),
        ("-0", now),
        ("-999999999", now),
    ] {
        let mut h = HeaderMap::new();
        h.append("connect-timeout-ms", value.parse().unwrap());
        h.append("connect-timeout-ms", "0".parse().unwrap());
        assert_eq!(timeout::deadline(&h, now, original).unwrap(), expected);
    }
    for (value, reason) in [
        (b"12345678901".as_slice(), TimeoutReason::TooLong),
        (b"+", TimeoutReason::Invalid),
        (b" 1", TimeoutReason::Invalid),
        (b"1_0", TimeoutReason::Invalid),
        (b"\xff", TimeoutReason::Invalid),
    ] {
        let mut h = HeaderMap::new();
        h.insert(
            "connect-timeout-ms",
            HeaderValue::from_bytes(value).unwrap(),
        );
        let error = timeout::deadline(&h, now, original).unwrap_err();
        assert_eq!(error.reason(), reason);
        assert_eq!(error.value(), value);
    }
}

#[test]
#[ignore = "explicit fixed Go timeout parsing reference; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
fn fixed_go_connect_timeout_parser_reference() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut cases: Vec<Vec<u8>> = [
        "",
        "0",
        "-0",
        "+0",
        "1",
        "+1",
        "-1",
        "9999999999",
        "+999999999",
        "-999999999",
        "0000000000",
        "10000000000",
        " 1",
        "1 ",
        "1_0",
        "1.0",
        "+",
        "-",
        "--1",
        "+-1",
        "１２",
    ]
    .into_iter()
    .map(|s| s.as_bytes().to_vec())
    .collect();
    for byte in 0..=255 {
        cases.push(vec![byte]);
        cases.push(vec![b'1', byte, b'0']);
    }
    for value in &cases {
        let observed = match timeout::parse(value) {
            Ok(None) => "NONE".into(),
            Ok(Some(value)) => format!("OK\t{value}"),
            Err(TimeoutReason::TooLong) => "ERR\tlength".into(),
            Err(TimeoutReason::Invalid) => "ERR\tinvalid".into(),
        };
        oracle.check(&format!("T\t{}", hex(value)), &observed);
    }
    oracle.finish();
    eprintln!(
        "{} fixed Go Connect timeout parsing cases; no wall-clock sleep.",
        cases.len()
    );
}
