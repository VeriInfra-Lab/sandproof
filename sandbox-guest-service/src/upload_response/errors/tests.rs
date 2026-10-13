use super::*;
use crate::file_download::tests::reference::{Reference, hex};
use http_body_util::BodyExt;
use std::{fmt::Write, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

fn fault(stage: Stage, path: &[u8], errno: Option<i32>, attribute: Option<&[u8]>) -> Fault {
    Fault {
        stage,
        path: path.to_vec(),
        errno: errno.map(LinuxErrno),
        attribute: attribute.map(<[u8]>::to_vec),
    }
}
fn limits() -> Limits {
    Limits {
        wire_bytes: 65536,
        chunk_bytes: 37,
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}

#[test]
fn exact_wire_budget_and_missing_observations_never_fabricate_an_error() {
    let observed = fault(Stage::Open, b"/guest/<&>\xff", Some(13), None);
    let bytes = payload(&observed, None, 65536).unwrap();
    assert_eq!(payload(&observed, None, bytes.len()).unwrap(), bytes);
    assert_eq!(
        payload(&observed, None, bytes.len() - 1),
        Err(Error::ResponseLimit)
    );
    assert_eq!(
        payload(&fault(Stage::Open, b"/file", None, None), None, 4096),
        Err(Error::UnclassifiedFault)
    );
    assert_eq!(
        payload(
            &fault(Stage::MetadataSet, b"/file", Some(13), None),
            None,
            4096
        ),
        Err(Error::UnclassifiedFault)
    );
}
#[tokio::test]
async fn error_response_clones_keep_capacity_and_original_deadline() {
    let slots = Arc::new(Semaphore::new(1));
    let guard = Arc::new(Arc::clone(&slots).try_acquire_owned().unwrap());
    let response = io_failure(
        fault(Stage::Write, b"/file", Some(28), None),
        Some(92),
        limits(),
        deadline(),
        guard,
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 507);
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let mut body = response.into_body();
    let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = first.clone();
    drop(first);
    let next = body.frame();
    tokio::pin!(next);
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(next.as_mut().poll(cx).is_pending()))
            .await
    );
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    assert!(next.await.unwrap().is_err());
    tokio::time::resume();
    drop(body);
    assert_eq!(slots.available_permits(), 0);
    drop(copy);
    let _slot = tokio::time::timeout(Duration::from_secs(5), slots.acquire_owned())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn fixed_go_upload_io_error_bytes_headers_and_status_match() {
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    let paths: &[&[u8]] = &[
        b"/file",
        b"/guest/<&>\"\\\t\n",
        b"/guest/\0",
        b"/guest/\xff\xfe",
        "/guest/中文\u{2028}\u{2029}".as_bytes(),
    ];
    let mut cases = 0;
    for stage in [
        Stage::ParentStat,
        Stage::ParentNotDirectory,
        Stage::ParentCreate,
        Stage::ParentChown,
        Stage::Stat,
        Stage::Directory,
        Stage::Chown,
        Stage::Open,
        Stage::FileChown,
        Stage::Write,
        Stage::WriteZero,
        Stage::MetadataList,
        Stage::MetadataRemove,
        Stage::MetadataSet,
    ] {
        for errno in [5, 13, 28, 122, 4096, -1] {
            for path in paths {
                for length in [None, Some(0), Some(17), Some(i64::MAX as u64)] {
                    let name = matches!(stage, Stage::MetadataRemove | Stage::MetadataSet)
                        .then_some(b"user.e2b.\"&\xff".as_slice());
                    compare(
                        &mut reference,
                        fault(stage, path, Some(errno), name),
                        length,
                    )
                    .await;
                    cases += 1;
                }
            }
        }
    }
    for byte in 0..=255 {
        let name = [b"user.e2b.".as_slice(), &[byte]].concat();
        compare(
            &mut reference,
            fault(Stage::MetadataRemove, b"/file", Some(13), Some(&name)),
            None,
        )
        .await;
        cases += 1;
    }
    // Automatic Content-Length includes json.Encoder's final newline.
    for length in [2047, 2048, 2049, 4096] {
        let overhead = payload(&fault(Stage::Directory, b"/", None, None), None, 65536)
            .unwrap()
            .len();
        let path = [b"/".as_slice(), vec![b'x'; length - overhead].as_slice()].concat();
        compare(
            &mut reference,
            fault(Stage::Directory, &path, None, None),
            None,
        )
        .await;
        cases += 1;
    }
    reference.finish();
    eprintln!(
        "Fixed Go upload I/O errors: {cases} status/header/JSON cases with original processFile and error wrappers."
    );
}
async fn compare(reference: &mut Reference, fault: Fault, expected: Option<u64>) {
    let input = format!(
        "J\t{:?}\t{}\t{}\t{}\t{}",
        fault.stage,
        fault.errno.map_or(5, |n| n.0),
        hex(&fault.path),
        hex(fault.attribute.as_deref().unwrap_or_default()),
        expected.map_or_else(|| "-1".into(), |n| n.to_string())
    );
    let response = io_failure(fault, expected, limits(), deadline(), Arc::new(()))
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
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        bytes.extend_from_slice(&frame.unwrap().into_data().unwrap());
    }
    write!(output, "\t{}", hex(&bytes)).unwrap();
    reference.check(&input, &output);
}
