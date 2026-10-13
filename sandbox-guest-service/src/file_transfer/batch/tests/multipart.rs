use super::{Fixture, limits};
use crate::file_transfer::{FileStreamError, MultipartError, MultipartLimits};
use axum::body::{Body, Bytes};
use futures_util::stream;
use sandproof_guest_runtime::multipart::{Error, Limits};
use std::{fmt::Write as _, sync::atomic::Ordering};
use tokio::sync::{mpsc, oneshot};

pub(super) const CONTENT_TYPE: &str = "multipart/form-data; boundary=B";
pub(super) fn wire_limits() -> MultipartLimits {
    MultipartLimits {
        wire: Limits {
            chunk_bytes: 7,
            boundary_bytes: 64,
            header_bytes: 1024,
            header_count: 16,
            line_bytes: 256,
            parts: 32,
            total_bytes: 8192,
        },
        frame_bytes: 4096,
        frames: 8192,
        trailer_bytes: 256,
        trailer_fields: 8,
    }
}
pub(super) fn field(name: &str, filename: Option<&str>, data: &[u8], encoding: bool) -> Vec<u8> {
    let mut header = format!("--B\r\nContent-Disposition: form-data; name=\"{name}\"");
    if let Some(filename) = filename {
        write!(header, "; filename=\"{filename}\"").unwrap();
    }
    if encoding {
        header.push_str("\r\nContent-Transfer-Encoding: quoted-printable");
    }
    [header.as_bytes(), b"\r\n\r\n", data, b"\r\n"].concat()
}
pub(super) fn body(bytes: &[u8], chunk: usize) -> Body {
    let chunks: Vec<_> = bytes
        .chunks(chunk)
        .map(|s| Ok::<_, std::io::Error>(Bytes::copy_from_slice(s)))
        .collect();
    Body::from_stream(stream::iter(chunks))
}
pub(super) async fn quiesce(fixture: &Fixture) {
    drop(fixture.streams.slots.acquire().await.unwrap());
}

#[tokio::test]
async fn streamed_parts_share_authorization_metadata_and_bounded_decoding() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let input = [
        field("ignored", None, b"discard", false),
        field("file", Some("dir/a"), b"\0\xff\r\n--B-extra", false),
        field("file", Some("dir/b"), b"A=00=ff=\r\nB", true),
        field("file", Some("empty"), b"", false),
        b"--B--\r\nepilogue".to_vec(),
    ]
    .concat();
    let batch = fixture.batch(None, limits()).await;
    let reply = batch
        .receive_multipart(
            CONTENT_TYPE,
            body(&input, 1),
            Some(input.len() as u64),
            wire_limits(),
        )
        .await
        .unwrap();
    assert_eq!(reply.results().len(), 3);
    assert!(
        reply
            .results()
            .iter()
            .all(|r| r.metadata.get("note").map(String::as_str) == Some("batch"))
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.streams.slots.available_permits(), 0);
    assert_eq!(
        std::fs::read(root.path().join("dir/a")).unwrap(),
        b"\0\xff\r\n--B-extra"
    );
    assert_eq!(
        std::fs::read(root.path().join("dir/b")).unwrap(),
        b"A\0\xffB"
    );
    assert_eq!(std::fs::read(root.path().join("empty")).unwrap(), b"");
    drop(reply);
    quiesce(&fixture).await;
    assert_eq!(fixture.streams.slots.available_permits(), 1);
}

#[tokio::test]
async fn query_override_missing_filename_empty_forms_and_duplicate_effects() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let input = [field("file", None, b"first", false), b"--B--".to_vec()].concat();
    assert!(matches!(
        fixture
            .batch(None, limits())
            .await
            .receive_multipart(CONTENT_TYPE, body(&input, 7), None, wire_limits())
            .await,
        Err(MultipartError::Transfer(FileStreamError::InvalidArgument))
    ));
    assert!(!root.path().join("default").exists());
    let reply = fixture
        .batch(Some(""), limits())
        .await
        .receive_multipart(CONTENT_TYPE, body(&input, 7), None, wire_limits())
        .await
        .unwrap();
    assert_eq!(
        reply.results()[0].path,
        root.path().join("default").to_str().unwrap()
    );
    drop(reply);
    let input = [
        field("file", Some("alias/../first"), b"one", false),
        field("file", Some("first"), b"two", false),
        field("file", Some("later"), b"three", false),
        b"--B--".to_vec(),
    ]
    .concat();
    assert!(matches!(
        fixture
            .batch(None, limits())
            .await
            .receive_multipart(CONTENT_TYPE, body(&input, 7), None, wire_limits())
            .await,
        Err(MultipartError::Transfer(FileStreamError::InvalidArgument))
    ));
    quiesce(&fixture).await;
    assert_eq!(std::fs::read(root.path().join("first")).unwrap(), b"one");
    assert!(!root.path().join("later").exists());
    let empty = fixture
        .batch(None, limits())
        .await
        .receive_multipart(
            "multipart/mixed; boundary=B",
            body(b"--B--", 1),
            None,
            wire_limits(),
        )
        .await
        .unwrap();
    assert!(empty.results().is_empty());
}

#[tokio::test]
async fn decoding_or_final_boundary_failure_keeps_previous_and_partial_files() {
    for (suffix, expected, encoded) in [
        (b"prefix".as_slice(), b"prefix".as_slice(), false),
        (b"A=00=ff", b"A\0\xff", true),
        (b"prefix\x7fbad\r\n--B--", b"prefix", true),
        (b"prefix\r\n--B broken\r\n", b"prefix", false),
    ] {
        let root = tempfile::tempdir().unwrap();
        let fixture = Fixture::new(root.path()).await;
        let mut second = field("file", Some("partial"), b"", encoded);
        second.truncate(second.len() - 2);
        second.extend_from_slice(suffix);
        let input = [field("file", Some("first"), b"complete", false), second].concat();
        assert!(matches!(
            fixture
                .batch(None, limits())
                .await
                .receive_multipart(CONTENT_TYPE, body(&input, 1), None, wire_limits())
                .await,
            Err(MultipartError::Decode(_))
        ));
        quiesce(&fixture).await;
        assert_eq!(
            std::fs::read(root.path().join("first")).unwrap(),
            b"complete"
        );
        assert_eq!(
            std::fs::read(root.path().join("partial")).unwrap(),
            expected
        );
        assert_eq!(fixture.streams.slots.available_permits(), 1);
    }
}

#[tokio::test]
async fn known_and_observed_body_limits_reject_without_false_success() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let input = [
        field("file", Some("file"), b"content", false),
        b"--B--".to_vec(),
    ]
    .concat();
    let err = fixture
        .batch(None, limits())
        .await
        .receive_multipart(CONTENT_TYPE, body(&input, 3), Some(8193), wire_limits())
        .await
        .err();
    assert_eq!(err, Some(MultipartError::InputLimit));
    assert!(!root.path().join("file").exists());
    let err = fixture
        .batch(None, limits())
        .await
        .receive_multipart(
            CONTENT_TYPE,
            body(&input, 128),
            None,
            MultipartLimits {
                frame_bytes: 8,
                ..wire_limits()
            },
        )
        .await
        .err();
    assert_eq!(err, Some(MultipartError::InputLimit));
    assert!(!root.path().join("file").exists());
    let err = fixture
        .batch(None, limits())
        .await
        .receive_multipart(
            CONTENT_TYPE,
            body(&input, 3),
            Some(input.len() as u64 + 1),
            wire_limits(),
        )
        .await
        .err();
    assert_eq!(err, Some(MultipartError::Transport));
    quiesce(&fixture).await;
    assert_eq!(std::fs::read(root.path().join("file")).unwrap(), b"content");
    for budget in [
        MultipartLimits {
            frames: 2,
            ..wire_limits()
        },
        MultipartLimits {
            wire: Limits {
                total_bytes: 8,
                ..wire_limits().wire
            },
            ..wire_limits()
        },
    ] {
        let err = fixture
            .batch(None, limits())
            .await
            .receive_multipart(
                CONTENT_TYPE,
                body(b"--B--\r\nignored epilogue", 1),
                None,
                budget,
            )
            .await
            .err();
        assert_eq!(err, Some(MultipartError::InputLimit));
    }
    let ignored = [
        field("ignored", None, &[b'x'; 100], false),
        b"--B--".to_vec(),
    ]
    .concat();
    let err = fixture
        .batch(None, limits())
        .await
        .receive_multipart(
            CONTENT_TYPE,
            body(&ignored, 1),
            None,
            MultipartLimits {
                wire: Limits {
                    total_bytes: 80,
                    ..wire_limits().wire
                },
                ..wire_limits()
            },
        )
        .await
        .err();
    assert_eq!(err, Some(MultipartError::InputLimit));
}

// Explicit signal fires when the next body poll observes an empty channel.
// It establishes that all preceding frames/file writes have been consumed.
pub(super) fn gated_body() -> (
    mpsc::Sender<Result<Bytes, std::io::Error>>,
    Body,
    oneshot::Receiver<()>,
) {
    let (send, receive) = mpsc::channel(1);
    let (observed, notice) = oneshot::channel();
    let stream = stream::unfold(
        (receive, Some(observed)),
        |(mut receive, mut observed)| async move {
            if receive.is_empty()
                && let Some(signal) = observed.take()
            {
                let _ = signal.send(());
            }
            let item = receive.recv().await?;
            Some((item, (receive, observed)))
        },
    );
    (send, Body::from_stream(stream), notice)
}

#[tokio::test]
async fn closing_boundary_still_holds_auth_and_capacity_until_actual_http_eof() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let (send, body, notice) = gated_body();
    let input = [
        field("file", Some("done"), b"written", false),
        b"--B--\r\n".to_vec(),
    ]
    .concat();
    send.send(Ok(Bytes::from(input))).await.unwrap();
    let batch = fixture.batch(None, limits()).await;
    let task = tokio::spawn(batch.receive_multipart(CONTENT_TYPE, body, None, wire_limits()));
    notice.await.unwrap();
    assert!(!task.is_finished());
    assert_eq!(fixture.streams.slots.available_permits(), 0);
    assert_eq!(std::fs::read(root.path().join("done")).unwrap(), b"written");
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
    assert_eq!(fixture.streams.slots.available_permits(), 0);
    drop(reply);
    quiesce(&fixture).await;
}

#[tokio::test]
async fn transport_error_after_mime_end_cannot_return_a_batch_success() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let (send, body, notice) = gated_body();
    send.send(Ok(Bytes::from(
        [
            field("file", Some("done"), b"written", false),
            b"--B--\r\n".to_vec(),
        ]
        .concat(),
    )))
    .await
    .unwrap();
    let task = tokio::spawn(fixture.batch(None, limits()).await.receive_multipart(
        CONTENT_TYPE,
        body,
        None,
        wire_limits(),
    ));
    notice.await.unwrap();
    send.send(Err(std::io::Error::other("synthetic disconnect")))
        .await
        .unwrap();
    drop(send);
    assert!(matches!(
        task.await.unwrap(),
        Err(MultipartError::Transport)
    ));
    quiesce(&fixture).await;
    assert_eq!(std::fs::read(root.path().join("done")).unwrap(), b"written");
}

#[tokio::test]
async fn cancellation_or_original_deadline_stops_before_next_file() {
    for cancelled in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let fixture = Fixture::new(root.path()).await;
        let (send, body, notice) = gated_body();
        // Last suffix is unambiguous payload, so it is physically written
        // before the input requests the gated next frame.
        let input = [
            field("file", Some("first"), b"one", false),
            field("file", Some("partial"), b"prefixX", false),
        ]
        .concat();
        send.send(Ok(Bytes::copy_from_slice(&input[..input.len() - 2])))
            .await
            .unwrap();
        let task = tokio::spawn(fixture.batch(None, limits()).await.receive_multipart(
            CONTENT_TYPE,
            body,
            None,
            wire_limits(),
        ));
        notice.await.unwrap();
        assert_eq!(fixture.streams.slots.available_permits(), 0);
        if cancelled {
            task.abort();
            assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        } else {
            tokio::time::pause();
            tokio::time::advance(std::time::Duration::from_secs(31)).await;
            assert!(matches!(
                task.await.unwrap(),
                Err(MultipartError::Transfer(FileStreamError::DeadlineExceeded))
            ));
            tokio::time::resume();
        }
        drop(send);
        quiesce(&fixture).await;
        assert_eq!(std::fs::read(root.path().join("first")).unwrap(), b"one");
        assert_eq!(
            std::fs::read(root.path().join("partial")).unwrap(),
            b"prefixX"
        );
    }
}

#[tokio::test]
async fn header_truncation_and_missing_closing_marker_are_explicit_failures() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    assert!(matches!(
        fixture
            .batch(None, limits())
            .await
            .receive_multipart(CONTENT_TYPE, body(b"--B\r\n", 1), None, wire_limits())
            .await,
        Err(MultipartError::Decode(Error::TruncatedHeaders))
    ));
    assert!(matches!(
        fixture
            .batch(None, limits())
            .await
            .receive_multipart(CONTENT_TYPE, body(b"", 1), None, wire_limits())
            .await,
        Err(MultipartError::Decode(Error::Truncated))
    ));
}

#[tokio::test]
async fn trailers_empty_frames_and_frames_after_trailers_are_bounded() {
    use axum::http::{HeaderMap, HeaderValue};
    use http_body_util::StreamBody;
    use hyper::body::Frame;
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let frame_body = |frames: Vec<Frame<Bytes>>| {
        Body::new(StreamBody::new(stream::iter(
            frames.into_iter().map(Ok::<_, std::io::Error>),
        )))
    };
    let mut headers = HeaderMap::new();
    headers.insert("x-end", HeaderValue::from_static("done"));
    let frames = vec![
        Frame::data(Bytes::from_static(b"--B--\r\n")),
        Frame::trailers(headers.clone()),
    ];
    let reply = fixture
        .batch(None, limits())
        .await
        .receive_multipart(CONTENT_TYPE, frame_body(frames), None, wire_limits())
        .await
        .unwrap();
    assert!(reply.results().is_empty());
    drop(reply);
    for budget in [
        MultipartLimits {
            trailer_bytes: 1,
            ..wire_limits()
        },
        MultipartLimits {
            trailer_fields: 1,
            ..wire_limits()
        },
    ] {
        let mut headers = headers.clone();
        headers.append("x-end", HeaderValue::from_static("duplicate"));
        let frames = vec![
            Frame::data(Bytes::from_static(b"--B--\r\n")),
            Frame::trailers(headers),
        ];
        assert!(matches!(
            fixture
                .batch(None, limits())
                .await
                .receive_multipart(CONTENT_TYPE, frame_body(frames), None, budget)
                .await,
            Err(MultipartError::InputLimit)
        ));
    }
    let frames = vec![
        Frame::data(Bytes::from_static(b"--B--\r\n")),
        Frame::trailers(headers),
        Frame::data(Bytes::from_static(b"late")),
    ];
    assert!(matches!(
        fixture
            .batch(None, limits())
            .await
            .receive_multipart(CONTENT_TYPE, frame_body(frames), None, wire_limits())
            .await,
        Err(MultipartError::Transport)
    ));
    let frames = vec![
        Frame::data(Bytes::new()),
        Frame::data(Bytes::new()),
        Frame::data(Bytes::new()),
    ];
    assert!(matches!(
        fixture
            .batch(None, limits())
            .await
            .receive_multipart(
                CONTENT_TYPE,
                frame_body(frames),
                None,
                MultipartLimits {
                    frames: 2,
                    ..wire_limits()
                }
            )
            .await,
        Err(MultipartError::InputLimit)
    ));
}
