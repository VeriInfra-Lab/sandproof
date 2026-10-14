use super::*;
use crate::file_download::tests::reference::{Reference, hex};
use axum::body::Body;
use file_protocol::{Entry, FileType, ModifiedTime};
use http_body_util::BodyExt;
use std::{collections::BTreeMap, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

mod gzip;

enum Records {
    One(Entry),
    Many(Vec<Entry>),
    Empty,
}
struct Retained {
    records: Records,
    _slot: OwnedSemaphorePermit,
}
fn view(retained: &Retained) -> file_protocol::Response<'_> {
    match &retained.records {
        Records::One(entry) => file_protocol::Response::Entry(entry),
        Records::Many(entries) => file_protocol::Response::Entries(entries),
        Records::Empty => file_protocol::Response::Empty,
    }
}
fn entry() -> Entry {
    Entry {
        name: "file".into(),
        kind: FileType::File,
        path: "/guest/file".into(),
        size: i64::MAX,
        mode: 420,
        permissions: "-rw-r--r--".into(),
        owner: "user".into(),
        group: "group".into(),
        modified_time: Some(ModifiedTime::new(1, 123_000_000).unwrap()),
        symlink_target: None,
        metadata: BTreeMap::new(),
    }
}
fn retain(records: Records) -> (Arc<Retained>, Arc<Semaphore>) {
    let slots = Arc::new(Semaphore::new(1));
    let retained = Arc::new(Retained {
        records,
        _slot: Arc::clone(&slots).try_acquire_owned().unwrap(),
    });
    (retained, slots)
}
fn limits() -> Limits {
    Limits {
        wire_bytes: 65536,
        chunk_bytes: 17,
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
async fn released(slots: Arc<Semaphore>) {
    let _slot = tokio::time::timeout(Duration::from_secs(5), slots.acquire_owned())
        .await
        .unwrap()
        .unwrap();
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut data = Vec::new();
    while let Some(frame) = body.frame().await {
        let chunk = frame.unwrap().into_data().unwrap();
        data.extend_from_slice(&chunk);
        // Release every chunk before polling again: this preserves backpressure.
    }
    data
}

#[tokio::test]
async fn success_uses_bounded_file_json_for_each_shape_and_header_length_boundary() {
    for records in [
        Records::Empty,
        Records::Many(vec![]),
        Records::One(entry()),
        Records::Many(vec![entry(), entry()]),
    ] {
        let (records, slots) = retain(records);
        let expected = file_protocol::encode_response(view(&records), 65536).unwrap();
        let response = send(
            records,
            view,
            Limits {
                wire_bytes: expected.len(),
                ..limits()
            },
            deadline(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "application/json");
        assert_eq!(response.headers()["accept-encoding"], "gzip");
        assert!(!response.headers().contains_key("content-encoding"));
        assert!(!response.headers().contains_key("x-content-type-options"));
        assert_eq!(drain(response.into_body()).await, expected);
        released(slots).await;
    }
}

#[tokio::test]
async fn backpressure_retains_records_and_permit_until_last_copy_is_released() {
    let (records, slots) = retain(Records::One(entry()));
    let expected = file_protocol::encode_response(view(&records), 65536).unwrap();
    let response = send(records, view, limits(), deadline()).await.unwrap();
    let mut body = response.into_body();
    let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = first.clone();
    let mut actual = first.to_vec();
    drop(first);
    let next = body.frame();
    tokio::pin!(next);
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(next.as_mut().poll(cx).is_pending()))
            .await
    );
    assert_eq!(slots.available_permits(), 0);
    drop(copy);
    let bytes = next.await.unwrap().unwrap().into_data().unwrap();
    assert!(bytes.len() <= limits().chunk_bytes);
    actual.extend_from_slice(&bytes);
    drop(bytes);
    actual.extend(drain(body).await);
    assert_eq!(actual, expected);
    released(slots).await;
}

#[tokio::test]
async fn cancellation_and_original_expiry_do_not_release_outstanding_byte_clones_or_fake_eof() {
    for expire in [false, true] {
        for chunk_bytes in [17, 65536] {
            let (records, slots) = retain(Records::One(entry()));
            let response = send(
                records,
                view,
                Limits {
                    chunk_bytes,
                    ..limits()
                },
                deadline(),
            )
            .await
            .unwrap();
            let mut body = response.into_body();
            let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
            let copy = bytes.clone();
            drop(bytes);
            if expire {
                tokio::time::pause();
                tokio::time::advance(Duration::from_secs(31)).await;
                assert!(body.frame().await.unwrap().is_err());
                tokio::time::resume();
            }
            drop(body);
            assert_eq!(slots.available_permits(), 0);
            drop(copy);
            released(slots).await;
        }
    }
}

#[tokio::test]
async fn preparation_failures_keep_the_original_records_and_deadline() {
    for (limits, end, error) in [
        (
            Limits {
                wire_bytes: 0,
                ..limits()
            },
            deadline(),
            Error::InvalidConfiguration,
        ),
        (
            Limits {
                chunk_bytes: 0,
                ..limits()
            },
            deadline(),
            Error::InvalidConfiguration,
        ),
        (
            Limits {
                wire_bytes: usize::MAX,
                ..limits()
            },
            deadline(),
            Error::InvalidConfiguration,
        ),
        (
            Limits {
                chunk_bytes: usize::MAX,
                ..limits()
            },
            deadline(),
            Error::InvalidConfiguration,
        ),
        (
            Limits {
                wire_bytes: 1,
                ..limits()
            },
            deadline(),
            Error::ResponseLimit,
        ),
        (limits(), Instant::now(), Error::DeadlineExceeded),
    ] {
        let (records, slots) = retain(Records::Empty);
        let failure = send(records, view, limits, end).await.unwrap_err();
        assert_eq!(failure.error(), error);
        assert_eq!(failure.deadline(), end);
        assert_eq!(slots.available_permits(), 0);
        assert!(!format!("{failure:?}").contains("guest"));
        drop(failure);
        released(slots).await;
    }
    let mut escaped = entry();
    escaped
        .metadata
        .insert("\"\\\n".into(), "\"\\\n".repeat(32));
    let (records, slots) = retain(Records::One(escaped));
    let length = file_protocol::encode_response(view(&records), 65536)
        .unwrap()
        .len();
    let failure = send(
        records,
        view,
        Limits {
            wire_bytes: length - 1,
            ..limits()
        },
        deadline(),
    )
    .await
    .unwrap_err();
    assert_eq!(failure.error(), Error::ResponseLimit);
    assert_eq!(slots.available_permits(), 0);
    drop(failure);
    released(slots).await;
}

#[tokio::test]
async fn cancelled_encoding_worker_retains_original_records_until_it_actually_returns() {
    use std::sync::Mutex;
    struct Gated {
        retained: Retained,
        entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        resume: Mutex<std::sync::mpsc::Receiver<()>>,
    }
    fn gated_view(records: &Gated) -> file_protocol::Response<'_> {
        records
            .entered
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        records
            .resume
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        view(&records.retained)
    }
    for encoding in [Encoding::Identity, Encoding::Gzip] {
        let slots = Arc::new(Semaphore::new(1));
        let (entered, started) = tokio::sync::oneshot::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        let records = Arc::new(Gated {
            retained: Retained {
                records: Records::One(entry()),
                _slot: Arc::clone(&slots).try_acquire_owned().unwrap(),
            },
            entered: Mutex::new(Some(entered)),
            resume: Mutex::new(resumed),
        });
        let task = tokio::spawn(send_with_encoding(
            records,
            gated_view,
            limits(),
            deadline(),
            encoding,
        ));
        started.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(slots.available_permits(), 0);
        resume.send(()).unwrap();
        released(slots).await;
    }
}

#[tokio::test]
#[ignore = "explicit Go 1.26.8 / Connect v1.18.1 oracle; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_connect_identity_json_transport_reference() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut cases = vec![Records::Empty, Records::Many(vec![]), Records::One(entry())];
    let mut escaped = entry();
    escaped.path = "/guest/路径<>&\"\\\n\u{2028}".into();
    escaped.metadata.insert("label".into(), "\t<>".into());
    cases.push(Records::Many(vec![escaped, entry()]));
    for length in [2047, 2048, 2049, 8192] {
        let mut large = entry();
        let base = file_protocol::encode_response(file_protocol::Response::Entry(&large), 65536)
            .unwrap()
            .len();
        large.name.push_str(&"a".repeat(length - base));
        cases.push(Records::One(large));
    }
    for records in cases {
        let (records, slots) = retain(records);
        let expected = file_protocol::encode_response(view(&records), 65536).unwrap();
        let response = send(records, view, limits(), deadline()).await.unwrap();
        let mut observed = vec![response.status().as_u16().to_string()];
        for name in [
            "content-type",
            "content-length",
            "accept-encoding",
            "content-encoding",
            "x-content-type-options",
            "vary",
        ] {
            observed.push(hex(response
                .headers()
                .get(name)
                .map_or(b"", |value| value.as_bytes())));
        }
        let bytes = drain(response.into_body()).await;
        observed.push(hex(&bytes));
        assert_eq!(bytes, expected);
        oracle.check(&hex(&expected), &observed.join("\t"));
        released(slots).await;
    }
    oracle.finish();
    eprintln!(
        "8 fixed Connect identity JSON status/header/body cases; transport only, passthrough test codec."
    );
}
