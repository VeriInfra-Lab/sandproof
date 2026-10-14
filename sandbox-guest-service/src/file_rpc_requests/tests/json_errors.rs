use super::*;
use crate::file_rpc_response;
use http_body_util::BodyExt;
use serde_json::json;

fn budgets() -> Limits {
    let mut limits = limits();
    limits.body.total_bytes = 65536;
    limits.body.frame_bytes = 65536;
    limits
}
fn response_limits() -> file_rpc_response::Limits {
    file_rpc_response::Limits {
        wire_bytes: 65536,
        chunk_bytes: 17,
    }
}
async fn rejected(
    pool: &FileRpcRequests,
    session: &Session<Meta>,
    method: &str,
    body: &[u8],
) -> Failure {
    pool.admit()
        .unwrap()
        .authenticate(session, headers())
        .await
        .unwrap()
        .decode(method, Body::from(body.to_vec()))
        .await
        .err()
        .expect("JSON rejection required")
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(frame) = body.frame().await {
        let data = frame.unwrap().into_data().unwrap();
        out.extend_from_slice(&data);
    }
    out
}

#[tokio::test]
async fn json_rejections_keep_diagnostics_private_and_retain_capacity_through_last_bytes() {
    let session = session().await;
    let pool = FileRpcRequests::new(budgets()).unwrap();
    let input = br#"{"path":"sensitive","path":"again"}"#;
    let failed = rejected(&pool, &session, "Stat", input).await;
    assert!(failed.has_response());
    assert!(!format!("{failed:?}").contains("path"));
    full(&pool);
    session
        .initialize(br#"{"accessToken":"token"}"#)
        .await
        .unwrap();
    let response = failed.into_response(response_limits()).await.unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(response.headers()["accept-encoding"], "gzip");
    let bytes = drain(response.into_body()).await;
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value["message"],
        "unmarshal message: unmarshal into *filesystem.StatRequest: proto: (line 1:21): duplicate field \"path\""
    );
    released(&pool).await;
    let end = Instant::now();
    let failed = rejected(&pool, &session, "Stat", input).await;
    assert!(failed.deadline() >= end);
    let expected_end = failed.deadline();
    let failure = failed
        .into_response(file_rpc_response::Limits {
            wire_bytes: bytes.len() - 1,
            chunk_bytes: 17,
        })
        .await
        .err()
        .unwrap();
    assert_eq!(failure.error(), file_rpc_response::Error::ResponseLimit);
    assert_eq!(failure.deadline(), expected_end);
    full(&pool);
    drop(failure);
    released(&pool).await;
    let failed = rejected(&pool, &session, "Stat", input).await;
    let response = failed
        .into_response(file_rpc_response::Limits {
            wire_bytes: bytes.len(),
            chunk_bytes: bytes.len(),
        })
        .await
        .unwrap();
    let mut body = response.into_body();
    let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let retained = frame.clone();
    drop(frame);
    drop(body);
    full(&pool);
    assert_eq!(retained.as_ref(), bytes);
    drop(retained);
    released(&pool).await;
}

fn corpus() -> Vec<(&'static str, Vec<u8>)> {
    let mut cases = Vec::new();
    for method in ["Stat", "MakeDir", "ListDir", "Move", "Remove"] {
        cases.extend(
            super::protobuf::messages()
                .into_iter()
                .map(|body| (method, body)),
        );
    }
    cases.extend(
        super::protobuf::numbers()
            .into_iter()
            .map(|n| ("ListDir", format!(r#"{{"depth":{n}}}"#).into_bytes())),
    );
    cases.extend(super::protobuf::exponent_messages());
    cases.extend(unicode_escapes());
    for byte in 0..=255 {
        for (prefix, suffix) in [
            (b"{\"path\":\"".as_slice(), b"\"}".as_slice()),
            (b"{\"future\":\"", b"\"}"),
            (b"{\"future\":{\"", b"\":0}}"),
            (b"{\"path\"", b"}"),
        ] {
            cases.push(("Stat", [prefix, &[byte], suffix].concat()));
        }
    }
    for seed in [
        br#"{"path":"x","depth":2}"#.as_slice(),
        br#"{"path":"\u0041"}"#,
        br#"{"future":[1,true,null]}"#,
    ] {
        for at in 0..seed.len() {
            for byte in [
                0, 9, 10, 32, b'0', b'+', b'e', b':', b'\\', b'"', b'}', b'[', b']', b',', 0xff,
            ] {
                let mut body = seed.to_vec();
                body[at] = byte;
                cases.push(("ListDir", body));
            }
        }
    }
    for body in [
        "{} false",
        "trueX",
        "{\"path\":]}",
        "{\"path\":\"a\",}",
        "\n{\"future雪\":0,\"depth\":false}",
        "{\"path\":\"a\",\"pa\\u0074h\":null}",
        "{\"source\":true,\"path\":false}",
    ] {
        cases.push(("Move", body.as_bytes().to_vec()));
        cases.push(("ListDir", body.as_bytes().to_vec()));
    }
    cases
}

#[tokio::test]
async fn expired_json_failure_never_receives_a_new_response_deadline() {
    let session = session().await;
    let pool = FileRpcRequests::new(budgets()).unwrap();
    let failed = rejected(&pool, &session, "ListDir", br#"{"depth":false}"#).await;
    let deadline = failed.deadline();
    tokio::time::pause();
    tokio::time::advance(deadline.saturating_duration_since(Instant::now())).await;
    let failure = failed.into_response(response_limits()).await.unwrap_err();
    assert_eq!(failure.error(), file_rpc_response::Error::DeadlineExceeded);
    assert_eq!(failure.deadline(), deadline);
    full(&pool);
    drop(failure);
    released(&pool).await;
}

fn unicode_escapes() -> Vec<(&'static str, Vec<u8>)> {
    let mut cases = Vec::new();
    for seed in [
        br#"{"path":"\u0000"}"#.as_slice(),
        br#"{"path":"\ud800\u0000"}"#,
    ] {
        let start = seed.windows(2).rposition(|bytes| bytes == b"\\u").unwrap() + 2;
        for offset in 0..4 {
            for byte in 0..=255 {
                let mut body = seed.to_vec();
                body[start + offset] = byte;
                cases.push(("Stat", body));
            }
        }
    }
    for body in [
        "\n{\"future\":\"雪😀\",\"depth\":false}",
        "{\"depth\":\"proto:\u{a0}literal\"}",
    ] {
        cases.push(("ListDir", body.as_bytes().to_vec()));
    }
    cases
}

#[tokio::test]
#[ignore = "explicit original protobuf/Connect JSON failure HTTP reference; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn original_generated_json_failures_match_complete_messages_and_http_headers() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let session = session().await;
    let pool = FileRpcRequests::new(budgets()).unwrap();
    let mut count = 0;
    for (method, body) in corpus() {
        let decoded = file_protocol::decode_request_observed(method, &body, body.len().max(1));
        match decoded {
            Ok(_) => continue,
            Err(file_protocol::DecodeFailure::Json(_)) => {}
            unexpected => panic!("unmapped diagnostic {unexpected:?}: {body:?}"),
        }
        let failed = rejected(&pool, &session, method, &body).await;
        assert!(failed.has_response());
        let response = failed.into_response(response_limits()).await.unwrap();
        let mut output = vec![response.status().as_u16().to_string()];
        for name in [
            "content-type",
            "content-length",
            "accept-encoding",
            "content-encoding",
            "x-content-type-options",
            "vary",
        ] {
            let value = response
                .headers()
                .get(name)
                .map_or(b"".as_slice(), |v| v.as_bytes());
            output.push(hex(if name == "content-length" && !value.is_empty() {
                b"valid"
            } else {
                value
            }));
        }
        let length = response.headers().get("content-length").cloned();
        let bytes = drain(response.into_body()).await;
        if let Some(length) = length {
            assert_eq!(length, bytes.len().to_string());
        }
        output.push(hex(&bytes));
        let input = json!({"Method":method,"Body":hex(&body)});
        oracle.check(
            &format!("K\t{}", hex(input.to_string().as_bytes())),
            &output.join("\t"),
        );
        released(&pool).await;
        count += 1;
    }
    oracle.finish();
    eprintln!(
        "{count} original protobuf JSON rejection HTTP cases; only build-dependent proto-prefix whitespace normalized."
    );
}
