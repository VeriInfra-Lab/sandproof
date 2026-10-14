use super::*;
use crate::file_rpc_response;
use http_body_util::BodyExt;

fn cases() -> Vec<Vec<u8>> {
    let member = gzip::compressed(b"{}");
    let joined = [member.clone(), member.clone()].concat();
    let mut cases: Vec<_> = (1..joined.len())
        .filter(|n| *n != member.len())
        .map(|n| joined[..n].to_vec())
        .collect();
    let mut crc = member.clone();
    let n = crc.len();
    crc[n - 8] ^= 1;
    let mut size = member.clone();
    let n = size.len();
    size[n - 4] ^= 1;
    cases.extend([
        b"invalid gzip header".to_vec(),
        crc,
        size,
        [member, b"invalid gzip member".to_vec()].concat(),
    ]);
    cases
}
async fn failed(pool: &FileRpcRequests, session: &Session<Meta>, data: &[u8]) -> Failure {
    let mut h = headers();
    h.insert("content-encoding", "gzip".parse().unwrap());
    let body = Body::from_stream(stream::iter(
        data.chunks(17)
            .map(|b| Ok::<_, std::io::Error>(Bytes::copy_from_slice(b)))
            .collect::<Vec<_>>(),
    ));
    pool.admit()
        .unwrap()
        .authenticate(session, h)
        .await
        .unwrap()
        .decode("MakeDir", body)
        .await
        .err()
        .unwrap()
}
fn response_limits() -> file_rpc_response::Limits {
    file_rpc_response::Limits {
        wire_bytes: 65536,
        chunk_bytes: 13,
    }
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        bytes.extend_from_slice(&frame.unwrap().into_data().unwrap());
    }
    bytes
}
fn message(error: &Error) -> &'static str {
    match error {
        Error::Compression {
            stage: CompressionStage::Header,
            error: GzipError::Truncated,
        } => "get decompressor: unexpected EOF",
        Error::Compression {
            stage: CompressionStage::Header,
            error: GzipError::Header,
        } => "get decompressor: gzip: invalid header",
        Error::Compression {
            stage: CompressionStage::Body,
            error: GzipError::Truncated,
        } => "decompress: unexpected EOF",
        Error::Compression {
            stage: CompressionStage::Body,
            error: GzipError::Header,
        } => "decompress: gzip: invalid header",
        Error::Compression {
            stage: CompressionStage::Body,
            error: GzipError::Checksum,
        } => "decompress: gzip: invalid checksum",
        error => panic!("unexpected gzip classification: {error:?}"),
    }
}

#[tokio::test]
async fn observed_gzip_errors_keep_fixed_phase_identity_json_and_exact_response_budget() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    for data in cases() {
        let failure = failed(&pool, &session, &data).await;
        assert!(failure.has_response());
        let expected = format!(
            r#"{{"code":"invalid_argument","message":"{}"}}"#,
            message(failure.error())
        );
        let response = failure
            .into_response(file_rpc_response::Limits {
                wire_bytes: expected.len(),
                ..response_limits()
            })
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
        assert_eq!(response.headers()["content-type"], "application/json");
        assert_eq!(response.headers()["accept-encoding"], "gzip");
        assert!(!response.headers().contains_key("content-encoding"));
        assert_eq!(
            response.headers()["content-length"],
            expected.len().to_string()
        );
        full(&pool);
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        assert_eq!(drain(response.into_body()).await, expected.as_bytes());
        released(&pool).await;
        let failure = failed(&pool, &session, &data).await;
        let end = failure.deadline();
        let failure = failure
            .into_response(file_rpc_response::Limits {
                wire_bytes: expected.len() - 1,
                ..response_limits()
            })
            .await
            .unwrap_err();
        assert_eq!(failure.error(), file_rpc_response::Error::ResponseLimit);
        assert_eq!(failure.deadline(), end);
        full(&pool);
        drop(failure);
        released(&pool).await;
    }
    let failure = failed(&pool, &session, b"invalid gzip header").await;
    let mut body = failure
        .into_response(response_limits())
        .await
        .unwrap()
        .into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = bytes.clone();
    drop(bytes);
    drop(body);
    full(&pool);
    drop(copy);
    released(&pool).await;
}

#[tokio::test]
async fn unmapped_codec_and_budget_failures_never_invent_a_connect_response() {
    let pool = FileRpcRequests::new(limits()).unwrap();
    for error in [
        Error::BodyLimit,
        Error::Transport,
        Error::DeadlineExceeded,
        Error::Unavailable,
        Error::Compression {
            stage: CompressionStage::Header,
            error: GzipError::EmptyHeader,
        },
        Error::Compression {
            stage: CompressionStage::Header,
            error: GzipError::Checksum,
        },
        Error::Compression {
            stage: CompressionStage::Body,
            error: GzipError::Deflate,
        },
        Error::Compression {
            stage: CompressionStage::Body,
            error: GzipError::DecodedLimit,
        },
    ] {
        let admitted = pool.admit().unwrap();
        let end = admitted.deadline;
        let failure = Failure {
            error,
            deadline: end,
            _owner: admitted.lease,
        };
        assert!(!failure.has_response());
        let failure = failure.into_response(response_limits()).await.unwrap_err();
        assert_eq!(failure.error(), file_rpc_response::Error::UnclassifiedFault);
        assert_eq!(failure.deadline(), end);
        full(&pool);
        drop(failure);
        released(&pool).await;
    }
}

#[tokio::test]
#[ignore = "explicit fixed Connect gzip failure HTTP reference; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_gzip_failures_match_status_headers_and_complete_identity_payload() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    for accept in ["gzip", "identity"] {
        for data in cases() {
            let failure = failed(&pool, &session, &data).await;
            let response = failure.into_response(response_limits()).await.unwrap();
            let mut observed = vec![response.status().as_u16().to_string()];
            for name in [
                "content-type",
                "content-length",
                "accept-encoding",
                "content-encoding",
                "x-content-type-options",
                "vary",
            ] {
                let bytes = response
                    .headers()
                    .get(name)
                    .map_or(b"".as_slice(), |v| v.as_bytes());
                if name == "content-length" {
                    observed.push(hex(b"valid"));
                } else {
                    observed.push(hex(bytes));
                }
            }
            let length = response.headers()["content-length"].clone();
            let body = drain(response.into_body()).await;
            assert_eq!(length, body.len().to_string());
            observed.push(hex(&body));
            let input = serde_json::json!({"Sent":["gzip"],"Accept":[accept],"Data":hex(&data)});
            oracle.check(
                &format!("G\t{}", hex(input.to_string().as_bytes())),
                &observed.join("\t"),
            );
            released(&pool).await;
            count += 1;
        }
    }
    oracle.finish();
    eprintln!("{count} fixed Connect gzip error status/header/complete identity payload cases.");
}
