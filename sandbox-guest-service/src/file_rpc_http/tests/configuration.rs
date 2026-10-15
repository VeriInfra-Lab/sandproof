use super::*;

#[tokio::test]
async fn invalid_context_and_response_configuration_never_reaches_file_work() {
    let root = tempfile::tempdir().unwrap();
    let fixture = fixture(root.path(), 65536).await;
    let inner = &fixture.http.inner;
    for case in 0..8 {
        let mut settings = inner.settings.clone();
        let mut response = inner.response;
        match case {
            0 => settings.daemon_workdir = "relative".into(),
            1 => settings.daemon_workdir.push('\0'),
            2 => settings.limits.authorization_bytes = 0,
            3 => settings.limits.path_bytes = 0,
            4 => response.wire_bytes = 0,
            5 => response.chunk_bytes = 0,
            6 => response.wire_bytes = usize::MAX,
            7 => response.chunk_bytes = usize::MAX,
            _ => unreachable!(),
        }
        assert!(
            matches!(
                FileRpcHttp::new(
                    inner.session.clone(),
                    inner.requests.clone(),
                    inner.files.clone(),
                    inner.accounts.clone(),
                    settings,
                    response
                ),
                Err(InvalidConfiguration)
            ),
            "case={case}"
        );
        drop(inner.requests.admit().unwrap());
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
    // A tiny but valid response quota remains a per-request encoding limit.
    // Do not require that every possible reply fit at construction time.
    let accepted = FileRpcHttp::new(
        inner.session.clone(),
        inner.requests.clone(),
        inner.files.clone(),
        inner.accounts.clone(),
        inner.settings.clone(),
        file_rpc_response::Limits {
            wire_bytes: 1,
            chunk_bytes: 1,
        },
    )
    .unwrap();
    assert!(accepted.shares_session(&inner.session));
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
