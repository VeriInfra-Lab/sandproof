use super::*;
use crate::file_encoding;
use sandproof_guest_runtime::gzip::{DecodeLimits, Decoder};

fn settings() -> NegotiationLimits {
    NegotiationLimits {
        identity: limits(),
        encoding: file_encoding::Limits {
            header_bytes: 8192,
            items: 64,
        },
        compressed_bytes: 8192,
    }
}
async fn negotiate(
    source: Traced,
    mut req: IdentityRequest,
    accepted: &'static str,
    stat_error: Option<DownloadError>,
    open_error: bool,
    guard: Owner,
) -> Result<Download, DownloadError> {
    req.headers
        .insert("accept-encoding", HeaderValue::from_static(accepted));
    let log = Arc::clone(&source.log);
    Download::open_negotiated(
        move || {
            log.lock().unwrap().push(Event::Stat);
            if let Some(error) = stat_error {
                return Err(error);
            }
            Ok((Timestamp::new(1, 0).unwrap(), move || {
                log.lock().unwrap().push(Event::Open);
                if open_error {
                    return Err(DownloadError::Open);
                }
                Ok(source)
            }))
        },
        req,
        settings(),
        Utc,
        deadline(),
        guard,
    )
    .await
}
fn uncompress(bytes: Vec<u8>) -> Vec<u8> {
    let mut decoder = Decoder::new(
        Cursor::new(bytes),
        DecodeLimits {
            chunk_bytes: 3,
            encoded_bytes: 8192,
            decoded_bytes: 8192,
            header_bytes: 128,
            members: 2,
        },
    )
    .unwrap();
    let mut out = Vec::new();
    while let Some(bytes) = decoder.next_chunk().unwrap() {
        out.extend(bytes);
    }
    out
}
#[tokio::test]
async fn stat_and_directory_errors_win_over_encoding_which_wins_over_open() {
    for failure in [
        DownloadError::SourceNotFound,
        DownloadError::SourceInvalid,
        DownloadError::Stat,
    ] {
        let (source, log) = traced();
        let (guard, released) = owner();
        assert!(
            matches!(negotiate(source, request(""), "identity;q=0", Some(failure), true, guard).await, Err(e) if e==failure)
        );
        signal(released).await;
        assert_eq!(*log.lock().unwrap(), [Event::Stat, Event::Drop]);
    }
    for (accepted, range, vary) in [
        ("identity;q=0", "", false),
        ("identity;q=0,gzip", "bytes=0-1", true),
    ] {
        let (source, log) = traced();
        let (guard, released) = owner();
        assert!(
            matches!(negotiate(source, request(range), accepted, None, true, guard).await,
            Err(DownloadError::Negotiation(e)) if e.vary_accept_encoding==vary)
        );
        signal(released).await;
        assert_eq!(*log.lock().unwrap(), [Event::Stat, Event::Drop]);
    }
    let (source, log) = traced();
    let (guard, released) = owner();
    assert!(matches!(
        negotiate(source, request(""), "gzip", None, true, guard).await,
        Err(DownloadError::Open)
    ));
    signal(released).await;
    assert_eq!(
        *log.lock().unwrap(),
        [Event::Stat, Event::Open, Event::Drop]
    );
}
#[tokio::test]
async fn selected_gzip_never_sniffs_and_only_four_fields_force_identity() {
    for field in [
        None,
        Some("if-match"),
        Some("if-unmodified-since"),
        Some("range"),
        Some("if-modified-since"),
        Some("if-none-match"),
        Some("if-range"),
    ] {
        let (source, log) = traced();
        let (guard, released) = owner();
        let mut req = automatic(request(""), "unknown");
        if let Some(field) = field {
            req.headers
                .insert(field, HeaderValue::from_static("nonsense"));
        }
        let compressed = matches!(field, None | Some("if-match" | "if-unmodified-since"));
        let download = negotiate(source, req, "GZIP", None, false, guard)
            .await
            .unwrap();
        assert_eq!(download.headers()["vary"], "Accept-Encoding");
        assert_eq!(
            download.headers().contains_key("content-encoding"),
            compressed
        );
        if compressed {
            assert_eq!(*log.lock().unwrap(), [Event::Stat, Event::Open]);
            assert_eq!(
                download.headers()["content-type"],
                "application/octet-stream"
            );
            assert!(!download.headers().contains_key("content-length"));
        }
        let (body, error) = drain(download).await;
        assert!(error.is_none());
        if compressed {
            assert_eq!(uncompress(body), b"01234");
        }
        signal(released).await;
    }
}
#[tokio::test]
async fn conditional_early_reply_and_gzip_head_keep_original_execution_rules() {
    let (source, log) = traced();
    let (guard, released) = owner();
    let mut req = automatic(request(""), "unknown");
    req.headers
        .insert("if-none-match", HeaderValue::from_static("*"));
    let download = negotiate(source, req, "gzip", None, false, guard)
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(
        *log.lock().unwrap(),
        [Event::Stat, Event::Open, Event::Drop]
    );
    drop(download);
    signal(released).await;
    let (source, log) = traced();
    let (guard, released) = owner();
    let mut req = automatic(request(""), "original.css");
    req.method = Method::HEAD;
    let download = negotiate(source, req, "gzip", None, false, guard)
        .await
        .unwrap();
    assert_eq!(
        download.headers()["content-type"],
        "text/css; charset=utf-8"
    );
    assert!(log.lock().unwrap().contains(&Event::Read));
    assert!(matches!(log.lock().unwrap().last(), Some(Event::Drop)));
    assert!(
        !log.lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, Event::Seek(_)))
    );
    assert_eq!(drain(download).await, (vec![], None));
    signal(released).await;
}
#[tokio::test]
async fn cancellation_during_inspect_or_open_retains_one_guard_until_call_returns() {
    for inspect in [true, false] {
        let (entered, started) = oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let (guard, mut released) = owner();
        let task = tokio::spawn(Download::open_negotiated(
            move || {
                let mut gate = Some((entered, wait));
                if inspect {
                    let (entered, wait) = gate.take().unwrap();
                    entered.send(()).unwrap();
                    wait.recv().unwrap();
                }
                Ok((Timestamp::new(1, 0).unwrap(), move || {
                    if let Some((entered, wait)) = gate {
                        entered.send(()).unwrap();
                        wait.recv().unwrap();
                    }
                    Ok(Cursor::new(b"data".to_vec()))
                }))
            },
            request(""),
            settings(),
            Utc,
            deadline(),
            guard,
        ));
        signal(started).await;
        task.abort();
        assert!(task.await.is_err());
        assert!(matches!(
            released.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        release.send(()).unwrap();
        signal(released).await;
    }
}

#[tokio::test]
#[ignore = "explicit Go 1.26.8 seek failure reference; set SANDPROOF_GUEST_DOWNLOAD_REFERENCE"]
async fn fixed_go_preparation_seek_errors_keep_exact_headers_and_public_text() {
    use crate::file_download::tests::reference::{Reference, hex};
    let mut oracle = Reference::new();
    for known in [true, false] {
        for failed in 1..=if known { 2 } else { 3 } {
            for method in [Method::GET, Method::HEAD] {
                let (mut source, _) = traced();
                source.fail_seek = failed;
                let (guard, released) = owner();
                let mut req = if known {
                    request("")
                } else {
                    automatic(request(""), "unknown")
                };
                req.disposition_path = None;
                req.method = method.clone();
                for (name, value) in [
                    ("content-disposition", "inline"),
                    ("etag", "tag"),
                    ("cache-control", "private"),
                    ("content-encoding", "gzip"),
                    ("content-length", "999"),
                ] {
                    req.response.insert(name, HeaderValue::from_static(value));
                }
                let download = negotiate(source, req, "identity", None, false, guard)
                    .await
                    .unwrap();
                assert_eq!(download.status(), StatusCode::INTERNAL_SERVER_ERROR);
                let mut observed = vec![download.status().as_u16().to_string()];
                for name in [
                    "last-modified",
                    "content-range",
                    "accept-ranges",
                    "content-type",
                    "content-length",
                    "etag",
                    "cache-control",
                    "content-encoding",
                    "x-content-type-options",
                    "vary",
                    "content-disposition",
                ] {
                    observed.push(hex(download
                        .headers()
                        .get(name)
                        .map_or(&[], HeaderValue::as_bytes)));
                }
                let (body, error) = drain(download).await;
                assert!(error.is_none());
                observed.push(hex(&body));
                oracle.check(
                    &format!(
                        "F\t{failed}\t{}\t{method}",
                        if known { "known" } else { "sniff" }
                    ),
                    &observed.join("\t"),
                );
                signal(released).await;
            }
        }
    }
    oracle.finish();
    eprintln!(
        "Fixed Go preparation errors: 10 sniff/length seek status, 11 headers and body cases."
    );
}

#[tokio::test]
async fn failed_preparation_seek_returns_bounded_anonymous_500_not_body_error() {
    for head in [false, true] {
        let (mut source, log) = traced();
        source.fail_seek = 1;
        let (guard, released) = owner();
        let mut req = automatic(request(""), "unknown");
        if head {
            req.method = Method::HEAD;
        }
        req.response
            .insert("etag", HeaderValue::from_static("private"));
        let download = negotiate(source, req, "identity", None, false, guard)
            .await
            .unwrap();
        assert_eq!(download.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(download.headers()["x-content-type-options"], "nosniff");
        assert_eq!(download.headers()["vary"], "Accept-Encoding");
        assert!(download.headers().contains_key("content-disposition"));
        assert!(!download.headers().contains_key("etag"));
        assert!(!download.headers().contains_key("last-modified"));
        assert!(matches!(log.lock().unwrap().last(), Some(Event::Drop)));
        assert_eq!(
            drain(download).await,
            (
                if head {
                    vec![]
                } else {
                    b"seeker can't seek\n".to_vec()
                },
                None
            )
        );
        signal(released).await;
    }
}
