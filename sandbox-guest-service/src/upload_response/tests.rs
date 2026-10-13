use super::*;
use crate::file_download::tests::reference::{Reference, hex};
use axum::body::Body;
use http_body_util::BodyExt;
use std::{collections::BTreeMap, fmt::Write, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

fn result(path: &str) -> UploadResult {
    UploadResult {
        path: path.into(),
        written: u64::MAX, // Internal accounting is not part of the HTTP reply.
        metadata: BTreeMap::new(),
    }
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
fn owner() -> (Owner, Arc<Semaphore>) {
    let slots = Arc::new(Semaphore::new(1));
    let guard = Arc::new(Arc::clone(&slots).try_acquire_owned().unwrap());
    (guard, slots)
}
async fn released(slots: Arc<Semaphore>) {
    let _slot = tokio::time::timeout(Duration::from_secs(5), slots.acquire_owned())
        .await
        .unwrap()
        .unwrap();
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let chunk = frame.unwrap().into_data().unwrap();
        bytes.extend_from_slice(&chunk);
        // Each chunk, including its clones, must be released before the next.
    }
    bytes
}
#[test]
fn envelope_omits_empty_metadata_preserves_order_and_counts_escaping_before_allocation() {
    assert_eq!(encode(&[], 2).unwrap(), b"[]");
    assert_eq!(encode(&[], 1), Err(Error::ResponseLimit));
    let mut entry = result("/guest/<a&b>\n");
    entry.metadata = BTreeMap::from([
        ("z".into(), "\u{2028}".into()),
        ("a".into(), "\"\\\t".into()),
    ]);
    let entries = vec![entry, result("/guest/plain")];
    let expected = br#"[{"metadata":{"a":"\"\\\t","z":"\u2028"},"name":"\u003ca\u0026b\u003e\n","path":"/guest/\u003ca\u0026b\u003e\n","type":"file"},{"name":"plain","path":"/guest/plain","type":"file"}]"#;
    assert_eq!(encode(&entries, expected.len()).unwrap(), expected);
    assert_eq!(
        encode(&entries, expected.len() - 1),
        Err(Error::ResponseLimit)
    );
}
#[tokio::test]
async fn body_backpressure_and_final_byte_clone_retain_original_capacity() {
    let entries = vec![result("/guest/file")];
    let expected = encode(&entries, 4096).unwrap();
    let (guard, slots) = owner();
    let response = success(entries, limits(), deadline(), guard).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "text/plain; charset=utf-8"
    );
    assert_eq!(
        response.headers()["content-length"],
        expected.len().to_string()
    );
    assert!(!response.headers().contains_key("x-content-type-options"));
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
    let chunk = next.await.unwrap().unwrap().into_data().unwrap();
    assert!(chunk.len() <= limits().chunk_bytes);
    actual.extend_from_slice(&chunk);
    drop(chunk);
    while let Some(frame) = body.frame().await {
        let chunk = frame.unwrap().into_data().unwrap();
        assert!(chunk.len() <= limits().chunk_bytes);
        actual.extend_from_slice(&chunk);
    }
    assert_eq!(actual, expected);
    released(slots).await;
}
#[tokio::test]
async fn dropped_body_and_original_expiry_keep_outstanding_bytes_and_never_fake_eof() {
    for (expire, chunk_bytes) in [(false, 17), (true, 17), (false, 65536), (true, 65536)] {
        let (guard, slots) = owner();
        let response = success(
            vec![result("/file")],
            Limits {
                chunk_bytes,
                ..limits()
            },
            deadline(),
            guard,
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
#[tokio::test]
async fn preparation_rejects_invalid_limit_oversize_and_original_expiry() {
    for (limit, end, expected) in [
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
                wire_bytes: 1,
                ..limits()
            },
            deadline(),
            Error::ResponseLimit,
        ),
        (limits(), Instant::now(), Error::DeadlineExceeded),
    ] {
        let (guard, slots) = owner();
        let error = success(vec![], limit, end, guard).await.err().unwrap();
        assert_eq!(error, expected);
        released(slots).await;
    }
}
fn reference_input(entries: &[UploadResult]) -> String {
    let mut input = String::from("R");
    for entry in entries {
        write!(input, "\t{}", hex(entry.path.as_bytes())).unwrap();
        for (key, value) in &entry.metadata {
            write!(input, "|{}:{}", hex(key.as_bytes()), hex(value.as_bytes())).unwrap();
        }
    }
    input
}
#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn fixed_go_upload_success_bytes_and_actual_http_headers_match() {
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    let mut cases = vec![
        vec![],
        vec![result("")],
        vec![result("/")],
        vec![result("/a//")],
    ];
    for c in (0..=127u8)
        .map(char::from)
        .chain(['中', 'İ', '\u{2028}', '\u{2029}', '\u{fffd}'])
    {
        let mut entry = result(&format!("/guest/{c}\\a"));
        entry.metadata =
            BTreeMap::from([("note".into(), c.to_string()), ("a".into(), "<>&".into())]);
        cases.push(vec![entry, result("/guest/plain")]);
    }
    // Cross the real net/http 2048-byte automatic Content-Length threshold.
    for length in [2047, 2048, 2049, 4096, 8192] {
        let mut entry = result("/guest/file");
        entry.metadata.insert("pad".into(), String::new());
        let overhead = encode(&[entry], 65536).unwrap().len();
        let mut entry = result("/guest/file");
        entry
            .metadata
            .insert("pad".into(), "x".repeat(length - overhead));
        cases.push(vec![entry]);
    }
    cases.push((0..128).map(|n| result(&format!("/guest/{n}"))).collect());
    let total = cases.len();
    for entries in cases {
        let input = reference_input(&entries);
        let response = success(entries, limits(), deadline(), Arc::new(()))
            .await
            .unwrap();
        let mut output = response.status().as_u16().to_string();
        for name in ["content-type", "content-length", "x-content-type-options"] {
            write!(
                output,
                "\t{}",
                hex(response
                    .headers()
                    .get(name)
                    .map_or(&[], HeaderValue::as_bytes))
            )
            .unwrap();
        }
        let bytes = drain(response.into_body()).await;
        write!(output, "\t{}", hex(&bytes)).unwrap();
        reference.check(&input, &output);
    }
    reference.finish();
    eprintln!(
        "Fixed Go upload success: {total} envelopes and real HTTP/1.1 status/header/body cases."
    );
}
