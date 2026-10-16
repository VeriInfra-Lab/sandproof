use super::*;

const ARGS: [&str; 3] = ["a b", "", "\n\u{2028}"];
const CWD: &str = "/original-'\"<>&中";

fn pty_source(stage: PtyStage, errno: i32, gzip: bool) -> (Source, Arc<Semaphore>) {
    let (mut source, slots) = source(CWD, gzip);
    source.rejection = Some(Rejection::Launch(LaunchFailure::new(
        native::request("'<>&中", &ARGS.map(String::from), true, 65537, u32::MAX),
        CWD.into(),
        LaunchFailureKind::Pty { stage, errno },
    )));
    (source, slots)
}

#[tokio::test]
async fn preparation_errors_keep_original_stage_raw_size_budget_and_last_byte_owner() {
    for stage in [PtyStage::OpenMaster, PtyStage::Unlock, PtyStage::Resize] {
        for (errno, code, text) in [
            (24, "resource_exhausted", "too many open files"),
            (28, "resource_exhausted", "no space left on device"),
            (25, "invalid_argument", "inappropriate ioctl for device"),
        ] {
            for gzip in [false, true] {
                let (source, _) = pty_source(stage, errno, gzip);
                let bytes = drain(send(source, limits()).await.unwrap().into_body()).await;
                let value: serde_json::Value = serde_json::from_slice(&decoded(&bytes)).unwrap();
                let suffix = if matches!(stage, PtyStage::OpenMaster) {
                    "open /dev/ptmx: "
                } else {
                    ""
                };
                assert_eq!(value["error"]["code"], code);
                assert_eq!(
                    value["error"]["message"],
                    format!(
                        "error starting pty with command ''<>&中 a b  \n\u{2028}' in dir '{CWD}' with '65537' cols and '4294967295' rows: {suffix}{text}"
                    )
                );
                let required = bytes.len().max(decoded(&bytes).len() + 5);
                for limit in [required - 1, required] {
                    let (source, slots) = pty_source(stage, errno, gzip);
                    let result = send(
                        source,
                        reply::Limits {
                            wire_bytes: limit,
                            ..limits()
                        },
                    )
                    .await;
                    if limit < required {
                        let failure = result.unwrap_err();
                        assert_eq!(failure.error, reply::Error::ResponseLimit);
                        assert_eq!(slots.available_permits(), 0);
                        drop(failure);
                    } else {
                        let mut body = result.unwrap().into_body();
                        let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
                        let last = bytes.clone();
                        drop(bytes);
                        drop(body);
                        assert_eq!(slots.available_permits(), 0);
                        drop(last);
                    }
                    released(slots).await;
                }
            }
        }
    }
    for absent in [false, true] {
        let (mut source, slots) = pty_source(PtyStage::OpenMaster, 24, false);
        if absent {
            source.rejection = None;
        } else {
            source.deadline = Instant::now();
        }
        let failure = send(source, limits()).await.unwrap_err();
        assert_eq!(
            failure.error,
            if absent {
                reply::Error::UnclassifiedFault
            } else {
                reply::Error::DeadlineExceeded
            }
        );
        assert_eq!(slots.available_permits(), 0);
        drop(failure);
        released(slots).await;
    }
}

#[test]
fn preparation_fact_requires_pty_and_positive_errno() {
    for stage in [PtyStage::OpenMaster, PtyStage::Unlock, PtyStage::Resize] {
        for (pty, errno) in [(false, 24), (true, 0), (true, -1)] {
            let failure = LaunchFailure::new(
                native::request("command", &[], pty, 1, 1),
                CWD.into(),
                LaunchFailureKind::Pty { stage, errno },
            );
            assert_eq!(
                failure.payload(4096).unwrap_err(),
                reply::Error::UnclassifiedFault
            );
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn preparation_fact_does_not_relabel_exec_or_cancellation() {
    use crate::command::{CommandError, RegistrationError};
    for stage in [PtyStage::OpenMaster, PtyStage::Unlock, PtyStage::Resize] {
        let fact = pty_source(stage, 24, false).0.rejection.unwrap();
        assert!(
            fact.matches(&RequestError::Registration(RegistrationError::Preparation(
                CommandError::Unavailable
            )))
        );
        for error in [
            RequestError::Unavailable,
            RequestError::DeadlineExceeded,
            RequestError::Command(CommandError::ExecFailed(24)),
            RequestError::Registration(RegistrationError::Preparation(CommandError::Cancelled)),
            RequestError::Registration(RegistrationError::Preparation(
                CommandError::DeadlineExceeded,
            )),
            RequestError::Registration(RegistrationError::Preparation(
                CommandError::InvalidArgument,
            )),
        ] {
            assert!(!fact.matches(&error));
        }
    }
}

#[tokio::test]
#[ignore = "explicit fixed Go PTY preparation rendering; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_pty_preparation_errors_match_complete_replies() {
    use crate::file_download::tests::reference::{Reference, hex};
    use axum::http::HeaderValue;
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    for (stage, name) in [
        (PtyStage::OpenMaster, "open-master"),
        (PtyStage::Unlock, "unlock"),
        (PtyStage::Resize, "resize"),
    ] {
        for (errno, native) in [
            (24, "EMFILE"),
            (28, "ENOSPC"),
            (11, "EAGAIN"),
            (22, "EINVAL"),
            (25, "ENOTTY"),
        ] {
            for codec in [JsonContentType::Json, JsonContentType::Utf8] {
                for gzip in [false, true] {
                    let (mut source, slots) = pty_source(stage, errno, gzip);
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
                    let input = serde_json::json!({"Method":"Start", "Kind":"pty-prepare", "Pty":true,
                        "Stage":name, "Native":native, "Cmd":"'<>&中", "Args":ARGS, "Cwd":CWD,
                        "Cols":65537, "Rows":u32::MAX, "Gzip":gzip, "UTF8":codec == JsonContentType::Utf8});
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
    oracle.finish();
    eprintln!(
        "{count} fixed Go PTY preparation renderings; original terminal syscalls not executed by this reference."
    );
}
