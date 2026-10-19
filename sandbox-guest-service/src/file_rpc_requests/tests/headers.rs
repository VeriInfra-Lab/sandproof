use super::*;
use crate::file_rpc_response;
use http_body_util::BodyExt;

#[tokio::test]
async fn admitted_selection_uses_first_media_type_without_changing_headers_or_deadline() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    let mut h = headers();
    h.append(
        "content-type",
        "Application/JSON; Charset=\"UTF-8\"".parse().unwrap(),
    );
    h.append("content-type", "application/proto".parse().unwrap());
    let admitted = pool.admit().unwrap();
    let end = admitted.deadline();
    let auth = admitted.authenticate(&session, h.clone()).await.unwrap();
    let codec = auth.json_codec();
    assert_eq!(codec, Some(file_protocol::JsonContentType::Utf8));
    assert_eq!(
        select_http(&axum::http::Method::POST, codec),
        Ok(file_protocol::JsonContentType::Utf8)
    );
    assert_eq!(auth.headers(), &h);
    assert_eq!(auth.deadline(), end);
    full(&pool);
    drop(auth);
    released(&pool).await;
}

#[test]
#[ignore = "explicit fixed Go Connect HTTP reference; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
fn fixed_go_json_media_type_selection_reference() {
    use file_protocol::{JsonContentType, json_content_type};
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut cases = vec![
        String::new(),
        "application/proto".into(),
        "application/grpc+json".into(),
    ];
    for base in [
        "application/json",
        " APPLICATION/JSON ",
        "APPLİCATİON/JSON",
        "application/json-extra",
    ] {
        for suffix in [
            "",
            ";",
            "; ",
            ";charset=utf-8",
            "; Charset = \"UTF-8\"",
            ";charset=latin1",
            ";charset=utf-8;charset=utf-8",
            ";charset=UTF-8;charset=utf-8",
            ";charset=utf-8;ignored=x",
            ";ignored*=unknown''x",
            ";ignored*1=x",
            ";charset*=utf-8''ut%66-8",
            ";charset*=unknown''ignored",
            ";charset*0=ut;charset*1=f-8",
            ";charset*0*=utf-8''ut;charset*1*=f%2D8",
            ";charset=utf-8;charset*=unknown''ignored",
            ";charset=utf-8;charset*=utf-8''latin1",
            ";charset*=utf-8''%FF",
            ";bad",
            ";;",
            ";charset=utf-8;",
            ";charset=\"utf\\-8\"",
            ";charset=utf-8;bad*0*=unknown''ignored;bad*1=x",
            ";charset=utf-8\u{a0}",
        ] {
            cases.push(format!("{base}{suffix}"));
        }
    }
    for base in [
        "application/json;charset=utf-8",
        "application/json; charset=\"utf-8\"",
    ] {
        for index in 0..base.len() {
            for byte in *b" \t;=\"\\*/A0" {
                let mut value = base.as_bytes().to_vec();
                value[index] = byte;
                cases.push(String::from_utf8(value).unwrap());
            }
        }
    }
    let count = cases.len();
    for value in cases {
        let expected = match json_content_type(&value, 4096).unwrap() {
            Some(JsonContentType::Json) => "JSON",
            Some(JsonContentType::Utf8) => "UTF8",
            None => "OTHER",
        };
        oracle.check(&format!("C\t{}", hex(value.as_bytes())), expected);
    }
    oracle.finish();
    eprintln!(
        "{count} fixed Connect JSON media-type selection HTTP cases; other protocols are not implemented here."
    );
}

fn response_limits() -> file_rpc_response::Limits {
    file_rpc_response::Limits {
        wire_bytes: 65536,
        chunk_bytes: 17,
    }
}
async fn failure(
    pool: &FileRpcRequests,
    session: &Session<Meta>,
    version: &[&[u8]],
    timeout: &[&[u8]],
) -> Failure {
    let mut h = headers();
    for (field, values) in [
        ("connect-protocol-version", version),
        ("connect-timeout-ms", timeout),
    ] {
        for value in values {
            h.append(field, HeaderValue::from_bytes(value).unwrap());
        }
    }
    let (body, polls) = body(vec![data(b"{")], Some(1));
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
    full(pool);
    failure
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let data = frame.unwrap().into_data().unwrap();
        bytes.extend_from_slice(&data);
    }
    bytes
}

#[tokio::test]
async fn version_and_timeout_failures_precede_body_and_keep_original_capacity() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    for (version, timeout, expected) in [
        (
            "2",
            "invalid",
            "Connect-Protocol-Version must be \"1\": got \"2\"",
        ),
        (
            "1",
            "invalid",
            "parse timeout: strconv.ParseInt: parsing \"invalid\": invalid syntax",
        ),
        (
            "",
            "10000000000",
            "parse timeout: \"10000000000\" has >10 digits",
        ),
    ] {
        let failed = failure(
            &pool,
            &session,
            &[version.as_bytes()],
            &[timeout.as_bytes()],
        )
        .await;
        assert!(!format!("{failed:?}").contains("invalid"));
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        let response = failed.into_response(response_limits()).await.unwrap();
        assert_eq!(response.status(), 400);
        assert_eq!(response.headers()["accept-encoding"], "gzip");
        let bytes = drain(response.into_body()).await;
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["code"], "invalid_argument");
        assert_eq!(value["message"], expected);
        assert!(!bytes.ends_with(b"\n"));
        released(&pool).await;
    }
}

#[tokio::test]
async fn optional_version_uses_only_first_field_without_trimming_or_case_folding() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    for first in [None, Some(""), Some("1")] {
        let mut h = headers();
        if let Some(first) = first {
            h.append("connect-protocol-version", first.parse().unwrap());
            h.append(
                "connect-protocol-version",
                "invalid second".parse().unwrap(),
            );
        }
        let decoded = pool
            .admit()
            .unwrap()
            .authenticate(&session, h)
            .await
            .unwrap()
            .decode("Stat", Body::from("{}"))
            .await
            .ok()
            .unwrap();
        assert_eq!(decoded.request(), &Request::Stat(String::new()));
        drop(decoded);
    }
    for first in [b" 1".as_slice(), b"1 ", b"01", b"+1", b"1,1", b"\xff"] {
        let failed = failure(&pool, &session, &[first, b"1"], &[]).await;
        let Error::Version(error) = failed.error() else {
            panic!("wrong failure")
        };
        assert_eq!(error.value(), first);
        drop(failed);
    }
}

#[tokio::test]
async fn header_response_clones_encoding_failures_and_unmapped_errors_retain_admission() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    let failed = failure(&pool, &session, &[b"2"], &[]).await;
    let mut body = failed
        .into_response(response_limits())
        .await
        .unwrap()
        .into_body();
    let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = first.clone();
    drop(first);
    drop(body);
    full(&pool);
    drop(copy);
    released(&pool).await;
    let failed = failure(&pool, &session, &[], &[b"invalid"]).await;
    let end = failed.deadline();
    let failed = failed
        .into_response(file_rpc_response::Limits {
            wire_bytes: 1,
            ..response_limits()
        })
        .await
        .unwrap_err();
    assert_eq!(failed.error(), file_rpc_response::Error::ResponseLimit);
    assert_eq!(failed.deadline(), end);
    full(&pool);
    drop(failed);
    released(&pool).await;
    let failed = pool
        .admit()
        .unwrap()
        .authenticate(&session, headers())
        .await
        .unwrap()
        .decode("WatchDir", Body::from("{}"))
        .await
        .err()
        .unwrap();
    let failed = failed.into_response(response_limits()).await.unwrap_err();
    assert_eq!(failed.error(), file_rpc_response::Error::UnclassifiedFault);
    full(&pool);
    drop(failed);
    released(&pool).await;
}

#[tokio::test(start_paused = true)]
async fn valid_timeout_is_capped_before_version_failure_without_renewing_expiry() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    for millis in ["0", "1000"] {
        let started = Instant::now();
        let failed = failure(&pool, &session, &[b"2"], &[millis.as_bytes()]).await;
        assert!(matches!(failed.error(), Error::Version(_)));
        assert_eq!(
            failed.deadline(),
            started + Duration::from_millis(millis.parse().unwrap())
        );
        tokio::time::advance(Duration::from_secs(2)).await;
        let failed = failed.into_response(response_limits()).await.unwrap_err();
        assert_eq!(failed.error(), file_rpc_response::Error::DeadlineExceeded);
        full(&pool);
        drop(failed);
        released(&pool).await;
    }
}

#[tokio::test]
#[ignore = "explicit fixed Go Connect HTTP reference; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_version_timeout_error_status_headers_and_escaping_reference() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let session = session().await;
    let pool = FileRpcRequests::new(Limits {
        headers: HeaderLimits {
            bytes: 32768,
            fields: 16,
        },
        ..limits()
    })
    .unwrap();
    let invalid = vec![
        b"2".to_vec(),
        b"invalid".to_vec(),
        b"+".to_vec(),
        b"12345678901".to_vec(),
        "用户'<>&\"\\\u{2028}\u{200b}".as_bytes().to_vec(),
        b"a\tb".to_vec(),
        vec![0xff, b'X'],
        vec![b'x'; 8192],
    ];
    let mut cases = Vec::<(Vec<Vec<u8>>, Vec<Vec<u8>>)>::new();
    for value in invalid {
        cases.push((
            vec![value.clone(), b"1".to_vec()],
            vec![b"invalid".to_vec()],
        ));
        if value != b"2" {
            // Valid numeric timeout is only a bad version case.
            cases.push((vec![b"1".to_vec()], vec![value, b"1000".to_vec()]));
        }
    }
    for version in [vec![], vec![vec![], b"2".to_vec()]] {
        cases.push((version, vec![b"invalid".to_vec()]));
    }
    for byte in (b'!'..=b'~').chain(0x80..=0xff) {
        if !byte.is_ascii_digit() {
            cases.push((vec![b"1".to_vec()], vec![vec![byte]]));
        }
    }
    let count = cases.len();
    for (versions, timeouts) in cases {
        let failed = failure(
            &pool,
            &session,
            &versions.iter().map(Vec::as_slice).collect::<Vec<_>>(),
            &timeouts.iter().map(Vec::as_slice).collect::<Vec<_>>(),
        )
        .await;
        let response = failed.into_response(response_limits()).await.unwrap();
        let mut observed = vec![response.status().as_u16().to_string()];
        for field in [
            "content-type",
            "content-length",
            "accept-encoding",
            "content-encoding",
            "x-content-type-options",
            "vary",
        ] {
            observed.push(hex(response
                .headers()
                .get(field)
                .map_or(b"", |v| v.as_bytes())));
        }
        observed.push(hex(&drain(response.into_body()).await));
        let input = serde_json::json!({"version":versions.iter().map(|v| hex(v)).collect::<Vec<_>>(), "timeout":timeouts.iter().map(|v| hex(v)).collect::<Vec<_>>()});
        oracle.check(
            &format!("H\t{}", hex(input.to_string().as_bytes())),
            &observed.join("\t"),
        );
        released(&pool).await;
    }
    oracle.finish();
    eprintln!(
        "{count} fixed Connect version/timeout error HTTP cases; identity JSON, no full envd."
    );
}
