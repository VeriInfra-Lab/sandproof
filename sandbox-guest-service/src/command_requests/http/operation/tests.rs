use super::super::{Encoding, Rpc};
use super::*;
use crate::file_download::tests::reference::{Reference, hex};
use http_body_util::BodyExt;
use sandproof_guest_runtime::gzip::{DecodeLimits, Decoder};
use sandproof_guest_runtime::protocol::{JsonContentType, ProtocolError};
use std::time::Duration;
use tokio::sync::Semaphore;
mod native_input;
mod native_resize;
mod native_signal;
mod tag;

fn source(rejection: Option<Rejection>, format: Option<ResponseFormat>, owner: Owner) -> Source {
    Source {
        rejection,
        format,
        request_error: RequestError::Protocol(ProtocolError::Unimplemented),
        deadline: Instant::now() + Duration::from_secs(30),
        _owner: owner,
    }
}
fn limits() -> reply::Limits {
    reply::Limits {
        wire_bytes: 4096,
        chunk_bytes: 4096,
    }
}
fn format(rpc: Rpc, gzip: bool, utf8: bool) -> ResponseFormat {
    ResponseFormat::new(
        rpc,
        if utf8 {
            JsonContentType::Utf8
        } else {
            JsonContentType::Json
        },
        if gzip {
            Encoding::Gzip
        } else {
            Encoding::Identity
        },
    )
}
async fn drain(mut body: axum::body::Body) -> Vec<u8> {
    let mut result = Vec::new();
    while let Some(frame) = body.frame().await {
        result.extend_from_slice(&frame.unwrap().into_data().unwrap());
    }
    result
}
fn expand(bytes: &[u8]) -> Vec<u8> {
    let mut decoder = Decoder::new(
        bytes,
        DecodeLimits {
            encoded_bytes: 4096,
            decoded_bytes: 4096,
            chunk_bytes: 4096,
            header_bytes: 128,
            members: 1,
        },
    )
    .unwrap();
    let value = decoder.next_chunk().unwrap().unwrap();
    assert!(decoder.next_chunk().unwrap().is_none());
    value
}

#[tokio::test]
async fn operation_replies_retain_exact_budget_original_error_and_last_byte_reference() {
    for rpc in [Rpc::Connect, Rpc::SendSignal] {
        for rejection in [
            Rejection::MissingSelector,
            Rejection::MissingInput,
            Rejection::PidNotFound(u32::MAX),
            Rejection::TagNotFound("tag\"\\\n<>&中".into()),
            Rejection::Signal(i32::MIN),
        ] {
            let format = Some(format(rpc, true, true));
            let response = send(
                source(Some(rejection.clone()), format, Arc::new(())),
                limits(),
            )
            .await
            .unwrap();
            let bytes = drain(response.into_body()).await;
            let raw = payload(&rejection, rpc.is_stream(), 4096).unwrap();
            let required = bytes
                .len()
                .max(raw.len() + if rpc.is_stream() { 5 } else { 0 });
            for budget in [required, required - 1] {
                let slots = Arc::new(Semaphore::new(1));
                let owner: Owner = Arc::new(slots.clone().try_acquire_owned().unwrap());
                let source = source(Some(rejection.clone()), format, owner);
                let deadline = source.deadline;
                let result = send(
                    source,
                    reply::Limits {
                        wire_bytes: budget,
                        ..limits()
                    },
                )
                .await;
                if budget == required {
                    let mut body = result.unwrap().into_body();
                    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
                    let copy = bytes.clone();
                    drop(bytes);
                    drop(body);
                    assert_eq!(slots.available_permits(), 0);
                    drop(copy);
                } else {
                    let failure = result.unwrap_err();
                    assert_eq!(failure.error(), reply::Error::ResponseLimit);
                    assert_eq!(failure.deadline(), deadline);
                    assert_eq!(
                        failure.request_error(),
                        RequestError::Protocol(ProtocolError::Unimplemented)
                    );
                    assert_eq!(slots.available_permits(), 0);
                    drop(failure);
                }
                assert_eq!(slots.available_permits(), 1);
            }
        }
    }
}

#[tokio::test]
async fn missing_operation_observation_selection_and_expiry_do_not_invent_a_reply() {
    for (observation, format, expired) in [
        (None, Some(format(Rpc::Connect, true, false)), false),
        (Some(Rejection::MissingSelector), None, false),
        (
            Some(Rejection::MissingSelector),
            Some(format(Rpc::Connect, true, false)),
            true,
        ),
    ] {
        let slots = Arc::new(Semaphore::new(1));
        let owner: Owner = Arc::new(slots.clone().try_acquire_owned().unwrap());
        let mut source = source(observation, format, owner);
        if expired {
            source.deadline = Instant::now();
        }
        let deadline = source.deadline;
        let failure = send(source, limits()).await.unwrap_err();
        assert_eq!(
            failure.error(),
            if expired {
                reply::Error::DeadlineExceeded
            } else {
                reply::Error::UnclassifiedFault
            }
        );
        assert_eq!(failure.deadline(), deadline);
        assert_eq!(slots.available_permits(), 0);
        drop(failure);
        let _permit = tokio::time::timeout(Duration::from_secs(5), slots.acquire())
            .await
            .unwrap()
            .unwrap();
    }
}

fn operation_failures(rpc: Rpc) -> Vec<(Rejection, &'static str, i64)> {
    let mut failures = vec![(Rejection::MissingSelector, "selector", 0_i64)];
    failures.extend(
        [0, 1, 123, 2_147_483_648, u32::MAX]
            .map(|pid| (Rejection::PidNotFound(pid), "pid", i64::from(pid))),
    );
    if rpc == Rpc::SendInput {
        failures.push((Rejection::MissingInput, "input", 0));
        failures.push((Rejection::StdinClosed, "stdin-closed", 0));
        failures.extend([
            (
                Rejection::Channel(ChannelRejection::StdinOnPty),
                "stdin-on-pty",
                0,
            ),
            (
                Rejection::Channel(ChannelRejection::PtyOnPipe),
                "pty-on-pipe",
                0,
            ),
        ]);
    }
    if rpc == Rpc::CloseStdin {
        failures.push((
            Rejection::Channel(ChannelRejection::ClosePty),
            "close-pty",
            0,
        ));
    }
    if rpc == Rpc::Update {
        failures.push((
            Rejection::Channel(ChannelRejection::ResizePipe),
            "resize-pipe",
            0,
        ));
    }
    if rpc == Rpc::SendSignal {
        failures.extend(
            [0, -1, 1, i32::MIN, i32::MAX].map(|n| (Rejection::Signal(n), "signal", i64::from(n))),
        );
    }
    failures
}

#[tokio::test]
#[ignore = "explicit fixed Go operation failure rendering; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_pid_parameter_failure_status_headers_and_payloads_match() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    for rpc in [
        Rpc::Connect,
        Rpc::SendInput,
        Rpc::SendSignal,
        Rpc::CloseStdin,
        Rpc::Update,
    ] {
        let failures = operation_failures(rpc);
        for gzip in [false, true] {
            for utf8 in [false, true] {
                for (rejection, kind, number) in &failures {
                    let response = send(
                        source(
                            Some(rejection.clone()),
                            Some(format(rpc, gzip, utf8)),
                            Arc::new(()),
                        ),
                        limits(),
                    )
                    .await
                    .unwrap();
                    let mut observation = vec![response.status().as_u16().to_string()];
                    for name in [
                        "content-type",
                        "content-encoding",
                        "connect-content-encoding",
                        "accept-encoding",
                        "connect-accept-encoding",
                        "content-length",
                        "allow",
                    ] {
                        observation.push(hex(response
                            .headers()
                            .get(name)
                            .map_or(b"", |v| v.as_bytes())));
                    }
                    let bytes = drain(response.into_body()).await;
                    if rpc.is_stream() {
                        assert_eq!(
                            bytes.len(),
                            u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize + 5
                        );
                        observation.push(bytes[0].to_string());
                        observation.push(hex(&if gzip {
                            expand(&bytes[5..])
                        } else {
                            bytes[5..].to_vec()
                        }));
                    } else {
                        observation.push(String::new());
                        observation.push(hex(&bytes));
                    }
                    let input = serde_json::json!({"Method":rpc.method(),"Kind":kind,"Number":number,"Gzip":gzip,"UTF8":utf8});
                    oracle.check(
                        &format!("R\t{}", hex(&serde_json::to_vec(&input).unwrap())),
                        &observation.join("\t"),
                    );
                    count += 1;
                }
            }
        }
    }
    oracle.finish();
    eprintln!(
        "{count} fixed Go operation failure rendering cases; process lookup and ordering are separate Linux tests."
    );
}

#[tokio::test]
async fn channel_rejections_keep_owning_the_request_and_refuse_stream_misclassification() {
    for rejection in [
        ChannelRejection::StdinOnPty,
        ChannelRejection::PtyOnPipe,
        ChannelRejection::ClosePty,
        ChannelRejection::ResizePipe,
    ]
    .map(Rejection::Channel)
    .into_iter()
    .chain([Rejection::StdinClosed, Rejection::StdinDisabled])
    .chain([Rejection::NativeSignal(3), Rejection::NativeSignal(1)])
    .chain([Rejection::NativeResize(5), Rejection::NativeResize(22)])
    .chain([false, true].map(|pty| Rejection::NativeInput {
        pid: u32::MAX,
        pty,
        errno: 5,
    })) {
        let raw = payload(&rejection, false, 4096).unwrap();
        for (stream, budget, expired) in [
            (false, raw.len(), false),
            (false, raw.len() - 1, false),
            (true, 4096, false),
            (false, 4096, true),
        ] {
            let slots = Arc::new(Semaphore::new(1));
            let owner: Owner = Arc::new(slots.clone().try_acquire_owned().unwrap());
            let mut source = source(
                Some(rejection.clone()),
                Some(format(
                    if stream { Rpc::Connect } else { Rpc::SendInput },
                    true,
                    true,
                )),
                owner,
            );
            if expired {
                source.deadline = Instant::now();
            }
            let deadline = source.deadline;
            let result = send(
                source,
                reply::Limits {
                    wire_bytes: budget,
                    ..limits()
                },
            )
            .await;
            if !stream && !expired && budget == raw.len() {
                let response = result.unwrap();
                assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
                let mut body = response.into_body();
                let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
                assert_eq!(bytes, raw);
                let last = bytes.clone();
                drop(bytes);
                drop(body);
                assert_eq!(slots.available_permits(), 0);
                drop(last);
            } else {
                let failure = result.unwrap_err();
                assert_eq!(failure.deadline(), deadline);
                assert_eq!(
                    failure.error(),
                    if stream {
                        reply::Error::UnclassifiedFault
                    } else if expired {
                        reply::Error::DeadlineExceeded
                    } else {
                        reply::Error::ResponseLimit
                    }
                );
                assert_eq!(slots.available_permits(), 0);
                drop(failure);
            }
            let _released = tokio::time::timeout(Duration::from_secs(5), slots.acquire())
                .await
                .unwrap()
                .unwrap();
        }
    }
}
