use super::*;
use axum::body::Body;
use http_body_util::BodyExt;
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

fn fault(stage: FaultStage, errno: Option<i32>) -> FileFault {
    FileFault {
        stage,
        path: b"/from".to_vec(),
        context: match stage {
            FaultStage::Rename => Some(FaultContext::RenameDestination(b"/to".to_vec())),
            FaultStage::WalkStat
            | FaultStage::WalkOpen
            | FaultStage::WalkRead
            | FaultStage::WalkType
            | FaultStage::WalkEntryStat => Some(FaultContext::WalkRoot(b"/root".to_vec())),
            FaultStage::RemoveRead => Some(FaultContext::RemoveReadName(b"leaf".to_vec())),
            _ => None,
        },
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
        FaultStage::RemoveAll,
        Some(22),
        500,
        "internal",
        "error removing file or directory: RemoveAll /from: invalid argument",
    ),
    (
        FaultStage::RemoveParentOpen,
        Some(20),
        500,
        "internal",
        "error removing file or directory: open /from: not a directory",
    ),
    (
        FaultStage::RemoveUnlink,
        Some(13),
        500,
        "internal",
        "error removing file or directory: unlinkat /from: permission denied",
    ),
    (
        FaultStage::RemoveOpenDir,
        Some(13),
        500,
        "internal",
        "error removing file or directory: openfdat /from: permission denied",
    ),
    (
        FaultStage::RemoveRead,
        Some(5),
        500,
        "internal",
        "error removing file or directory: readdirnames /from: readdirent leaf: input/output error",
    ),
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
    (
        FaultStage::ResolveStat,
        Some(2),
        404,
        "not_found",
        "path not found: lstat /from: no such file or directory",
    ),
    (
        FaultStage::ResolveStat,
        Some(13),
        500,
        "internal",
        "error resolving symlink: lstat /from: permission denied",
    ),
    (
        FaultStage::ResolveReadlink,
        Some(22),
        500,
        "internal",
        "error resolving symlink: readlink /from: invalid argument",
    ),
    (
        FaultStage::ResolveNotDirectory,
        None,
        500,
        "internal",
        "error resolving symlink: not a directory",
    ),
    (
        FaultStage::ResolveCycle,
        None,
        400,
        "failed_precondition",
        "cyclic symlink or chain >255 links at \"/from\"",
    ),
    (
        FaultStage::DirectoryStat,
        Some(2),
        404,
        "not_found",
        "directory not found: stat /from: no such file or directory",
    ),
    (
        FaultStage::DirectoryStat,
        Some(13),
        500,
        "internal",
        "error getting file info: stat /from: permission denied",
    ),
    (
        FaultStage::DirectoryNotDirectory,
        None,
        400,
        "invalid_argument",
        "path is not a directory: /from",
    ),
    (
        FaultStage::WalkStat,
        Some(2),
        500,
        "internal",
        "error reading directory /root: lstat /from: no such file or directory",
    ),
    (
        FaultStage::WalkOpen,
        Some(13),
        500,
        "internal",
        "error reading directory /root: open /from: permission denied",
    ),
    (
        FaultStage::WalkRead,
        Some(5),
        500,
        "internal",
        "error reading directory /root: readdirent /from: input/output error",
    ),
    (
        FaultStage::WalkType,
        Some(13),
        500,
        "internal",
        "error reading directory /root: fstatat /from: permission denied",
    ),
    (
        FaultStage::WalkEntryStat,
        Some(20),
        500,
        "internal",
        "error reading directory /root: internal: error getting file info: lstat /from: not a directory",
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
    diagnostic.context = Some(FaultContext::RenameDestination(
        "/雪\u{2028}".as_bytes().to_vec(),
    ));
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
                context: None,
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

#[test]
fn cycle_quoting_and_context_roles_do_not_invent_missing_paths() {
    let mut cycle = fault(FaultStage::ResolveCycle, None);
    cycle.path = b"/q\xff\"\n".to_vec();
    let bytes = payload(&cycle, 4096).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value["message"],
        r#"cyclic symlink or chain >255 links at "/q\xff\"\n""#
    );
    assert_eq!(payload(&cycle, bytes.len() - 1), Err(Error::ResponseLimit));
    for mut observed in [
        fault(FaultStage::Rename, Some(2)),
        fault(FaultStage::WalkOpen, Some(13)),
        fault(FaultStage::RemoveRead, Some(5)),
        cycle,
    ] {
        observed.context = match observed.stage {
            FaultStage::Rename => Some(FaultContext::WalkRoot(b"/wrong".to_vec())),
            FaultStage::WalkOpen => None,
            _ => Some(FaultContext::RenameDestination(b"/wrong".to_vec())),
        };
        assert_eq!(payload(&observed, 4096), Err(Error::UnclassifiedFault));
    }
}
