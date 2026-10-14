use super::*;

#[tokio::test]
async fn empty_method_reply_retains_capacity_until_eof_drop_or_original_expiry() {
    for head in [false, true] {
        for end in ["eof", "drop", "expiry"] {
            let (records, slots) = retain(Records::Empty);
            let deadline = deadline();
            let response = method_not_allowed(
                head,
                Limits {
                    wire_bytes: 1,
                    chunk_bytes: 1,
                },
                deadline,
                records,
            )
            .await
            .unwrap();
            assert_eq!(response.status(), 405);
            assert_eq!(response.headers()["allow"], "POST");
            assert_eq!(response.headers().len(), if head { 1 } else { 2 });
            if !head {
                assert_eq!(response.headers()["content-length"], "0");
            }
            assert_eq!(slots.available_permits(), 0);
            let mut body = response.into_body();
            match end {
                "eof" => assert!(body.frame().await.is_none()),
                "drop" => (),
                "expiry" => {
                    tokio::time::pause();
                    tokio::time::advance(deadline.saturating_duration_since(Instant::now())).await;
                    assert!(body.frame().await.unwrap().is_err());
                    tokio::time::resume();
                }
                _ => unreachable!(),
            }
            drop(body);
            released(slots).await;
        }
    }
}

#[tokio::test]
async fn method_reply_configuration_and_expiry_failures_retain_owner_and_deadline() {
    for (limits, deadline, error) in [
        (
            Limits {
                wire_bytes: 0,
                ..limits()
            },
            deadline(),
            Error::InvalidConfiguration,
        ),
        (
            Limits {
                chunk_bytes: 0,
                ..limits()
            },
            deadline(),
            Error::InvalidConfiguration,
        ),
        (limits(), Instant::now(), Error::DeadlineExceeded),
    ] {
        let (records, slots) = retain(Records::Empty);
        let failure = method_not_allowed(false, limits, deadline, records)
            .await
            .unwrap_err();
        assert_eq!(failure.error(), error);
        assert_eq!(failure.deadline(), deadline);
        assert_eq!(slots.available_permits(), 0);
        drop(failure);
        released(slots).await;
    }
}

#[tokio::test]
#[ignore = "explicit Go 1.26.8 / Connect v1.18.1 oracle; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn original_unary_http_method_rejections_match_status_headers_and_empty_body() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    for rpc in ["Stat", "MakeDir", "ListDir", "Move", "Remove"] {
        for method in [
            "GET", "HEAD", "PUT", "PATCH", "DELETE", "OPTIONS", "TRACE", "get", "post", "CUSTOM",
        ] {
            for types in [
                vec![],
                vec![""],
                vec!["application/json"],
                vec!["application/grpc"],
                vec!["not a type", "application/json"],
            ] {
                let (records, slots) = retain(Records::Empty);
                let response = method_not_allowed(method == "HEAD", limits(), deadline(), records)
                    .await
                    .unwrap();
                let mut observed = vec![response.status().as_u16().to_string()];
                for name in [
                    "allow",
                    "accept-post",
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
                        .map_or(b"", |value| value.as_bytes())));
                }
                let bytes = drain(response.into_body()).await;
                assert!(bytes.is_empty());
                observed.push(hex(&bytes));
                let input = serde_json::json!({"RPC": rpc, "Method": method, "ContentType": types});
                oracle.check(
                    &format!("M\t{}", hex(&serde_json::to_vec(&input).unwrap())),
                    &observed.join("\t"),
                );
                released(slots).await;
                count += 1;
            }
        }
    }
    oracle.finish();
    eprintln!(
        "{count} original unary method HTTP rejections; actual HEAD length omission, no normalization."
    );
}
