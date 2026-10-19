//! Original pinned Go service methods, opt-in Linux execution. This compares
//! real syscall failures with the Rust HTTP component, not a Go HTTP listener.
use super::*;
use std::{
    io::Write,
    process::{Command, Stdio},
};

#[tokio::test]
#[ignore = "requires SANDPROOF_GUEST_GO_FILES_REFERENCE built from pinned envd"]
async fn original_go_methods_match_observed_unary_error_messages() {
    let binary = std::env::var("SANDPROOF_GUEST_GO_FILES_REFERENCE")
        .expect("original Go reference required");
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    prepare_error_tree(root.path());
    for (method, value) in [
        ("Stat", serde_json::json!({"path":"missing"})),
        ("Stat", serde_json::json!({"path":"missing<>&\"\n雪"})),
        ("Stat", serde_json::json!({"path":"file/child"})),
        ("Stat", serde_json::json!({"path":"nul\0"})),
        ("MakeDir", serde_json::json!({"path":"dir"})),
        ("MakeDir", serde_json::json!({"path":"file"})),
        ("MakeDir", serde_json::json!({"path":"loop"})),
        (
            "Move",
            serde_json::json!({"source":"missing","destination":"partial/dest"}),
        ),
        (
            "Move",
            serde_json::json!({"source":"file","destination":"file/child"}),
        ),
        (
            "Move",
            serde_json::json!({"source":"file","destination":"dir"}),
        ),
        (
            "Move",
            serde_json::json!({"source":"dir","destination":"dir"}),
        ),
        (
            "Move",
            serde_json::json!({"source":"missing","destination":"dir"}),
        ),
        (
            "Move",
            serde_json::json!({"source":"source-dir","destination":"dir"}),
        ),
    ] {
        let mut value = value;
        for (_, path) in value.as_object_mut().unwrap() {
            *path = serde_json::Value::String(format!(
                "{}/{}",
                root.path().display(),
                path.as_str().unwrap()
            ));
        }
        let response = f
            .http
            .handle(request(method, Body::from(value.to_string())))
            .await
            .unwrap();
        assert!(response.status().is_client_error() || response.status().is_server_error());
        let actual: serde_json::Value =
            serde_json::from_slice(&drain(response.into_body()).await).unwrap();
        released(&f).await;
        // Reproduce the original pre-operation fixture before calling Go.
        if root.path().join("partial").exists() {
            std::fs::remove_dir(root.path().join("partial")).unwrap();
        }
        value["Method"] = method.into();
        value["Detailed"] = true.into();
        let mut child = Command::new(&binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(child.stdin.take().unwrap(), "{value}").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(actual, expected, "{method} {value}");
        assert_eq!(
            std::fs::read(root.path().join("file")).unwrap(),
            b"original"
        );
    }
}

#[tokio::test]
#[ignore = "requires SANDPROOF_GUEST_GO_FILES_REFERENCE built from pinned envd"]
async fn original_go_list_dir_errors_match_http() {
    let binary = std::env::var("SANDPROOF_GUEST_GO_FILES_REFERENCE")
        .expect("original Go reference required");
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    prepare_error_tree(root.path());
    std::os::unix::fs::symlink("missing", root.path().join("broken")).unwrap();
    std::os::unix::fs::symlink("file", root.path().join("link-file")).unwrap();
    for path in [
        "missing",
        "file",
        "file/child",
        "loop",
        "broken",
        "link-file",
        "missing/../dir",
        "nul\0",
        "dir/../missing",
    ] {
        let value = serde_json::json!({"path":format!("{}/{path}", root.path().display())});
        compare_failure(&binary, &f, "ListDir", value).await;
    }
    let value = serde_json::json!({"path":format!("/../{}/missing", root.path().display())});
    compare_failure(&binary, &f, "ListDir", value).await;
}

async fn compare_failure(binary: &str, f: &Fixture, method: &str, mut value: serde_json::Value) {
    let response = f
        .http
        .handle(request(method, Body::from(value.to_string())))
        .await
        .unwrap();
    assert!(response.status().is_client_error() || response.status().is_server_error());
    let actual: serde_json::Value =
        serde_json::from_slice(&drain(response.into_body()).await).unwrap();
    released(f).await;
    value["Method"] = method.into();
    value["Detailed"] = true.into();
    let mut child = Command::new(binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{value}").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(actual, expected, "{method} {value}");
}

#[tokio::test]
#[ignore = "requires SANDPROOF_GUEST_GO_FILES_REFERENCE built from pinned envd"]
async fn original_go_remove_errors_match_http() {
    let binary = std::env::var("SANDPROOF_GUEST_GO_FILES_REFERENCE")
        .expect("original Go reference required");
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    prepare_error_tree(root.path());
    for suffix in [
        "dir/.",
        "missing/.",
        "file/child",
        "file/child/deeper",
        "file//child///",
        "loop/child",
        "nul\0",
        "file/<>&\"\n",
    ] {
        let value = serde_json::json!({"path":format!("{}/{suffix}", root.path().display())});
        compare_failure(&binary, &f, "Remove", value).await;
    }
    let value = serde_json::json!({"path":format!("///{}/file/child///", root.path().display())});
    compare_failure(&binary, &f, "Remove", value).await;
    assert_eq!(
        std::fs::read(root.path().join("file")).unwrap(),
        b"original"
    );
    assert!(root.path().join("dir").is_dir());
}
