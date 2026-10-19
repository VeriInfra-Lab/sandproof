use super::*;
use crate::file_download::tests::reference::timezone::{MODIFIED, fixed_material, zone_limits};
use axum::http::HeaderValue;
use sandproof_guest_runtime::file_download::local_zone::{LoadLimits, Snapshot};
use std::{
    fs::{File, FileTimes},
    os::unix::fs::symlink,
    time::SystemTime,
};

fn with_snapshot(
    fixture: &Fixture,
    root: &Path,
    local: Arc<Snapshot>,
) -> FileHttp<Meta, Accounts, Snapshot> {
    let original = &fixture.http.inner;
    let Settings {
        files,
        upload,
        download,
        upload_response,
        mime,
        ..
    } = settings(root);
    FileHttp::new(
        original.session.clone(),
        original.requests.clone(),
        original.streams.clone(),
        original.accounts.clone(),
        Settings {
            files,
            upload,
            download,
            upload_response,
            mime,
            local,
        },
    )
    .unwrap()
}

#[tokio::test]
async fn file_timezone_snapshot_drives_real_get_head_conditions_after_source_removal() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("zone");
    let localtime = root.path().join("localtime");
    std::fs::write(&source, fixed_material()).unwrap();
    symlink("zone", &localtime).unwrap();
    let local = Arc::new(
        Snapshot::load_file(
            &localtime,
            LoadLimits {
                path_bytes: 4096,
                snapshot: zone_limits(),
            },
        )
        .unwrap(),
    );
    std::fs::remove_file(&source).unwrap();
    std::fs::remove_file(&localtime).unwrap();

    let path = root.path().join("file.bin");
    std::fs::write(&path, b"01234").unwrap();
    File::open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(
            SystemTime::UNIX_EPOCH
                + Duration::from_secs(u64::try_from(MODIFIED).unwrap())
                + Duration::from_nanos(123),
        ))
        .unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().mtime(), MODIFIED);

    let fixture = fixture(root.path()).await;
    let http = with_snapshot(&fixture, root.path(), local);
    for method in [Method::GET, Method::HEAD] {
        for (name, value, expected) in [
            (
                "if-modified-since",
                "Sunday, 06-Nov-94 09:50:37 XYZ",
                StatusCode::OK,
            ),
            (
                "if-modified-since",
                "Sunday, 06-Nov-94 09:50:38 XYZ",
                StatusCode::NOT_MODIFIED,
            ),
            (
                "if-unmodified-since",
                "Sunday, 06-Nov-94 09:50:37 XYZ",
                StatusCode::PRECONDITION_FAILED,
            ),
            (
                "if-range",
                "Sunday, 06-Nov-94 09:50:38 XYZ",
                StatusCode::PARTIAL_CONTENT,
            ),
            ("if-range", "Sunday, 06-Nov-94 09:50:37 XYZ", StatusCode::OK),
            (
                "if-range",
                "Sunday, 06-Nov-94 08:49:37 ABC",
                StatusCode::PARTIAL_CONTENT,
            ),
        ] {
            let mut req = request(method.clone(), "path=file.bin", Body::empty());
            req.headers_mut()
                .insert(name, HeaderValue::from_static(value));
            if name == "if-range" {
                req.headers_mut()
                    .insert("range", HeaderValue::from_static("bytes=1-2"));
            }
            let response = http.clone().handle(req).await.unwrap();
            assert_eq!(response.status(), expected, "{method} {name}: {value}");
            assert_eq!(
                response.headers()["last-modified"],
                "Sun, 06 Nov 1994 08:49:37 GMT"
            );
            if expected == StatusCode::PARTIAL_CONTENT {
                assert_eq!(response.headers()["content-range"], "bytes 1-2/5");
                assert_eq!(response.headers()["content-length"], "2");
            }
            let body = collect(response).await;
            let expected_body: &[u8] = if method == Method::HEAD {
                b""
            } else {
                match expected {
                    StatusCode::OK => b"01234",
                    StatusCode::PARTIAL_CONTENT => b"12",
                    _ => b"",
                }
            };
            assert_eq!(body.as_ref(), expected_body);
            drop(body);
            released(&fixture.requests).await;
        }
    }
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 12);
    assert_eq!(std::fs::read(path).unwrap(), b"01234");
}
