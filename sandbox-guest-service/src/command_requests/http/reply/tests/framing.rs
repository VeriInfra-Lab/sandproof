use super::*;
use crate::command_requests::{Error as RequestError, compression};
use crate::file_download::tests::reference::{Reference, hex};
use crate::file_rpc_requests::tests::gzip::compressed;
use sandproof_guest_runtime::protocol::{FramingError as F, ProtocolError};

fn frame(flag: u8, bytes: &[u8]) -> Vec<u8> {
    let mut result = vec![flag];
    result.extend(u32::try_from(bytes.len()).unwrap().to_be_bytes());
    result.extend(bytes);
    result
}
fn observed(rpc: Rpc, wire: &[u8], gzip: bool) -> RequestError {
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    compression::decode_selected(rpc.method(), wire, 16384, 13, deadline(), &cancelled, gzip)
        .unwrap_err()
}
fn format(rpc: Rpc, gzip: bool, utf8: bool) -> ResponseFormat {
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        format!(
            "application/connect+json{}",
            if utf8 { "; charset=utf-8" } else { "" }
        )
        .parse()
        .unwrap(),
    );
    if gzip {
        headers.insert("connect-content-encoding", "gzip".parse().unwrap());
    }
    select(
        &headers,
        &Method::POST,
        rpc,
        4096,
        Instant::now(),
        deadline(),
    )
    .unwrap()
    .response()
}

#[test]
fn framing_observations_follow_actual_receive_and_decode_order() {
    for rpc in [Rpc::Start, Rpc::Connect] {
        assert_eq!(
            observed(rpc, b"", false),
            RequestError::Framing(F::ZeroMessages)
        );
        for n in 1..5 {
            assert_eq!(
                observed(rpc, &frame(255, b"{} ")[..n], false),
                RequestError::Framing(F::IncompleteEnvelope)
            );
        }
        // Truncation precedes compression support, even for special envelopes.
        assert_eq!(
            observed(rpc, &frame(255, b"{} ")[..7], false),
            RequestError::Framing(F::Promised {
                expected: 3,
                received: 2
            })
        );
        assert_eq!(
            observed(rpc, &frame(1, b""), false),
            RequestError::Framing(F::CompressedWithoutSupport)
        );
        assert_eq!(
            observed(rpc, &frame(2, b"{}"), false),
            RequestError::Framing(F::ZeroMessages)
        );
        assert_eq!(
            observed(rpc, &[frame(4, b"bad"), vec![7]].concat(), false),
            RequestError::Framing(F::ExtraBytes(1))
        );
        assert_eq!(
            observed(rpc, &frame(4, b"bad"), false),
            RequestError::Framing(F::InvalidFlags(4))
        );
        assert_eq!(
            observed(rpc, &[frame(0, b"{}"), frame(0, b"")].concat(), false),
            RequestError::Framing(F::MultipleMessages)
        );
        for wire in [
            [frame(0, b"bad"), vec![7]].concat(),
            [frame(0, b"{}"), frame(0, b"bad")].concat(),
            frame(2, b"bad"),
        ] {
            assert_eq!(
                observed(rpc, &wire, false),
                RequestError::Protocol(ProtocolError::InvalidArgument)
            );
        }
        assert!(matches!(
            observed(rpc, &[frame(3, b"bad"), vec![7]].concat(), true),
            RequestError::Compression { .. }
        ));
        assert_eq!(
            observed(rpc, &[frame(3, &compressed(b"{}")), vec![7]].concat(), true),
            RequestError::Framing(F::ExtraBytes(1))
        );
        // Component budgets are not relabeled as original wire truncation.
        assert_eq!(
            observed(rpc, &[0, 0, 1, 0, 0], false),
            RequestError::Protocol(ProtocolError::ResourceExhausted)
        );
    }
}

#[tokio::test]
async fn observed_framing_replies_keep_exact_budgets_deadline_and_last_byte_owner() {
    let cases = [
        b"".to_vec(),
        vec![0],
        frame(1, b""),
        frame(4, b"{}"),
        [frame(2, b"{}"), vec![0]].concat(),
    ];
    for gzip in [false, true] {
        for wire in &cases {
            let error = SelectionError::Request(observed(Rpc::Start, wire, false));
            let format = Some(format(Rpc::Start, gzip, true));
            let reply = send(error.clone(), format, limits(), deadline(), Arc::new(()))
                .await
                .unwrap();
            assert_eq!(reply.status(), 200);
            let bytes = drain(reply.into_body()).await;
            let raw = if gzip {
                decoded(&bytes[5..])
            } else {
                bytes[5..].to_vec()
            };
            let required = bytes.len().max(raw.len() + 5);
            for budget in [required, required - 1] {
                let (owner, slots) = retain();
                let end = deadline();
                let result = send(
                    error.clone(),
                    format,
                    Limits {
                        wire_bytes: budget,
                        chunk_bytes: 16384,
                    },
                    end,
                    owner,
                )
                .await;
                if budget < required {
                    let failure = result.unwrap_err();
                    assert_eq!(failure.error(), Error::ResponseLimit);
                    assert_eq!(failure.selection_error(), &error);
                    assert_eq!(failure.deadline(), end);
                    assert_eq!(slots.available_permits(), 0);
                    drop(failure);
                } else {
                    let mut body = result.unwrap().into_body();
                    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
                    let last = bytes.clone();
                    drop(body);
                    drop(bytes);
                    assert_eq!(slots.available_permits(), 0);
                    drop(last);
                }
                released(slots).await;
            }
            let (owner, slots) = retain();
            let failure = send(error.clone(), format, limits(), Instant::now(), owner)
                .await
                .unwrap_err();
            assert_eq!(failure.error(), Error::DeadlineExceeded);
            assert_eq!(failure.selection_error(), &error);
            assert_eq!(slots.available_permits(), 0);
            drop(failure);
            released(slots).await;
        }
    }
    let error = SelectionError::Request(observed(Rpc::Start, b"", false));
    let (_, unary) = rejected(Rpc::List, false);
    for format in [None, unary] {
        assert_eq!(
            send(error.clone(), format, limits(), deadline(), Arc::new(()))
                .await
                .unwrap_err()
                .error(),
            Error::UnclassifiedFault
        );
    }
}

fn cases(gzip: bool) -> Vec<Vec<u8>> {
    let mut cases = vec![vec![]];
    for flags in 0..=255 {
        for plain in [b"".as_slice(), b"{}"] {
            let bytes = if gzip && flags & 1 != 0 && !plain.is_empty() {
                compressed(plain)
            } else {
                plain.to_vec()
            };
            let envelope = frame(flags, &bytes);
            for prefix in [vec![], frame(0, b"{}")] {
                cases.push([prefix.clone(), envelope.clone()].concat());
                if flags % 8 == 0 {
                    cases.push([prefix, envelope.clone(), vec![7, 8, 9]].concat());
                }
            }
        }
    }
    for flag in [0, 1, 2, 3, 4, 255] {
        for n in 1..8 {
            let partial = frame(flag, b"{} ")[..n].to_vec();
            cases.push(partial.clone());
            cases.push([frame(0, b"{}"), partial].concat());
        }
    }
    cases
}

#[tokio::test]
#[ignore = "explicit fixed Go command framing HTTP bytes; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_framing_failures_match_status_headers_flags_and_complete_error_bytes() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    for rpc in [Rpc::Start, Rpc::Connect] {
        for gzip in [false, true] {
            for utf8 in [false, true] {
                let format = format(rpc, gzip, utf8);
                for body in cases(gzip) {
                    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
                    let Err(error @ RequestError::Framing(_)) = compression::decode_selected(
                        rpc.method(),
                        &body,
                        16384,
                        13,
                        deadline(),
                        &cancelled,
                        gzip,
                    ) else {
                        continue;
                    };
                    let reply = send(
                        SelectionError::Request(error),
                        Some(format),
                        limits(),
                        deadline(),
                        Arc::new(()),
                    )
                    .await
                    .unwrap();
                    let mut result = vec![reply.status().as_u16().to_string()];
                    for name in [
                        "content-type",
                        "content-encoding",
                        "connect-content-encoding",
                        "accept-encoding",
                        "connect-accept-encoding",
                        "content-length",
                        "allow",
                    ] {
                        result.push(hex(reply.headers().get(name).map_or(b"", |v| v.as_bytes())));
                    }
                    let bytes = drain(reply.into_body()).await;
                    assert_eq!(
                        bytes.len(),
                        u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize + 5
                    );
                    result.push(bytes[0].to_string());
                    result.push(hex(&if gzip {
                        decoded(&bytes[5..])
                    } else {
                        bytes[5..].to_vec()
                    }));
                    let input = serde_json::json!({"Method":rpc.method(),"Body":hex(&body),"Gzip":gzip,"Reply":true,"UTF8":utf8});
                    oracle.check(
                        &format!("E\t{}", hex(&serde_json::to_vec(&input).unwrap())),
                        &result.join("\t"),
                    );
                    count += 1;
                }
            }
        }
    }
    oracle.finish();
    eprintln!("{count} fixed Go command framing HTTP status/header/flags/full error byte cases.");
}
