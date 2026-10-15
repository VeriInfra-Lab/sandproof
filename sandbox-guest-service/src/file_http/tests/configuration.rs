use super::*;
type Change = (&'static str, fn(&mut Settings<Utc>));

#[tokio::test]
async fn invalid_dormant_modes_are_rejected_before_accounts_or_file_effects() {
    let root = tempfile::tempdir().unwrap();
    let fixture = fixture(root.path()).await;
    let inner = &fixture.http.inner;
    let changes: &[Change] = &[
        ("relative daemon cwd", |s| {
            s.files.daemon_workdir = "relative".into();
        }),
        ("NUL daemon cwd", |s| s.files.daemon_workdir.push('\0')),
        ("file path budget", |s| s.files.limits.path_bytes = 0),
        ("upload headers", |s| s.upload.headers.fields = 0),
        ("upload encoding", |s| s.upload.encoding.items = usize::MAX),
        ("gzip input frames", |s| s.upload.gzip.http.frames = 0),
        ("gzip first header", |s| {
            s.upload.gzip.decode.header_bytes = 9;
        }),
        ("gzip output allocation", |s| {
            s.upload.gzip.decode.chunk_bytes = usize::MAX;
        }),
        ("multipart transport", |s| {
            s.upload.multipart.trailer_fields = 0;
        }),
        ("multipart decoder", |s| s.upload.multipart.wire.parts = 0),
        ("multipart read lookahead", |s| {
            s.upload.multipart.wire.chunk_bytes = isize::MAX as usize / 16;
            s.upload.multipart.wire.header_bytes = isize::MAX as usize / 16;
        }),
        ("batch results", |s| s.upload.batch.retained_bytes = 0),
        ("upload response", |s| s.upload_response.chunk_bytes = 0),
        ("download request", |s| {
            s.download.response.request.items = 0;
        }),
        ("download response", |s| {
            s.download.response.response_header_bytes = 0;
        }),
        ("download encoding", |s| {
            s.download.encoding.header_bytes = 0;
        }),
    ];
    for (name, change) in changes {
        let mut settings = settings(root.path());
        change(&mut settings);
        assert!(
            matches!(
                FileHttp::new(
                    inner.session.clone(),
                    inner.requests.clone(),
                    inner.streams.clone(),
                    inner.accounts.clone(),
                    settings
                ),
                Err(InvalidConfiguration)
            ),
            "{name}"
        );
        drop(inner.requests.admit().unwrap());
        assert_eq!(inner.streams.available_slots(), 1, "{name}");
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 0, "{name}");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0, "{name}");
    }

    // Existing zero-byte quotas are request limits, not invalid configuration.
    // A missing absolute cwd is also resolved by requests, not guessed at startup.
    let mut settings = settings(root.path());
    settings.files.daemon_workdir.push_str("/not-created");
    settings.upload.gzip.http.total_bytes = 0;
    settings.upload.gzip.decode.encoded_bytes = 0;
    settings.upload.gzip.decode.decoded_bytes = 0;
    settings.download.response.wire_bytes = 0;
    settings.download.compressed_bytes = 0;
    settings.upload_response.wire_bytes = 1;
    let accepted = FileHttp::new(
        inner.session.clone(),
        inner.requests.clone(),
        inner.streams.clone(),
        inner.accounts.clone(),
        settings,
    )
    .unwrap();
    assert!(accepted.shares_session(&inner.session));
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
