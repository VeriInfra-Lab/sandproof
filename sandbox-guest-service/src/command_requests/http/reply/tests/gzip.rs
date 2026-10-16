use super::*;
use crate::command_requests::{CompressionStage, Error as RequestError, compression};
use crate::file_download::tests::reference::{Reference, hex};
use crate::file_rpc_requests::tests::gzip::compressed;
use crate::upload_body::GzipError;

fn frame(flag: u8, bytes: &[u8]) -> Vec<u8> {
    let mut result = vec![flag];
    result.extend(u32::try_from(bytes.len()).unwrap().to_be_bytes());
    result.extend(bytes);
    result
}
fn failures() -> Vec<Vec<u8>> {
    let member = compressed(b"{}");
    let joined = [member.clone(), member.clone()].concat();
    let mut cases: Vec<_> = (1..joined.len())
        .filter(|n| *n != member.len())
        .map(|n| joined[..n].to_vec())
        .collect();
    for offset in [8, 4] {
        let mut broken = member.clone();
        let at = broken.len() - offset;
        broken[at] ^= 1;
        cases.push(broken);
    }
    cases.extend([
        b"invalid gzip header".to_vec(),
        [member, b"invalid member".to_vec()].concat(),
    ]);
    cases
}
fn observed(rpc: Rpc, body: &[u8]) -> SelectionError {
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    SelectionError::Request(
        compression::decode(rpc.method(), body, 16384, 13, deadline(), &cancelled).unwrap_err(),
    )
}

#[tokio::test]
async fn gzip_replies_keep_known_phase_exact_budget_original_deadline_and_last_copy() {
    for rpc in [Rpc::List, Rpc::Start] {
        let (_, format) = rejected(rpc, true);
        let data = if rpc.is_stream() {
            frame(1, b"bad")
        } else {
            b"bad".to_vec()
        };
        let error = observed(rpc, &data);
        let full = send(error.clone(), format, limits(), deadline(), Arc::new(()))
            .await
            .unwrap();
        assert_eq!(full.status(), if rpc.is_stream() { 200 } else { 400 });
        let bytes = drain(full.into_body()).await;
        let payload = if rpc.is_stream() {
            decoded(&bytes[5..])
        } else {
            bytes.clone()
        };
        let prefix = if rpc.is_stream() { 5 } else { 0 };
        let required = bytes.len().max(payload.len() + prefix);
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
                let response = result.unwrap();
                let mut body = response.into_body();
                let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
                let copy = bytes.clone();
                drop(body);
                drop(bytes);
                assert_eq!(slots.available_permits(), 0);
                drop(copy);
            }
            released(slots).await;
        }
        let (owner, slots) = retain();
        let end = Instant::now();
        let failure = send(error.clone(), format, limits(), end, owner)
            .await
            .unwrap_err();
        assert_eq!(failure.error(), Error::DeadlineExceeded);
        assert_eq!(failure.selection_error(), &error);
        assert_eq!(slots.available_permits(), 0);
        drop(failure);
        released(slots).await;
    }
}

#[tokio::test]
async fn unknown_codec_budget_empty_json_and_missing_selection_do_not_fabricate_gzip_text() {
    let (_, format) = rejected(Rpc::Start, true);
    for error in [
        RequestError::BodyLimit,
        RequestError::DeadlineExceeded,
        RequestError::Unavailable,
        RequestError::Protocol(sandproof_guest_runtime::protocol::ProtocolError::InvalidArgument),
        RequestError::Compression {
            stage: CompressionStage::Header,
            error: GzipError::Checksum,
        },
        RequestError::Compression {
            stage: CompressionStage::Body,
            error: GzipError::Deflate,
        },
    ] {
        let (owner, slots) = retain();
        let error = SelectionError::Request(error);
        let result = send(error.clone(), format, limits(), deadline(), owner).await;
        if error == SelectionError::Request(RequestError::BodyLimit) {
            // Known local quota is now an explicit resource refusal, not an
            // invented gzip diagnostic. Unknown codec/I/O faults below remain
            // unclassified and keep the original request responsibility.
            let response = result.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::OK);
            let wire = drain(response.into_body()).await;
            assert_eq!(wire[0], 3);
            assert_eq!(decoded(&wire[5..]), br#"{"error":{"code":"resource_exhausted","message":"command request exceeds configured body limits"}}"#);
            released(slots).await;
            continue;
        }
        let failure = result.unwrap_err();
        assert_eq!(failure.error(), Error::UnclassifiedFault);
        assert_eq!(failure.selection_error(), &error);
        assert_eq!(slots.available_permits(), 0);
        drop(failure);
        released(slots).await;
    }
    let error = observed(Rpc::List, b"bad");
    assert_eq!(
        send(error, None, limits(), deadline(), Arc::new(()))
            .await
            .unwrap_err()
            .error(),
        Error::UnclassifiedFault
    );
}

fn selected_format(rpc: Rpc, utf8: bool) -> ResponseFormat {
    let content_type = if rpc.is_stream() {
        "application/connect+json"
    } else {
        "application/json"
    };
    let content_type = format!(
        "{content_type}{}",
        if utf8 { "; charset=utf-8" } else { "" }
    );
    let mut headers = HeaderMap::new();
    headers.insert("content-type", content_type.parse().unwrap());
    headers.insert(
        if rpc.is_stream() {
            "connect-content-encoding"
        } else {
            "content-encoding"
        },
        "gzip".parse().unwrap(),
    );
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

#[tokio::test]
#[ignore = "explicit fixed Go command gzip failure HTTP bytes; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_command_gzip_failures_match_headers_and_complete_error_payloads() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    for rpc in [
        Rpc::Start,
        Rpc::Connect,
        Rpc::List,
        Rpc::SendInput,
        Rpc::CloseStdin,
        Rpc::SendSignal,
        Rpc::Update,
    ] {
        for utf8 in [false, true] {
            let format = selected_format(rpc, utf8);
            for broken in failures() {
                let bodies = if rpc.is_stream() {
                    vec![
                        frame(1, &broken),
                        [frame(0, b"{}"), frame(3, &broken)].concat(),
                    ]
                } else {
                    vec![broken]
                };
                for body in bodies {
                    let error = observed(rpc, &body);
                    let response = send(error, Some(format), limits(), deadline(), Arc::new(()))
                        .await
                        .unwrap();
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
                            .map_or(b"", |v| v.as_bytes())));
                    }
                    let bytes = drain(response.into_body()).await;
                    if rpc.is_stream() {
                        assert_eq!(
                            bytes.len(),
                            u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize + 5
                        );
                        observed.push(bytes[0].to_string());
                        observed.push(hex(&decoded(&bytes[5..])));
                    } else {
                        observed.push(String::new());
                        observed.push(hex(&bytes));
                    }
                    let input = serde_json::json!({"Method":rpc.method(),"Body":hex(&body),"Gzip":true,"Reply":true,"UTF8":utf8});
                    oracle.check(
                        &format!("E\t{}", hex(&serde_json::to_vec(&input).unwrap())),
                        &observed.join("\t"),
                    );
                    count += 1;
                }
            }
        }
    }
    oracle.finish();
    eprintln!(
        "{count} fixed Go command gzip rejection HTTP status/header/flags/full payload cases."
    );
}
