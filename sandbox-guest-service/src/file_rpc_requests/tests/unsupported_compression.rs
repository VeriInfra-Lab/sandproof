use super::*;
use crate::file_rpc_response;
use http_body_util::BodyExt;

fn fields(values: &[&[u8]], version: &[u8], timeout: &[u8]) -> HeaderMap {
    let mut h = headers();
    for value in values {
        h.append("content-encoding", HeaderValue::from_bytes(value).unwrap());
    }
    h.insert(
        "connect-protocol-version",
        HeaderValue::from_bytes(version).unwrap(),
    );
    h.insert(
        "connect-timeout-ms",
        HeaderValue::from_bytes(timeout).unwrap(),
    );
    h
}
async fn failed(pool: &FileRpcRequests, session: &Session<Meta>, h: HeaderMap) -> Failure {
    let (body, polls) = body(vec![data(b"invalid JSON")], Some(12));
    let failure = pool
        .admit()
        .unwrap()
        .authenticate(session, h)
        .await
        .unwrap()
        .decode("MakeDir", body)
        .await
        .err()
        .unwrap();
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    failure
}
fn response_limits() -> file_rpc_response::Limits {
    file_rpc_response::Limits {
        wire_bytes: 65536,
        chunk_bytes: 19,
    }
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut output = Vec::new();
    while let Some(frame) = body.frame().await {
        output.extend_from_slice(&frame.unwrap().into_data().unwrap());
    }
    output
}

#[tokio::test]
async fn unknown_first_encoding_precedes_version_timeout_and_body_with_owned_501() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    for encoding in ["br", "GZIP", "gzip;q=1", "gzip,identity", "secret-codec"] {
        let failure = failed(
            &pool,
            &session,
            fields(&[encoding.as_bytes(), b"gzip"], b"2", b"invalid"),
        )
        .await;
        let Error::UnsupportedCompression(value) = failure.error() else {
            panic!("wrong priority")
        };
        assert_eq!(value.value(), encoding.as_bytes());
        assert!(!format!("{failure:?}").contains(encoding));
        let response = failure.into_response(response_limits()).await.unwrap();
        assert_eq!(response.status(), 501);
        assert_eq!(response.headers()["content-type"], "application/json");
        assert_eq!(response.headers()["accept-encoding"], "gzip");
        assert!(!response.headers().contains_key("content-encoding"));
        full(&pool);
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        let length = response.headers()["content-length"].clone();
        let bytes = drain(response.into_body()).await;
        assert_eq!(length, bytes.len().to_string());
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["code"], "unimplemented");
        assert_eq!(
            json["message"],
            format!("unknown compression {encoding:?}: supported encodings are gzip")
        );
        released(&pool).await;
    }
    // A supported first value ignores later encodings, even when they are bad.
    for values in [
        vec![],
        vec![b"".as_slice(), b"br"],
        vec![b"identity".as_slice(), b"br"],
        vec![b"gzip".as_slice(), b"br"],
    ] {
        let failure = failed(&pool, &session, fields(&values, b"2", b"invalid")).await;
        assert!(matches!(failure.error(), Error::Version(_)));
        drop(failure);
        released(&pool).await;
    }
}

#[tokio::test]
async fn compression_rejection_preserves_tightened_deadline_exact_budget_and_last_copy() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    let expected = br#"{"code":"unimplemented","message":"unknown compression \"br\": supported encodings are gzip"}"#;
    for wire_bytes in [expected.len(), expected.len() - 1] {
        let failure = failed(&pool, &session, fields(&[b"br"], b"2", b"5000")).await;
        let end = failure.deadline();
        assert!(end <= Instant::now() + Duration::from_secs(5));
        let response = failure
            .into_response(file_rpc_response::Limits {
                wire_bytes,
                ..response_limits()
            })
            .await;
        full(&pool);
        if wire_bytes == expected.len() {
            assert_eq!(drain(response.unwrap().into_body()).await, expected);
        } else {
            let failure = response.unwrap_err();
            assert_eq!(failure.error(), file_rpc_response::Error::ResponseLimit);
            assert_eq!(failure.deadline(), end);
            drop(failure);
        }
        released(&pool).await;
    }
    let failure = failed(&pool, &session, fields(&[b"br"], b"1", b"0")).await;
    assert!(matches!(failure.error(), Error::UnsupportedCompression(_)));
    let end = failure.deadline();
    let expired = failure.into_response(response_limits()).await.unwrap_err();
    assert_eq!(expired.error(), file_rpc_response::Error::DeadlineExceeded);
    assert_eq!(expired.deadline(), end);
    full(&pool);
    drop(expired);
    released(&pool).await;
    let failure = failed(&pool, &session, fields(&[b"br"], b"1", b"")).await;
    let mut body = failure
        .into_response(response_limits())
        .await
        .unwrap()
        .into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let last = bytes.clone();
    drop(bytes);
    drop(body);
    full(&pool);
    drop(last);
    released(&pool).await;
}

#[tokio::test]
#[ignore = "explicit fixed Connect unsupported compression HTTP reference; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_unknown_compression_and_header_priority_match_complete_http_response() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut encodings = vec![
        b"br".to_vec(),
        b"GZIP".to_vec(),
        b"gzip;q=1".to_vec(),
        b"gzip,identity".to_vec(),
        b"a\t<b>&\"\\".to_vec(),
        "编码\u{2028}".as_bytes().to_vec(),
    ];
    encodings.extend((0x80..=0xff).map(|byte| vec![byte]));
    let mut count = 0;
    for value in encodings {
        for (version, timeout) in [
            (b"1".as_slice(), b"".as_slice()),
            (b"2".as_slice(), b"invalid".as_slice()),
        ] {
            let encodings = [value.as_slice(), b"gzip"];
            let failure = failed(&pool, &session, fields(&encodings, version, timeout)).await;
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
                observed.push(hex(response
                    .headers()
                    .get(name)
                    .map_or(b"", |v| v.as_bytes())));
            }
            observed.push(hex(&drain(response.into_body()).await));
            let input = serde_json::json!({"version":[hex(version)],"timeout":[hex(timeout)],"compression":encodings.iter().map(|v|hex(v)).collect::<Vec<_>>()});
            oracle.check(
                &format!("H\t{}", hex(input.to_string().as_bytes())),
                &observed.join("\t"),
            );
            released(&pool).await;
            count += 1;
        }
    }
    oracle.finish();
    eprintln!("{count} fixed Connect compression rejection and precedence HTTP cases.");
}
