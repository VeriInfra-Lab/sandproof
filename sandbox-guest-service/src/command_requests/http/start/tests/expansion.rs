use super::*;
use crate::upload_response::PathExpansion;

fn expansion_source(user: &str, gzip: bool) -> (Source, Arc<Semaphore>) {
    let (mut source, slots) = source("unused", gzip);
    source.rejection = Some(Rejection::Expansion(PathExpansion::new(user)));
    (source, slots)
}

#[tokio::test]
async fn expansion_reply_keeps_original_account_exact_budget_and_last_byte_owner() {
    let user = "private-'\"\\<>&中\n\u{2028}";
    for gzip in [false, true] {
        let (initial, _) = expansion_source(user, gzip);
        let bytes = drain(send(initial, limits()).await.unwrap().into_body()).await;
        let required = bytes.len().max(decoded(&bytes).len() + 5);
        for wire_bytes in [required - 1, required] {
            let (source, slots) = expansion_source(user, gzip);
            let result = send(
                source,
                reply::Limits {
                    wire_bytes,
                    ..limits()
                },
            )
            .await;
            if wire_bytes < required {
                let failure = result.unwrap_err();
                assert_eq!(failure.error, reply::Error::ResponseLimit);
                assert!(!format!("{failure:?}").contains("private-"));
                assert_eq!(slots.available_permits(), 0);
                drop(failure);
            } else {
                let mut body = result.unwrap().into_body();
                let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&decoded(&first)).unwrap(),
                    serde_json::json!({"error":{"code":"invalid_argument","message":format!("failed to expand path '' for user '{user}': cannot expand user-specific home dir")}})
                );
                let last = first.clone();
                drop(first);
                drop(body);
                assert_eq!(slots.available_permits(), 0);
                drop(last);
            }
            released(slots).await;
        }
        let (mut source, slots) = expansion_source(user, gzip);
        source.deadline = Instant::now();
        let failure = send(source, limits()).await.unwrap_err();
        assert_eq!(failure.error, reply::Error::DeadlineExceeded);
        assert_eq!(slots.available_permits(), 0);
        drop(failure);
        released(slots).await;
    }
}

#[cfg(target_os = "linux")]
#[test]
fn observed_facts_require_the_original_matching_failure() {
    use crate::command::{CommandError, RegistrationError};
    let cases = [
        (
            Rejection::Expansion(PathExpansion::new("alice")),
            RequestError::Context(crate::ContextError::InvalidArgument),
        ),
        (
            Rejection::MissingCwd("/missing".into()),
            RequestError::Registration(RegistrationError::Preparation(
                CommandError::InvalidArgument,
            )),
        ),
        (
            Rejection::Native(NativeFailure::new(
                sandproof_guest_runtime::protocol::StartRequest {
                    process: None,
                    pty: None,
                    tag: None,
                    stdin: None,
                },
                "/cwd".into(),
                20,
            )),
            RequestError::Command(CommandError::ExecFailed(20)),
        ),
    ];
    for (index, (fact, expected)) in cases.iter().enumerate() {
        assert!(fact.matches(expected));
        for (other, (_, error)) in cases.iter().enumerate() {
            assert_eq!(fact.matches(error), other == index);
        }
        for error in [
            RequestError::DeadlineExceeded,
            RequestError::Unavailable,
            RequestError::Command(CommandError::ExecFailed(13)),
        ] {
            assert!(!fact.matches(&error));
        }
    }
}

#[tokio::test]
#[ignore = "explicit fixed Go path expansion; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_start_expansion_matches_original_path_function_and_complete_reply() {
    use crate::file_download::tests::reference::{Reference, hex};
    use axum::http::HeaderValue;
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    for user in ["alice", "'\"\\<>&中\u{2028}", &"x".repeat(2100)] {
        for (cwd, fallback) in [
            ("~other/x", None),
            ("~人", Some("/ignored")),
            ("", Some("~other")),
        ] {
            for codec in [JsonContentType::Json, JsonContentType::Utf8] {
                for gzip in [false, true] {
                    let (mut source, slots) = expansion_source(user, gzip);
                    source.format = Some(ResponseFormat::new(
                        Rpc::Start,
                        codec,
                        if gzip {
                            Encoding::Gzip
                        } else {
                            Encoding::Identity
                        },
                    ));
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
                    let input = serde_json::json!({"Method":"Start","Kind":"expansion","User":user,"Cwd":cwd,"DefaultCwd":fallback,"Gzip":gzip,"UTF8":codec==JsonContentType::Utf8});
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
        "{count} fixed Go Start path expansion replies; original path function, no process handler execution."
    );
}
