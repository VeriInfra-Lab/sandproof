use super::*;

#[tokio::test]
async fn missing_tag_preserves_the_exact_selector_in_unary_and_stream_errors() {
    for tag in [
        "",
        "123",
        "a'b\"\\<>&中\u{2028}",
        "\0\n\r",
        &"x".repeat(512),
    ] {
        for rpc in [Rpc::Connect, Rpc::SendSignal] {
            for gzip in [false, true] {
                for utf8 in [false, true] {
                    let selected = format(rpc, gzip, utf8);
                    let response = send(
                        source(
                            Some(Rejection::TagNotFound(tag.into())),
                            Some(selected),
                            Arc::new(()),
                        ),
                        limits(),
                    )
                    .await
                    .unwrap();
                    assert_eq!(response.status(), if rpc.is_stream() { 200 } else { 404 });
                    assert_eq!(
                        response.headers()["content-type"],
                        if rpc.is_stream() {
                            selected.content_type()
                        } else {
                            "application/json"
                        }
                    );
                    let bytes = drain(response.into_body()).await;
                    let raw = if rpc.is_stream() {
                        assert_eq!(bytes[0], if gzip { 3 } else { 2 });
                        assert_eq!(
                            u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize,
                            bytes.len() - 5
                        );
                        if gzip {
                            expand(&bytes[5..])
                        } else {
                            bytes[5..].to_vec()
                        }
                    } else {
                        bytes
                    };
                    let error = serde_json::json!({"code":"not_found",
                        "message":format!("process with tag {tag} not found")});
                    assert_eq!(
                        serde_json::from_slice::<serde_json::Value>(&raw).unwrap(),
                        if rpc.is_stream() {
                            serde_json::json!({"error":error})
                        } else {
                            error
                        }
                    );
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn tag_not_found_fact_cannot_render_other_registry_or_operation_failures() {
    use crate::command::{CommandError, RegistryError};
    let fact = Rejection::TagNotFound("original".into());
    assert!(fact.matches(&RequestError::Registry(RegistryError::NotFound)));
    for error in [
        RequestError::Registry(RegistryError::AlreadyExists),
        RequestError::Registry(RegistryError::NotReady),
        RequestError::Registry(RegistryError::Unavailable),
        RequestError::Command(CommandError::Unavailable),
        RequestError::DeadlineExceeded,
    ] {
        assert!(!fact.matches(&error));
    }
}

#[tokio::test]
#[ignore = "explicit fixed Go tag error rendering; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_missing_tag_status_headers_and_payload_match() {
    use axum::http::HeaderValue;
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    for tag in [
        "",
        "123",
        "a'b\"\\<>&中\u{2028}",
        "\0\n\r",
        &"x".repeat(512),
    ] {
        for rpc in [
            Rpc::Connect,
            Rpc::SendInput,
            Rpc::SendSignal,
            Rpc::CloseStdin,
            Rpc::Update,
        ] {
            for gzip in [false, true] {
                for utf8 in [false, true] {
                    let response = send(
                        source(
                            Some(Rejection::TagNotFound(tag.into())),
                            Some(format(rpc, gzip, utf8)),
                            Arc::new(()),
                        ),
                        limits(),
                    )
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
                            .map_or(b"", HeaderValue::as_bytes)));
                    }
                    let bytes = drain(response.into_body()).await;
                    if rpc.is_stream() {
                        observed.push(bytes[0].to_string());
                        observed.push(hex(&if gzip {
                            expand(&bytes[5..])
                        } else {
                            bytes[5..].to_vec()
                        }));
                    } else {
                        observed.push(String::new());
                        observed.push(hex(&bytes));
                    }
                    let input = serde_json::json!({"Method":rpc.method(),"Kind":"tag","Tag":tag,
                        "Gzip":gzip,"UTF8":utf8});
                    oracle.check(
                        &format!("R\t{}", hex(&serde_json::to_vec(&input).unwrap())),
                        &observed.join("\t"),
                    );
                    count += 1;
                }
            }
        }
    }
    oracle.finish();
    assert_eq!(count, 100);
    eprintln!(
        "{count} fixed Go missing-tag renderings; no live tag lookup or duplicate policy equivalence."
    );
}
