use super::*;
use axum::body::Body;
use http_body_util::BodyExt;
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

fn fault(stage: FaultStage, errno: Option<i32>) -> FileFault {
    FileFault {
        stage,
        path: b"/from".to_vec(),
        destination: Some(b"/to".to_vec()),
        errno: errno.map(LinuxErrno),
    }
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut result = Vec::new();
    while let Some(frame) = body.frame().await {
        let bytes = frame.unwrap().into_data().unwrap();
        result.extend_from_slice(&bytes);
    }
    result
}
const CASES: &[(FaultStage, Option<i32>, u16, &str, &str)] = &[
    (
        FaultStage::EntryStat,
        Some(2),
        404,
        "not_found",
        "file not found: lstat /from: no such file or directory",
    ),
    (
        FaultStage::EntryStat,
        Some(20),
        500,
        "internal",
        "error getting file info: lstat /from: not a directory",
    ),
    (
        FaultStage::MakeDirStat,
        Some(40),
        500,
        "internal",
        "error getting file info: stat /from: too many levels of symbolic links",
    ),
    (
        FaultStage::MakeDirExists,
        None,
        409,
        "already_exists",
        "directory already exists: /from",
    ),
    (
        FaultStage::MakeDirNotDirectory,
        None,
        400,
        "invalid_argument",
        "path already exists but it is not a directory: /from",
    ),
    (
        FaultStage::Rename,
        Some(2),
        404,
        "not_found",
        "source file not found: rename /from /to: no such file or directory",
    ),
    (
        FaultStage::Rename,
        Some(18),
        500,
        "internal",
        "error renaming: rename /from /to: invalid cross-device link",
    ),
    (
        FaultStage::ParentStat,
        Some(13),
        500,
        "internal",
        "failed to stat directory: stat /from: permission denied",
    ),
    (
        FaultStage::ParentNotDirectory,
        None,
        500,
        "internal",
        "path is a file: /from",
    ),
    (
        FaultStage::ParentCreate,
        Some(28),
        500,
        "internal",
        "failed to create directory: mkdir /from: no space left on device",
    ),
    (
        FaultStage::ParentChown,
        Some(1),
        500,
        "internal",
        "failed to chown directory: chown /from: operation not permitted",
    ),
];

#[tokio::test]
async fn stage_selects_fixed_envd_message_code_and_headers() {
    for &(stage, errno, status, code, message) in CASES {
        let response = io_failure(
            fault(stage, errno),
            Limits {
                wire_bytes: 4096,
                chunk_bytes: 7,
            },
            Instant::now() + Duration::from_secs(30),
            Arc::new(()),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["content-type"], "application/json");
        assert_eq!(response.headers()["accept-encoding"], "gzip");
        assert!(!response.headers().contains_key("content-encoding"));
        let bytes = drain(response.into_body()).await;
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            serde_json::json!({"code":code,"message":message})
        );
        assert!(!bytes.ends_with(b"\n"));
    }
}
#[tokio::test]
async fn escaped_error_budget_missing_observation_and_byte_copies_retain_capacity() {
    let mut diagnostic = fault(FaultStage::Rename, Some(2));
    diagnostic.path = b"/secret\xff\"<>&\n".to_vec();
    diagnostic.destination = Some("/雪\u{2028}".as_bytes().to_vec());
    assert!(!format!("{diagnostic:?}").contains("secret"));
    let bytes = payload(&diagnostic, 4096).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value["message"],
        "source file not found: rename /secret\u{fffd}\"<>&\n /雪\u{2028}: no such file or directory"
    );
    assert!(bytes.windows(6).any(|p| p == b"\\u003c"));
    assert_eq!(payload(&diagnostic, bytes.len()).unwrap(), bytes);
    let end = Instant::now() + Duration::from_secs(30);
    for (observed, limit, error) in [
        (diagnostic.clone(), bytes.len() - 1, Error::ResponseLimit),
        (
            fault(FaultStage::EntryStat, None),
            4096,
            Error::UnclassifiedFault,
        ),
        (
            FileFault {
                destination: None,
                ..diagnostic.clone()
            },
            4096,
            Error::UnclassifiedFault,
        ),
    ] {
        let slots = Arc::new(Semaphore::new(1));
        let owner: Owner = Arc::new(slots.clone().try_acquire_owned().unwrap());
        let failure = io_failure(
            observed,
            Limits {
                wire_bytes: limit,
                chunk_bytes: 7,
            },
            end,
            owner,
        )
        .await
        .unwrap_err();
        assert_eq!(failure.error(), error);
        assert_eq!(failure.deadline(), end);
        assert_eq!(slots.available_permits(), 0);
        drop(failure);
        drop(
            tokio::time::timeout(Duration::from_secs(5), slots.acquire_owned())
                .await
                .unwrap()
                .unwrap(),
        );
    }
    let slots = Arc::new(Semaphore::new(1));
    let owner: Owner = Arc::new(slots.clone().try_acquire_owned().unwrap());
    let mut body = io_failure(
        diagnostic,
        Limits {
            wire_bytes: 4096,
            chunk_bytes: 7,
        },
        end,
        owner,
    )
    .await
    .unwrap()
    .into_body();
    let copy = body
        .frame()
        .await
        .unwrap()
        .unwrap()
        .into_data()
        .unwrap()
        .clone();
    drop(body);
    assert_eq!(slots.available_permits(), 0);
    drop(copy);
    drop(
        tokio::time::timeout(Duration::from_secs(5), slots.acquire_owned())
            .await
            .unwrap()
            .unwrap(),
    );
}
