use super::*;

fn pipe_source(stream: PipeStream, errno: i32, gzip: bool) -> (Source, Arc<Semaphore>) {
    let (mut source, slots) = source("/not-public", gzip);
    source.rejection = Some(Rejection::Launch(LaunchFailure::new(
        native::request(
            "'<>&中",
            &["a b".into(), String::new(), "\n\u{2028}".into()],
            false,
            0,
            0,
        ),
        "/not-public".into(),
        LaunchFailureKind::Pipe { stream, errno },
    )));
    (source, slots)
}

#[tokio::test]
async fn pipe_errors_keep_internal_class_exact_budget_deadline_and_last_byte_owner() {
    for gzip in [false, true] {
        let (source, _) = pipe_source(PipeStream::Stderr, 24, gzip);
        let bytes = drain(send(source, limits()).await.unwrap().into_body()).await;
        let value: serde_json::Value = serde_json::from_slice(&decoded(&bytes)).unwrap();
        assert_eq!(value["error"]["code"], "internal");
        assert_eq!(
            value["error"]["message"],
            "error creating stderr pipe for command ''<>&中 a b  \n\u{2028}': pipe2: too many open files"
        );
        let required = bytes.len().max(decoded(&bytes).len() + 5);
        let (source, slots) = pipe_source(PipeStream::Stderr, 24, gzip);
        let failure = send(
            source,
            reply::Limits {
                wire_bytes: required - 1,
                ..limits()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(failure.error, reply::Error::ResponseLimit);
        assert_eq!(slots.available_permits(), 0);
        drop(failure);
        released(slots).await;
        let (source, slots) = pipe_source(PipeStream::Stderr, 24, gzip);
        let response = send(
            source,
            reply::Limits {
                wire_bytes: required,
                ..limits()
            },
        )
        .await
        .unwrap();
        let mut body = response.into_body();
        let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
        let last = bytes.clone();
        drop(bytes);
        drop(body);
        assert_eq!(slots.available_permits(), 0);
        drop(last);
        released(slots).await;
    }
    for absent in [false, true] {
        let (mut source, slots) = pipe_source(PipeStream::Stdin, 24, false);
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
fn invalid_pipe_observations_cannot_be_formatted_as_a_known_native_failure() {
    for (pty, errno) in [(true, 24), (false, 0), (false, -1)] {
        let failure = LaunchFailure::new(
            native::request("command", &[], pty, 1, 1),
            String::new(),
            LaunchFailureKind::Pipe {
                stream: PipeStream::Stdout,
                errno,
            },
        );
        assert_eq!(
            failure.payload(4096).unwrap_err(),
            reply::Error::UnclassifiedFault
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn pipe_fact_cannot_relabel_launch_cancellation_or_another_registration_error() {
    use crate::command::{CommandError, RegistrationError};
    let fact = pipe_source(PipeStream::Stdout, 24, false)
        .0
        .rejection
        .unwrap();
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

#[tokio::test]
#[ignore = "explicit fixed Go pipe-error rendering; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_pipe_failures_match_complete_replies_and_keep_internal_resource_errors() {
    use crate::file_download::tests::reference::{Reference, hex};
    use axum::http::HeaderValue;
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    for (stream, name) in [
        (PipeStream::Stdout, "stdout"),
        (PipeStream::Stderr, "stderr"),
        (PipeStream::Stdin, "stdin"),
    ] {
        for (errno, native) in [
            (24, "EMFILE"),
            (23, "ENFILE"),
            (12, "ENOMEM"),
            (22, "EINVAL"),
        ] {
            for codec in [JsonContentType::Json, JsonContentType::Utf8] {
                for gzip in [false, true] {
                    let (mut source, slots) = pipe_source(stream, errno, gzip);
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
                    let input = serde_json::json!({"Method":"Start", "Kind":"pipe", "Cmd":"'<>&中", "Args":["a b", "", "\n\u{2028}"], "Pipe":name, "Native":native, "Gzip":gzip, "UTF8":codec == JsonContentType::Utf8});
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
        "{count} fixed Go pipe-failure renderings; original pipe syscalls not executed by this reference."
    );
}
