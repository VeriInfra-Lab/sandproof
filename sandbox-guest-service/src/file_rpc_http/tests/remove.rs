use super::*;

#[tokio::test]
async fn remove_errors_keep_observations_and_owned_response_then_recover() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    prepare_error_tree(root.path());
    for (path, detail) in [
        ("dir/.", "RemoveAll ROOT/dir/.: invalid argument"),
        ("file/child", "unlinkat ROOT/file/child: not a directory"),
        ("file/child/deeper", "open ROOT/file/child: not a directory"),
        ("file//child///", "open ROOT/file/: not a directory"),
        (
            "loop/child",
            "open ROOT/loop: too many levels of symbolic links",
        ),
        ("nul\0", "unlinkat ROOT/nul\0: invalid argument"),
    ] {
        let value = serde_json::json!({"path":format!("{}/{path}", root.path().display())});
        let response = f
            .http
            .handle(request("Remove", Body::from(value.to_string())))
            .await
            .unwrap();
        assert_eq!(response.status(), 500);
        full(&f);
        file_full(&f).await;
        let value: serde_json::Value =
            serde_json::from_slice(&drain(response.into_body()).await).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "code":"internal",
                "message":format!("error removing file or directory: {}", detail.replace("ROOT", root.path().to_str().unwrap()))
            })
        );
        released(&f).await;
        assert_eq!(
            std::fs::read(root.path().join("file")).unwrap(),
            b"original"
        );
        assert!(root.path().join("dir").exists());
    }
    for path in ["missing", "dir"] {
        let response = f
            .http
            .handle(request(
                "Remove",
                Body::from(serde_json::json!({"path":path}).to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(drain(response.into_body()).await, b"{}");
        released(&f).await;
    }
    assert!(!root.path().join("dir").exists());
}
