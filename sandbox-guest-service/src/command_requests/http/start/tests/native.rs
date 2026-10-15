use super::*;
use sandproof_guest_runtime::protocol::{ProcessConfig, Pty, PtySize, StartRequest};

fn request(program: &str, args: &[String], pty: bool, cols: u32, rows: u32) -> StartRequest {
    StartRequest {
        process: Some(ProcessConfig {
            cmd: program.into(),
            args: args.to_vec(),
            ..ProcessConfig::default()
        }),
        pty: pty.then_some(Pty {
            size: Some(PtySize { cols, rows }),
        }),
        tag: None,
        stdin: None,
    }
}
fn native_source(pty: bool, errno: i32, gzip: bool) -> (Source, Arc<Semaphore>) {
    let (mut source, slots) = source("/private-secret", gzip);
    source.rejection = Some(Rejection::Native(NativeFailure::new(
        request(
            "/bin/sh",
            &["a b".into(), String::new(), "'\"\\\n<>&中".into()],
            pty,
            65537,
            u32::MAX,
        ),
        "/private-secret".into(),
        errno,
    )));
    (source, slots)
}

#[tokio::test]
async fn native_replies_keep_original_argv_raw_pty_size_and_last_byte_owner() {
    for pty in [false, true] {
        for (errno, code) in [(20, "invalid_argument"), (11, "resource_exhausted")] {
            let (source, slots) = native_source(pty, errno, false);
            let response = send(source, limits()).await.unwrap();
            let mut body = response.into_body();
            let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
            let value: serde_json::Value = serde_json::from_slice(&decoded(&first)).unwrap();
            assert_eq!(value["error"]["code"], code);
            let message = value["error"]["message"].as_str().unwrap();
            assert!(message.contains("/bin/sh a b  '\"\\\n<>&中"));
            assert_eq!(
                message.contains("with '65537' cols and '4294967295' rows"),
                pty
            );
            let last = first.clone();
            drop(first);
            drop(body);
            assert_eq!(slots.available_permits(), 0);
            drop(last);
            released(slots).await;
        }
    }
}

#[tokio::test]
async fn native_reply_exact_budget_expiry_and_missing_observation_never_drop_request_credit() {
    for gzip in [false, true] {
        let (initial, _) = native_source(true, 24, gzip);
        let bytes = drain(send(initial, limits()).await.unwrap().into_body()).await;
        let required = bytes.len().max(decoded(&bytes).len() + 5);
        for wire_bytes in [required - 1, required] {
            let (source, slots) = native_source(true, 24, gzip);
            let result = send(
                source,
                reply::Limits {
                    wire_bytes,
                    ..limits()
                },
            )
            .await;
            if wire_bytes == required {
                assert_eq!(
                    decoded(&drain(result.unwrap().into_body()).await),
                    decoded(&bytes)
                );
            } else {
                let failure = result.unwrap_err();
                assert_eq!(failure.error, reply::Error::ResponseLimit);
                assert!(!format!("{failure:?}").contains("private-secret"));
                assert_eq!(slots.available_permits(), 0);
                drop(failure);
            }
            released(slots).await;
        }
    }
    for expired in [false, true] {
        let (mut source, slots) = native_source(false, 20, false);
        if expired {
            source.deadline = Instant::now();
        } else {
            source.rejection = None;
        }
        let failure = send(source, limits()).await.unwrap_err();
        assert_eq!(
            failure.error,
            if expired {
                reply::Error::DeadlineExceeded
            } else {
                reply::Error::UnclassifiedFault
            }
        );
        assert_eq!(slots.available_permits(), 0);
        drop(failure);
        released(slots).await;
    }
}

#[tokio::test]
#[ignore = "explicit fixed Go native-start rendering; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_native_start_errors_match_symbolic_errno_and_full_replies() {
    use crate::file_download::tests::reference::{Reference, hex};
    use axum::http::HeaderValue;
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    for (number, name) in [
        (1, "EPERM"),
        (2, "ENOENT"),
        (5, "EIO"),
        (7, "E2BIG"),
        (8, "ENOEXEC"),
        (9, "EBADF"),
        (11, "EAGAIN"),
        (12, "ENOMEM"),
        (13, "EACCES"),
        (20, "ENOTDIR"),
        (22, "EINVAL"),
        (23, "ENFILE"),
        (24, "EMFILE"),
        (28, "ENOSPC"),
        (36, "ENAMETOOLONG"),
        (40, "ELOOP"),
        (i32::MAX, "UNKNOWN"),
    ] {
        for pty in [false, true] {
            for (program, args, cols, rows) in [
                ("", vec![], 0, 0),
                (
                    "'\"\\<>&中",
                    vec!["a b".into(), String::new(), "\n\u{2028}".into()],
                    65537,
                    u32::MAX,
                ),
                ("program", vec!["x".repeat(2100)], 80, 24),
            ] {
                for codec in [JsonContentType::Json, JsonContentType::Utf8] {
                    for gzip in [false, true] {
                        let (mut source, slots) = source("/path'\"\\\n<>&中", gzip);
                        let Some(Rejection::MissingCwd(cwd)) = source.rejection.take() else {
                            panic!("cwd fixture");
                        };
                        let input = serde_json::json!({"Method":"Start","Kind":"native","Cmd":program,"Args":args,"Cwd":cwd,"Native":name,"Pty":pty,"Cols":cols,"Rows":rows,"Gzip":gzip,"UTF8":codec==JsonContentType::Utf8});
                        source.rejection = Some(Rejection::Native(NativeFailure::new(
                            request(program, &args, pty, cols, rows),
                            cwd,
                            number,
                        )));
                        source.format = Some(ResponseFormat::new(
                            Rpc::Start,
                            codec,
                            if gzip {
                                Encoding::Gzip
                            } else {
                                Encoding::Identity
                            },
                        ));
                        let response = send(source, limits()).await.unwrap();
                        let mut observed = vec![response.status().as_u16().to_string()];
                        for name in [
                            "content-type",
                            "content-encoding",
                            "connect-content-encoding",
                            "accept-encoding",
                            "connect-accept-encoding",
                            "content-length",
                            "allow",
                        ] {
                            observed.push(hex(response
                                .headers()
                                .get(name)
                                .map_or(b"", HeaderValue::as_bytes)));
                        }
                        let bytes = drain(response.into_body()).await;
                        observed.push(bytes[0].to_string());
                        observed.push(hex(&decoded(&bytes)));
                        oracle.check(
                            &format!("R\t{}", hex(&serde_json::to_vec(&input).unwrap())),
                            &observed.join("\t"),
                        );
                        released(slots).await;
                        count += 1;
                    }
                }
            }
        }
    }
    oracle.finish();
    eprintln!(
        "{count} fixed Go native-start renderings using symbolic errno; no actual original spawn."
    );
}
