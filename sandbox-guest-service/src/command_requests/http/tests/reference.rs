use super::*;
use crate::{
    connect_timeout::TimeoutReason,
    file_download::tests::reference::{Reference, hex},
};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    status: u16,
    content_type: String,
    encoding: &'static str,
    code: &'static str,
    message: String,
    allow: &'static str,
}
fn quoted(value: &[u8]) -> String {
    let mut bytes = Vec::new();
    crate::go_quote::quoted(value, &mut |chunk| {
        bytes.extend_from_slice(chunk);
        Ok(())
    })
    .unwrap();
    String::from_utf8(bytes).unwrap()
}
fn observation(
    result: Result<Plan, (Error, Instant, Option<ResponseFormat>)>,
    stream: bool,
) -> Observation {
    let mut seen = Observation {
        status: 200,
        content_type: String::new(),
        encoding: "",
        code: "",
        message: String::new(),
        allow: "",
    };
    match result {
        Ok(plan) => {
            seen.content_type = plan.content_type().into();
            if plan.response_encoding == Encoding::Gzip {
                seen.encoding = "gzip";
            }
        }
        Err((error, _, format)) => {
            if let Some(format) = format {
                seen.content_type = if stream {
                    format.content_type().into()
                } else {
                    "application/json".into()
                };
                if stream && format.encoding() == Encoding::Gzip {
                    seen.encoding = "gzip";
                }
            }
            match error {
                Error::Method => {
                    seen.status = 405;
                    seen.allow = "POST";
                }
                Error::ContentType => {
                    seen.status = 415;
                }
                Error::Compression(value) => {
                    if !stream {
                        seen.status = 501;
                    }
                    seen.code = "unimplemented";
                    seen.message = format!(
                        "unknown compression {}: supported encodings are gzip",
                        quoted(value.value())
                    );
                }
                Error::Version(value) => {
                    if !stream {
                        seen.status = 400;
                    }
                    seen.code = "invalid_argument";
                    seen.message = format!(
                        "Connect-Protocol-Version must be \"1\": got {}",
                        quoted(value.value())
                    );
                }
                Error::Timeout(value) => {
                    if !stream {
                        seen.status = 400;
                    }
                    seen.code = "invalid_argument";
                    seen.message = match value.reason() {
                        TimeoutReason::TooLong => {
                            format!("parse timeout: {} has >10 digits", quoted(value.value()))
                        }
                        TimeoutReason::Invalid => format!(
                            "parse timeout: strconv.ParseInt: parsing {}: invalid syntax",
                            quoted(value.value())
                        ),
                    };
                }
                error => panic!("not a header observation: {error:?}"),
            }
        }
    }
    seen
}
fn check(
    oracle: &mut Reference,
    rpc: Rpc,
    method: &str,
    fields: &BTreeMap<&'static str, Vec<&str>>,
) {
    let mut h = headers(rpc.is_stream());
    for (key, values) in fields {
        h.remove(*key);
        for value in values {
            h.append(*key, value.parse().unwrap());
        }
    }
    let now = Instant::now();
    let seen = observation(
        select(
            &h,
            &method.parse().unwrap(),
            rpc,
            4096,
            now,
            now + Duration::from_secs(30),
        ),
        rpc.is_stream(),
    );
    let input = serde_json::json!({"RPC":rpc.method(),"Method":method,"Headers":fields});
    oracle.check(
        &format!("V\t{}", hex(&serde_json::to_vec(&input).unwrap())),
        &serde_json::to_string(&seen).unwrap(),
    );
}

#[test]
#[ignore = "explicit fixed Go Connect command headers; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
fn selected_command_headers_match_original_connect_http_transport() {
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
        let (sent, accept, other) = if rpc.is_stream() {
            (
                "connect-content-encoding",
                "connect-accept-encoding",
                "content-encoding",
            )
        } else {
            (
                "content-encoding",
                "accept-encoding",
                "connect-content-encoding",
            )
        };
        check(&mut oracle, rpc, "POST", &BTreeMap::new());
        for method in ["GET", "HEAD", "PUT", "DELETE"] {
            check(
                &mut oracle,
                rpc,
                method,
                &BTreeMap::from([("content-type", vec!["invalid"]), (sent, vec!["br"])]),
            );
        }
        for value in [
            "",
            "identity",
            "gzip",
            "GZIP",
            "br",
            "gzip;q=0",
            "gzip,identity",
        ] {
            for bad in [false, true] {
                let mut h = BTreeMap::from([
                    (sent, vec![value, "br"]),
                    (accept, vec!["gzip"]),
                    (other, vec!["br"]),
                ]);
                if bad {
                    h.insert("connect-protocol-version", vec!["bad"]);
                    h.insert("connect-timeout-ms", vec!["bad"]);
                }
                check(&mut oracle, rpc, "POST", &h);
            }
        }
        for value in [
            "", "identity", "br gzip", "gzip,br", "gzip;q=1", "br\tgzip", "GZIP",
        ] {
            check(
                &mut oracle,
                rpc,
                "POST",
                &BTreeMap::from([(accept, vec![value, "gzip"])]),
            );
        }
        for value in ["", "1", "bad", "1,1", "\"\\中"] {
            check(
                &mut oracle,
                rpc,
                "POST",
                &BTreeMap::from([
                    ("connect-protocol-version", vec![value, "bad"]),
                    (accept, vec!["gzip"]),
                ]),
            );
        }
        for value in [
            "",
            "5000",
            "+5000",
            "9999999999",
            "bad",
            "+",
            "1_0",
            "10000000000",
        ] {
            check(
                &mut oracle,
                rpc,
                "POST",
                &BTreeMap::from([
                    ("connect-timeout-ms", vec![value, "bad"]),
                    (accept, vec!["gzip"]),
                ]),
            );
        }
        mime(&mut oracle, rpc);
    }
    oracle.finish();
}

fn mime(oracle: &mut Reference, rpc: Rpc) {
    let prefix = if rpc.is_stream() {
        "APPLICATION/CONNECT+JSON"
    } else {
        "APPLICATION/JSON"
    };
    for suffix in [
        "",
        "; CHARSET=\"UTF-8\"",
        ";charset*=utf-8''UTF%2D8",
        ";charset*0=ut;charset*1=f-8",
        ";ignored*=unknown''ignored",
        ";extra=x",
        ";charset=utf-8;extra=x",
        ";charset=UTF-8;charset=utf-8",
    ] {
        let content = format!("{prefix}{suffix}");
        check(
            oracle,
            rpc,
            "POST",
            &BTreeMap::from([("content-type", vec![content.as_str(), "invalid"])]),
        );
    }
}
