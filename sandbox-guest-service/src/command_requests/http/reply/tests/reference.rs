use super::*;
use crate::file_download::tests::reference::{Reference, hex};
use std::collections::BTreeMap;

async fn check(oracle: &mut Reference, rpc: Rpc, fields: &BTreeMap<&'static str, Vec<String>>) {
    check_method(oracle, rpc, "POST", fields).await;
}
async fn check_method(
    oracle: &mut Reference,
    rpc: Rpc,
    method: &str,
    fields: &BTreeMap<&'static str, Vec<String>>,
) {
    let mut headers = HeaderMap::new();
    for (name, values) in fields {
        for value in values {
            headers.append(*name, value.parse().unwrap());
        }
    }
    let (error, end, format) = select(
        &headers,
        &method.parse().unwrap(),
        rpc,
        16384,
        Instant::now(),
        deadline(),
    )
    .unwrap_err();
    let response = send(error, format, limits(), end, Arc::new(()))
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
    let stream = response
        .headers()
        .get("content-type")
        .is_some_and(|v| v.as_bytes().starts_with(b"application/connect+"));
    let body = drain(response.into_body()).await;
    if stream {
        let size = u32::from_be_bytes(body[1..5].try_into().unwrap()) as usize;
        assert_eq!(body.len(), size + 5);
        let payload = if body[0] & 1 == 0 {
            body[5..].to_vec()
        } else {
            decoded(&body[5..])
        };
        observation.push(body[0].to_string());
        observation.push(hex(&payload));
    } else {
        observation.push(String::new());
        observation.push(hex(&body));
    }
    let input = serde_json::json!({"RPC": rpc.method(), "Method":method, "Headers": fields});
    oracle.check(
        &format!("W\t{}", hex(&serde_json::to_vec(&input).unwrap())),
        &observation.join("\t"),
    );
}

#[tokio::test]
#[ignore = "explicit fixed Go Connect command error bytes; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn emitted_header_failures_match_original_connect_status_headers_flags_and_json_bytes() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    for rpc in [
        Rpc::Start,
        Rpc::Connect,
        Rpc::List,
        Rpc::SendInput,
        Rpc::CloseStdin,
        Rpc::SendSignal,
        Rpc::Update,
    ] {
        let (base, sent, accept) = if rpc.is_stream() {
            (
                "application/connect+json",
                "connect-content-encoding",
                "connect-accept-encoding",
            )
        } else {
            ("application/json", "content-encoding", "accept-encoding")
        };
        for charset in ["", "; charset=utf-8"] {
            for encoding in ["identity", "gzip"] {
                for case in 0..7 {
                    let mut fields = BTreeMap::from([
                        ("content-type", vec![format!("{base}{charset}")]),
                        (accept, vec![encoding.into()]),
                    ]);
                    match case {
                        0 => {
                            fields.insert(sent, vec!["br".into(), "gzip".into()]);
                            fields.insert("connect-protocol-version", vec!["bad".into()]);
                            fields.insert("connect-timeout-ms", vec!["bad".into()]);
                        }
                        1 => {
                            fields.insert(
                                "connect-protocol-version",
                                vec!["\"\\中<&>\u{2028}".into()],
                            );
                        }
                        2 => {
                            fields.insert("connect-protocol-version", vec!["x".repeat(2100)]);
                        }
                        3 => {
                            fields.insert("connect-timeout-ms", vec!["bad".into()]);
                        }
                        4 => {
                            fields.insert("connect-timeout-ms", vec!["10000000000".into()]);
                        }
                        5 => {
                            fields.insert("connect-timeout-ms", vec!["+".into()]);
                        }
                        _ => {
                            fields.insert(sent, vec!["gzip".into()]);
                            fields
                                .insert("connect-protocol-version", vec!["bad".into(), "1".into()]);
                        }
                    }
                    check(&mut oracle, rpc, &fields).await;
                }
            }
        }
    }
    oracle.finish();
}

#[tokio::test]
#[ignore = "explicit fixed Go Connect command method replies; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn non_post_replies_match_original_before_codec_compression_version_and_timeout() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    for rpc in [
        Rpc::Start,
        Rpc::Connect,
        Rpc::List,
        Rpc::SendInput,
        Rpc::CloseStdin,
        Rpc::SendSignal,
        Rpc::Update,
    ] {
        for method in ["GET", "HEAD", "PUT", "DELETE", "PATCH", "OPTIONS"] {
            for content in ["application/json", "application/connect+json", "invalid"] {
                let fields = BTreeMap::from([
                    ("content-type", vec![content.into()]),
                    ("content-encoding", vec!["br".into()]),
                    ("connect-content-encoding", vec!["br".into()]),
                    ("connect-protocol-version", vec!["bad".into()]),
                    ("connect-timeout-ms", vec!["bad".into()]),
                ]);
                check_method(&mut oracle, rpc, method, &fields).await;
            }
        }
    }
    oracle.finish();
}
