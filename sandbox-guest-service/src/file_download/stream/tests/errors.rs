use super::negotiation::settings;
use super::*;
use crate::file_download::tests::reference::{Reference, hex};
use crate::file_encoding;
use wire_errno::LinuxErrno;

fn error_request(path: &[u8], head: bool) -> IdentityRequest {
    let mut req = request("");
    req.disposition_path = Some(path.to_vec());
    if head {
        req.method = Method::HEAD;
    }
    req
}
async fn run(
    stage: &str,
    number: i32,
    mut req: IdentityRequest,
    settings: NegotiationLimits,
    guard: Owner,
) -> Result<Download, DownloadError> {
    let (mut source, _) = traced();
    let stat = match stage {
        "missing" => Some(DownloadError::SourceNotFound),
        "directory" => Some(DownloadError::Directory),
        "stat" => Some(DownloadError::StatIo(LinuxErrno(number))),
        _ => None,
    };
    let open = stage == "open";
    if stage == "encoding" || stage == "conditional" {
        req.headers
            .insert("accept-encoding", HeaderValue::from_static("identity;q=0"));
        if stage == "conditional" {
            req.headers.insert(
                "accept-encoding",
                HeaderValue::from_static("gzip,identity;q=0"),
            );
            req.headers
                .insert("range", HeaderValue::from_static("bytes=1-2"));
        }
    }
    if stage == "range" {
        req.headers
            .insert("range", HeaderValue::from_static("bytes=1-2"));
        source.fail_seek = 3;
        source.seek_errno = Some(LinuxErrno(number));
        for (name, value) in [
            ("content-disposition", "inline"),
            ("etag", "tag"),
            ("cache-control", "private"),
            ("content-encoding", "gzip"),
            ("content-length", "999"),
        ] {
            req.response.insert(name, HeaderValue::from_static(value));
        }
    }
    Download::open_negotiated(
        move || {
            if let Some(error) = stat {
                return Err(error);
            }
            Ok((Timestamp::new(1, 0).unwrap(), move || {
                if open {
                    return Err(DownloadError::OpenIo(LinuxErrno(number)));
                }
                Ok(source)
            }))
        },
        req,
        settings,
        Utc,
        deadline(),
        guard,
    )
    .await
}
async fn observed(download: Download) -> String {
    let mut output = vec![download.status().as_u16().to_string()];
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
        output.push(hex(download
            .headers()
            .get(name)
            .map_or(&[], HeaderValue::as_bytes)));
    }
    let (body, error) = drain(download).await;
    assert!(error.is_none());
    output.push(hex(&body));
    output.join("\t")
}

#[tokio::test]
async fn json_keeps_stage_status_escaping_and_original_owner_through_byte_clones() {
    let path = b"/guest/<a&b>\"\\\n\x00";
    for (stage, number, status, vary) in [
        ("missing", 0, 404, false),
        ("directory", 0, 400, false),
        ("stat", 20, 500, false),
        ("stat", 22, 500, false),
        ("stat", 2, 404, false),
        ("open", 2, 500, true),
        ("encoding", 0, 406, false),
        ("conditional", 0, 406, true),
    ] {
        for head in [false, true] {
            let (guard, released) = owner();
            let download = run(stage, number, error_request(path, head), settings(), guard)
                .await
                .unwrap();
            assert_eq!(download.status().as_u16(), status);
            assert_eq!(
                download.headers()["content-type"],
                "application/json; charset=utf-8"
            );
            assert_eq!(download.headers()["x-content-type-options"], "nosniff");
            assert_eq!(download.headers().contains_key("vary"), vary);
            assert!(!download.headers().contains_key("content-disposition"));
            let (body, error) = drain(download).await;
            assert!(error.is_none());
            if head {
                assert!(body.is_empty());
            } else {
                assert!(body.ends_with(b"\n"));
                assert!(body.starts_with(format!("{{\"code\":{status},\"message\":\"").as_bytes()));
                if stage == "stat" && number == 20 {
                    let escaped = r#"/guest/\u003ca\u0026b\u003e\"\\\n\u0000"#;
                    let expected = format!(
                        "{{\"code\":500,\"message\":\"error checking if path exists '{escaped}': stat {escaped}: not a directory\"}}\n"
                    );
                    assert_eq!(body, expected.as_bytes());
                }
            }
            signal(released).await;
        }
    }
    let (guard, mut released) = owner();
    let mut download = run(
        "open",
        13,
        error_request(b"/guest/file", false),
        settings(),
        guard,
    )
    .await
    .unwrap();
    let bytes = Bytes::from_owner(download.next().await.unwrap().unwrap());
    let retained = bytes.clone();
    drop(bytes);
    drop(download);
    assert!(matches!(
        released.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    drop(retained);
    signal(released).await;
}

#[tokio::test]
async fn range_seek_416_clears_only_go_error_headers_and_late_failure_stays_body_error() {
    for head in [false, true] {
        let (guard, released) = owner();
        let download = run(
            "range",
            29,
            error_request(b"/guest/file", head),
            settings(),
            guard,
        )
        .await
        .unwrap();
        assert_eq!(download.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        for name in [
            "last-modified",
            "content-range",
            "accept-ranges",
            "etag",
            "cache-control",
            "content-encoding",
            "content-length",
        ] {
            assert!(!download.headers().contains_key(name), "{name}");
        }
        assert_eq!(
            download.headers()["content-type"],
            "text/plain; charset=utf-8"
        );
        assert_eq!(download.headers()["vary"], "Accept-Encoding");
        assert_eq!(
            download.headers()["content-disposition"],
            "inline; filename=file"
        );
        let (body, error) = drain(download).await;
        assert!(error.is_none());
        assert_eq!(
            body,
            if head {
                vec![]
            } else {
                b"seek /guest/file: illegal seek\n".to_vec()
            }
        );
        signal(released).await;
    }
    let (mut source, _) = traced();
    source.fail_seek = 3;
    source.seek_errno = Some(LinuxErrno(5));
    let (guard, released) = owner();
    let download = Download::open_negotiated(
        move || Ok((Timestamp::new(1, 0).unwrap(), move || Ok(source))),
        request("bytes=0-0,3-3"),
        settings(),
        Utc,
        deadline(),
        guard,
    )
    .await
    .unwrap();
    assert_eq!(download.status(), StatusCode::PARTIAL_CONTENT);
    let (body, error) = drain(download).await;
    assert!(!body.is_empty()); // first part's header precedes its failed seek
    assert!(!body.ends_with(b"--\r\n"));
    assert_eq!(error, Some(DownloadError::RangeSeekIo(LinuxErrno(5))));
    signal(released).await;
}

#[tokio::test]
async fn error_payload_limits_apply_before_allocation_even_for_head_and_unknown_errors_stay_opaque()
{
    let path = b"/guest/\x00\x00\x00\x00\x00\x00\x00\x00";
    let (guard, released) = owner();
    let normal = run("open", 13, error_request(path, false), settings(), guard)
        .await
        .unwrap();
    let length = drain(normal).await.0.len() as u64;
    signal(released).await;
    for head in [false, true] {
        for delta in [0, 1] {
            let (guard, released) = owner();
            let mut small = settings();
            small.identity.plan.wire_bytes = length - delta;
            let result = run("open", 13, error_request(path, head), small, guard).await;
            if delta == 0 {
                assert!(drain(result.unwrap()).await.1.is_none());
            } else {
                assert!(matches!(
                    result,
                    Err(DownloadError::Plan(Error::ResponseLimit))
                ));
            }
            signal(released).await;
        }
    }
    for fail in [
        DownloadError::Stat,
        DownloadError::Open,
        DownloadError::SourceInvalid,
        DownloadError::Negotiation(file_encoding::DownloadError {
            cause: file_encoding::Error::HeaderLimit,
            vary_accept_encoding: false,
        }),
    ] {
        let (guard, released) = owner();
        let result = Download::open_negotiated(
            move || Err::<(Timestamp, fn() -> Result<Cursor<Vec<u8>>, DownloadError>), _>(fail),
            error_request(b"/guest/file", false),
            settings(),
            Utc,
            deadline(),
            guard,
        )
        .await;
        assert!(matches!(result, Err(e) if e == fail));
        signal(released).await;
    }
}

#[tokio::test]
#[ignore = "requires explicit fixed Go reference binary"]
async fn fixed_go_error_envelopes_and_range_seek_match_headers_and_bytes() {
    let mut oracle = Reference::new();
    let paths: &[&[u8]] = &[
        b"/guest/file",
        b"",
        b"/guest/<a&b>\"\\\n\r\t\x00\x08\x0c\x1f",
        "/guest/中文\u{2028}\u{2029}".as_bytes(),
        b"/guest/\xff\xc0\xed\xa0\x80",
    ];
    let numbers: Vec<i32> = (-1..=140).chain([i32::MIN, i32::MAX]).collect();
    let mut count = 0;
    for stage in [
        "missing",
        "directory",
        "stat",
        "open",
        "encoding",
        "conditional",
        "range",
    ] {
        let tested: &[i32] = if matches!(stage, "stat" | "open" | "range") {
            &numbers
        } else {
            &[0]
        };
        for &number in tested {
            for &path in paths {
                for head in [false, true] {
                    let method = if head { "HEAD" } else { "GET" };
                    let (guard, released) = owner();
                    let download = run(stage, number, error_request(path, head), settings(), guard)
                        .await
                        .unwrap();
                    let input = if stage == "range" {
                        format!("R\t{number}\t{}\t{method}", hex(path))
                    } else {
                        format!("J\t{stage}\t{number}\t{}\t{method}", hex(path))
                    };
                    oracle.check(&input, &observed(download).await);
                    signal(released).await;
                    count += 1;
                }
            }
        }
    }
    oracle.finish();
    eprintln!(
        "Fixed Go error comparison: {count} envelopes/range seek cases (Linux errno table; portable, no kernel calls)."
    );
}
