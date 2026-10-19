use super::*;
use crate::{InitLimits, Metadata};
use axum::body::Bytes;
use futures_util::stream;
use http_body_util::StreamBody;
use hyper::body::Frame;
use sha2::{Digest, Sha512};
use std::{
    future::Future,
    pin::Pin,
    sync::{Condvar, Mutex},
    task::Poll,
};

#[cfg(target_os = "linux")]
mod preflight;
mod token;

struct Meta;
impl MetadataSource for Meta {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        Ok(Metadata {
            instance_id: "vm".into(),
            template_id: "template".into(),
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
            total_bytes: 128,
            frame_bytes: 128,
            frames: 8,
            trailer_bytes: 64,
            trailer_fields: 2,
        },
    }
}
fn headers() -> HeaderMap {
    HeaderMap::from_iter([("x-access-token".parse().unwrap(), "token".parse().unwrap())])
}
async fn authenticate(pool: &CommandRequests, session: &Session<Meta>) -> Authenticated {
    pool.admit()
        .unwrap()
        .authenticate(session, headers())
        .await
        .unwrap()
}
fn full(pool: &CommandRequests) {
    assert!(matches!(
        pool.admit(),
        Err(AdmissionError::ResourceExhausted)
    ));
}
async fn released(pool: &CommandRequests) {
    drop(
        tokio::time::timeout(Duration::from_secs(5), pool.slots.acquire())
            .await
            .unwrap()
            .unwrap(),
    );
}
async fn pending<T>(mut future: Pin<&mut impl Future<Output = T>>) {
    assert!(std::future::poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx).is_pending())).await);
}
fn framed(bytes: &[u8]) -> Body {
    let mut frame = vec![0];
    frame.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_be_bytes());
    frame.extend_from_slice(bytes);
    Body::from(frame)
}

#[test]
fn invalid_budgets_reject_before_admission() {
    let mut cases = [limits(); 10];
    cases[0].in_flight = 0;
    cases[1].in_flight = Semaphore::MAX_PERMITS + 1;
    cases[2].timeout = Duration::ZERO;
    cases[3].timeout = Duration::MAX;
    cases[4].headers.bytes = 0;
    cases[5].body.total_bytes = 0;
    cases[6].body.total_bytes = u64::MAX;
    cases[7].body.frame_bytes = 0;
    cases[8].body.frames = 0;
    cases[9].body.trailer_fields = 0;
    for limits in cases {
        assert!(matches!(
            CommandRequests::new(limits),
            Err(AdmissionError::InvalidConfiguration)
        ));
    }
}

#[test]
fn cancelled_or_expired_command_gzip_keeps_auth_until_queued_worker_exits() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        for expire in [false, true] {
            let session = session().await;
            let pool = CommandRequests::new(limits()).unwrap();
            let (entered, started) = tokio::sync::oneshot::channel();
            let (resume, resumed) = std::sync::mpsc::channel();
            let occupied = tokio::task::spawn_blocking(move || {
                entered.send(()).unwrap();
                resumed.recv_timeout(Duration::from_secs(5)).unwrap();
            });
            started.await.unwrap();
            let auth = authenticate(&pool, &session).await;
            let deadline = auth.deadline();
            let wire = crate::file_rpc_requests::tests::gzip::compressed(b"{}");
            let mut decoding = Box::pin(auth.decode_with(
                Body::from(wire),
                deadline,
                |bytes, limit, deadline, cancelled| {
                    compression::decode("List", bytes, limit, 7, deadline, cancelled)
                },
            ));
            pending(decoding.as_mut()).await;
            if expire {
                tokio::time::pause();
                tokio::time::advance(Duration::from_secs(31)).await;
                let failure = decoding.await.err().unwrap();
                assert_eq!(failure.error(), Error::DeadlineExceeded);
                drop(failure);
                tokio::time::resume();
            } else {
                drop(decoding);
            }
            full(&pool);
            let mut init = Box::pin(session.initialize(br#"{"accessToken":"token"}"#));
            pending(init.as_mut()).await;
            resume.send(()).unwrap();
            occupied.await.unwrap();
            init.await.unwrap();
            released(&pool).await;
        }
    });
}

#[tokio::test]
async fn each_request_reauthenticates_and_failures_keep_capacity() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    for value in [None, Some("wrong"), Some("token")] {
        let admitted = pool.admit().unwrap();
        let mut headers = HeaderMap::new();
        if let Some(value) = value {
            headers.insert("x-access-token", value.parse().unwrap());
        }
        let result = admitted.authenticate(&session, headers).await;
        full(&pool);
        if value == Some("token") {
            assert!(result.is_ok());
        } else {
            assert_eq!(
                result.as_ref().err().unwrap().error(),
                Error::Authorization(ServiceError::Unauthenticated)
            );
        }
        drop(result);
    }
    let mut oversized = headers();
    oversized.insert("large", "x".repeat(256).parse().unwrap());
    let failure = pool
        .admit()
        .unwrap()
        .authenticate(&session, oversized)
        .await
        .err()
        .unwrap();
    assert_eq!(failure.error(), Error::HeaderLimit);
    full(&pool);
    drop(failure);
    assert!(pool.admit().is_ok());
}

#[tokio::test]
async fn all_selected_methods_decode_without_executing_and_keep_the_original_slot() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    for method in [
        "Start",
        "Connect",
        "List",
        "SendInput",
        "CloseStdin",
        "SendSignal",
        "Update",
    ] {
        let admitted = authenticate(&pool, &session).await;
        let deadline = admitted.deadline();
        let body = if matches!(method, "Start" | "Connect") {
            framed(b"{}")
        } else {
            Body::from("{}")
        };
        let decoded = admitted
            .decode(method, body, deadline + Duration::from_mins(1))
            .await
            .unwrap();
        assert_eq!(decoded.deadline, deadline);
        assert_eq!(
            decoded.authenticated.authorization().defaults().user,
            "root"
        );
        match method {
            "Start" => assert!(matches!(decoded.request, Request::Start(_))),
            "Connect" => assert!(matches!(decoded.request, Request::Connect(None))),
            "List" => assert_eq!(decoded.request, Request::List),
            _ => {}
        }
        full(&pool);
        drop(decoded);
    }
}

#[tokio::test]
async fn wire_frames_lengths_trailers_and_json_are_bounded_before_dispatch() {
    let session = session().await;
    let mut settings = limits();
    settings.body.total_bytes = 8;
    settings.body.frame_bytes = 4;
    settings.body.frames = 2;
    let pool = CommandRequests::new(settings).unwrap();
    let transport = || {
        Body::from_stream(stream::once(async {
            Err::<Bytes, _>(std::io::Error::other("secret"))
        }))
    };
    let bodies = [
        (Body::from("         "), Error::BodyLimit),
        (Body::from("     "), Error::BodyLimit),
        (
            Body::from_stream(stream::iter(
                (0..3).map(|_| Ok::<_, std::io::Error>(Bytes::new())),
            )),
            Error::BodyLimit,
        ),
        (transport(), Error::Transport),
        (
            Body::from("bad"),
            Error::Protocol(ProtocolError::InvalidArgument),
        ),
    ];
    for (body, expected) in bodies {
        let auth = authenticate(&pool, &session).await;
        let deadline = auth.deadline();
        let failure = auth.decode("List", body, deadline).await.err().unwrap();
        assert_eq!(failure.error(), expected);
        full(&pool);
        drop(failure);
    }
    let trailers =
        HeaderMap::from_iter([("large".parse().unwrap(), "x".repeat(64).parse().unwrap())]);
    let body = Body::new(StreamBody::new(stream::iter([Ok::<_, std::io::Error>(
        Frame::trailers(trailers),
    )])));
    let auth = authenticate(&pool, &session).await;
    let failure = auth
        .decode("List", body, Instant::now() + Duration::from_secs(20))
        .await
        .err()
        .unwrap();
    assert_eq!(failure.error(), Error::BodyLimit);
}

#[tokio::test(start_paused = true)]
async fn stalled_body_and_late_decode_use_the_original_deadline() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    let auth = authenticate(&pool, &session).await;
    let deadline = auth.deadline();
    tokio::time::advance(Duration::from_secs(29)).await;
    let body = Body::from_stream(stream::pending::<Result<Bytes, std::io::Error>>());
    let task = tokio::spawn(auth.decode("List", body, deadline + Duration::from_mins(1)));
    tokio::time::advance(Duration::from_secs(1)).await;
    let failure = task.await.unwrap().err().unwrap();
    assert_eq!(failure.error(), Error::DeadlineExceeded);
    assert_eq!(failure.deadline(), deadline);
    full(&pool);
    drop(failure);
    let auth = authenticate(&pool, &session).await;
    let shortened = Instant::now();
    let failure = auth
        .decode("List", Body::from("{}"), shortened)
        .await
        .err()
        .unwrap();
    assert_eq!(failure.error(), Error::DeadlineExceeded);
    assert_eq!(failure.deadline(), shortened);
}

struct Gate(Arc<(Mutex<bool>, Condvar)>);
impl Gate {
    fn new() -> Self {
        Self(Arc::new((Mutex::new(false), Condvar::new())))
    }
    fn release(&self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        self.release();
    }
}

#[tokio::test]
async fn cancelled_decoder_keeps_auth_and_capacity_until_its_actual_exit() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    let auth = authenticate(&pool, &session).await;
    let gate = Gate::new();
    let worker_gate = gate.0.clone();
    let (entered, started) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(auth.decode_with(
        Body::from("{}"),
        Instant::now() + Duration::from_secs(30),
        move |_, _, _, _| {
            entered.send(()).unwrap();
            let _guard = worker_gate
                .1
                .wait_while(worker_gate.0.lock().unwrap(), |open| !*open)
                .unwrap();
            Ok(Request::List)
        },
    ));
    started.await.unwrap();
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    full(&pool);
    let mut initialize =
        Box::pin(session.initialize(br#"{"accessToken":"token","defaultUser":"changed"}"#));
    pending(initialize.as_mut()).await;
    gate.release();
    released(&pool).await;
    initialize.await.unwrap();
    assert_eq!(
        session
            .authorize(Some("token"))
            .await
            .unwrap()
            .defaults()
            .user,
        "changed"
    );
}
