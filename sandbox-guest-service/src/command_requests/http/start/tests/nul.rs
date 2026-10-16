use super::*;
use sandproof_guest_runtime::protocol::{ProcessConfig, Pty, PtySize, StartRequest};

fn observed(pty: bool, environment: bool, gzip: bool) -> (Source, Arc<Semaphore>) {
    let (mut source, slots) = source("/cwd\0'中", gzip);
    source.rejection = Some(Rejection::Launch(LaunchFailure::new(
        StartRequest {
            process: Some(ProcessConfig {
                cmd: "cmd\0'\"<>&中".into(),
                args: vec!["a b".into(), String::new()],
                ..ProcessConfig::default()
            }),
            pty: pty.then_some(Pty {
                size: Some(PtySize {
                    cols: 65537,
                    rows: u32::MAX,
                }),
            }),
            tag: None,
            stdin: None,
        },
        "/cwd\0'中".into(),
        if environment {
            LaunchFailureKind::EnvironmentNul
        } else {
            LaunchFailureKind::ArgumentNul
        },
    )));
    (source, slots)
}

#[tokio::test]
async fn nul_validation_replies_keep_distinct_causes_and_last_byte_owner() {
    for pty in [false, true] {
        for environment in [false, true] {
            let (source, slots) = observed(pty, environment, false);
            let mut body = send(source, limits()).await.unwrap().into_body();
            let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
            let value: serde_json::Value = serde_json::from_slice(&decoded(&first)).unwrap();
            assert_eq!(value["error"]["code"], "invalid_argument");
            let message = value["error"]["message"].as_str().unwrap();
            assert!(message.contains("cmd\0'\"<>&中 a b "));
            assert_eq!(
                message.contains("with '65537' cols and '4294967295' rows"),
                pty
            );
            assert!(message.ends_with(if environment {
                ": exec: environment variable contains NUL"
            } else {
                ": fork/exec /bin/sh: invalid argument"
            }));
            let last = first.clone();
            drop(first);
            drop(body);
            assert_eq!(slots.available_permits(), 0);
            drop(last);
            released(slots).await;
        }
    }
}

#[tokio::test]
#[ignore = "explicit fixed Go NUL validation; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn actual_go_preexec_nul_failures_match_complete_stream_replies() {
    use crate::file_download::tests::reference::{Reference, hex};
    use axum::http::HeaderValue;
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    for pty in [false, true] {
        for environment in [false, true] {
            for codec in [JsonContentType::Json, JsonContentType::Utf8] {
                for gzip in [false, true] {
                    let (mut source, slots) = observed(pty, environment, gzip);
                    source.format = Some(ResponseFormat::new(
                        Rpc::Start,
                        codec,
                        if gzip {
                            Encoding::Gzip
                        } else {
                            Encoding::Identity
                        },
                    ));
                    let input = serde_json::json!({"Method":"Start","Kind":"nul","Cmd":"cmd\0'\"<>&中","Args":["a b",""],"Cwd":"/cwd\0'中","Env":if environment { serde_json::json!({"A":"x\0y"}) } else { serde_json::json!({}) },"Pty":pty,"Cols":65537,"Rows":u32::MAX,"Gzip":gzip,"UTF8":codec==JsonContentType::Utf8});
                    let response = send(source, limits()).await.unwrap();
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
                            .map_or(b"", HeaderValue::as_bytes)));
                    }
                    let bytes = drain(response.into_body()).await;
                    observed.push(bytes[0].to_string());
                    observed.push(hex(&decoded(&bytes)));
                    oracle.check(
                        &format!("R\t{}", hex(&serde_json::to_vec(&input).unwrap())),
                        &observed.join("\t"),
                    );
                    released(slots).await;
                    count += 1;
                }
            }
        }
    }
    oracle.finish();
    eprintln!(
        "{count} fixed Go preexec NUL validations and stream replies; no original handler/PTY or user exec."
    );
}
