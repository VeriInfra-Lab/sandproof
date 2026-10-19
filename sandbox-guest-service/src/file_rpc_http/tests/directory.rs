use super::*;

#[tokio::test]
async fn directory_failures_use_observed_paths_and_original_owned_response() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    prepare_error_tree(root.path());
    std::os::unix::fs::symlink("missing", root.path().join("broken")).unwrap();
    std::os::unix::fs::symlink("file", root.path().join("link-file")).unwrap();
    for (path, status, message) in [
        (
            "missing",
            404,
            "path not found: lstat ROOT/missing: no such file or directory",
        ),
        (
            "missing/../dir",
            404,
            "path not found: lstat ROOT/missing: no such file or directory",
        ),
        (
            "broken",
            404,
            "path not found: lstat ROOT/missing: no such file or directory",
        ),
        ("file", 400, "path is not a directory: ROOT/file"),
        ("link-file", 400, "path is not a directory: ROOT/file"),
        (
            "file/child",
            500,
            "error resolving symlink: not a directory",
        ),
        (
            "loop",
            400,
            "cyclic symlink or chain >255 links at \"ROOT/loop\"",
        ),
    ] {
        let value = serde_json::json!({"path":format!("{}/{path}", root.path().display())});
        let response = f
            .http
            .handle(request("ListDir", Body::from(value.to_string())))
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        full(&f);
        file_full(&f).await;
        let value: serde_json::Value =
            serde_json::from_slice(&drain(response.into_body()).await).unwrap();
        assert_eq!(
            value["message"],
            message.replace("ROOT", root.path().to_str().unwrap())
        );
        released(&f).await;
    }
    let response = f
        .http
        .handle(request("ListDir", Body::from(r#"{"path":"dir"}"#)))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    drain(response.into_body()).await;
    released(&f).await;
}
