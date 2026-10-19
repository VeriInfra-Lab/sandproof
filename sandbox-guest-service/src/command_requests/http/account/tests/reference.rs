use super::*;
use crate::file_download::tests::reference::{Reference, hex};
use std::collections::BTreeMap;

async fn check(
    oracle: &mut Reference,
    method: &Method,
    user: &str,
    fields: &BTreeMap<&'static str, Vec<&str>>,
) {
    let mut headers = HeaderMap::new();
    for (name, values) in fields {
        for value in values {
            headers.append(*name, value.parse().unwrap());
        }
    }
    let response = send(
        Source {
            user: Some(UnknownUser::new(user)),
            request_error: RequestError::Unavailable,
            snapshot: Snapshot::capture(&headers, method, 4096),
            deadline: deadline(),
            _owner: Arc::new(()),
        },
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
            .map_or(b"", HeaderValue::as_bytes)));
    }
    let stream = response.headers()["content-type"]
        .as_bytes()
        .starts_with(b"application/connect+");
    let bytes = drain(response.into_body()).await;
    let payload = if stream {
        assert_eq!(bytes[0], 2);
        assert_eq!(
            u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize,
            bytes.len() - 5
        );
        observation.push("2".into());
        &bytes[5..]
    } else {
        observation.push(String::new());
        bytes.as_slice()
    };
    observation.push(hex(payload));
    let input = serde_json::json!({"Method":method.as_str(),"User":user,"Headers":fields});
    oracle.check(
        &format!("Y\t{}", hex(&serde_json::to_vec(&input).unwrap())),
        &observation.join("\t"),
    );
}

#[tokio::test]
#[ignore = "explicit fixed Go authn command replies; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn explicit_account_errors_match_original_authn_headers_and_json_bytes() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    for method in [Method::POST, Method::GET, Method::HEAD, Method::PUT] {
        for content in [
            "application/json",
            "application/connect+json",
            "APPLICATION/CONNECT+JSON; charset=UTF-8",
            "application/connect+json;extra=x",
            "application/connect+json;z=\"a b\";a=x",
            "application/connect+json;x*=utf-8''%E9%9B%AA",
            "application/connect+json;bad",
            "APPLICATION/CONNECT+JSON;bad",
            "invalid",
            "",
            "application/connect+json;charset*=utf-8''UTF%2D8",
            "application/connect+proto",
        ] {
            for version in ["", "1", "bad"] {
                let fields = BTreeMap::from([
                    ("content-type", vec![content, "invalid"]),
                    ("connect-protocol-version", vec![version, "bad"]),
                    ("connect-timeout-ms", vec!["bad"]),
                    ("connect-content-encoding", vec!["br"]),
                    ("connect-accept-encoding", vec!["gzip"]),
                    ("accept-encoding", vec!["gzip"]),
                ]);
                for user in ["absent", "", "\"\\<&>雪", "\0\r\n", &"x".repeat(2100)] {
                    check(&mut oracle, &method, user, &fields).await;
                }
            }
        }
    }
    oracle.finish();
}
