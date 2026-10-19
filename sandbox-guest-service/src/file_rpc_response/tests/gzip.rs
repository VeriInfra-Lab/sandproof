use super::*;
use sandproof_guest_runtime::gzip::{DecodeLimits, Decoder};

fn decoded(bytes: &[u8]) -> Vec<u8> {
    let mut decoder = Decoder::new(
        bytes,
        DecodeLimits {
            chunk_bytes: 13,
            encoded_bytes: 65536,
            decoded_bytes: 65536,
            header_bytes: 64,
            members: 1,
        },
    )
    .unwrap();
    let mut result = Vec::new();
    while let Some(chunk) = decoder.next_chunk().unwrap() {
        result.extend(chunk);
    }
    result
}

#[tokio::test]
async fn gzip_success_is_complete_for_every_shape_and_both_byte_budgets() {
    for records in [
        Records::Empty,
        Records::Many(vec![]),
        Records::One(entry()),
        Records::Many(vec![entry(), entry()]),
    ] {
        let (records, slots) = retain(records);
        let expected = file_protocol::encode_response(view(&records), 65536).unwrap();
        let response = send_with_encoding(records, view, limits(), deadline(), Encoding::Gzip)
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-encoding"], "gzip");
        assert_eq!(response.headers()["accept-encoding"], "gzip");
        let length = response.headers().get("content-length").cloned();
        let body = drain(response.into_body()).await;
        assert_eq!(length.unwrap(), body.len().to_string());
        assert_eq!(decoded(&body), expected);
        released(slots).await;
    }
    let (records, _) = retain(Records::Empty);
    let length = drain(
        send_with_encoding(records, view, limits(), deadline(), Encoding::Gzip)
            .await
            .unwrap()
            .into_body(),
    )
    .await
    .len();
    for limit in [length, length - 1, 1] {
        let (records, slots) = retain(Records::Empty);
        let end = deadline();
        let result = send_with_encoding(
            records,
            view,
            Limits {
                wire_bytes: limit,
                ..limits()
            },
            end,
            Encoding::Gzip,
        )
        .await;
        if limit == length {
            assert_eq!(decoded(&drain(result.unwrap().into_body()).await), b"{}");
        } else {
            let failure = result.unwrap_err();
            assert_eq!(failure.error(), Error::ResponseLimit);
            assert_eq!(failure.deadline(), end);
            assert_eq!(slots.available_permits(), 0);
            drop(failure);
        }
        released(slots).await;
    }
}

#[tokio::test]
async fn gzip_bytes_retain_capacity_after_cancellation_and_expiry() {
    for expire in [false, true] {
        let (records, slots) = retain(Records::One(entry()));
        let mut body = send_with_encoding(records, view, limits(), deadline(), Encoding::Gzip)
            .await
            .unwrap()
            .into_body();
        let copy = body
            .frame()
            .await
            .unwrap()
            .unwrap()
            .into_data()
            .unwrap()
            .clone();
        let mut next = Box::pin(body.frame());
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(next.as_mut().poll(cx).is_pending()))
                .await
        );
        drop(next);
        if expire {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(31)).await;
            assert!(body.frame().await.unwrap().is_err());
            tokio::time::resume();
        }
        drop(body);
        assert_eq!(slots.available_permits(), 0);
        drop(copy);
        released(slots).await;
    }
}

async fn semantic_response(response: Response) -> String {
    let mut result = vec![response.status().as_u16().to_string()];
    let gzip = response
        .headers()
        .get("content-encoding")
        .is_some_and(|v| v == "gzip");
    let length = response.headers().get("content-length").cloned();
    for field in [
        "content-type",
        "content-length",
        "accept-encoding",
        "content-encoding",
        "x-content-type-options",
        "vary",
    ] {
        let value = response
            .headers()
            .get(field)
            .map_or(b"".as_slice(), |v| v.as_bytes());
        result.push(hex(if field == "content-length" && !value.is_empty() {
            b"valid"
        } else {
            value
        }));
    }
    let body = drain(response.into_body()).await;
    if let Some(length) = length {
        assert_eq!(length, body.len().to_string());
    }
    result.push(hex(&if gzip { decoded(&body) } else { body }));
    result.join("\t")
}

#[tokio::test]
#[ignore = "explicit fixed Go Connect HTTP gzip reference; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_unary_gzip_negotiation_success_and_identity_errors() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut large = entry();
    let mut seed = 0x311_u32;
    let text: String = (0..12000)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            char::from(b'!' + u8::try_from((seed >> 24) % 90).unwrap())
        })
        .collect();
    large.metadata.insert("random".into(), text);
    let mut count = 0;
    for accept in [
        vec![],
        vec!["gzip"],
        vec!["br,gzip"],
        vec!["br gzip"],
        vec!["br\tgzip"],
        vec!["GZIP"],
        vec!["gzip;q=1"],
        vec!["identity"],
        vec!["identity", "gzip"],
        vec!["", "gzip"],
        vec!["\u{85}gzip"],
        vec!["gzip", "identity"],
    ] {
        let mut headers = axum::http::HeaderMap::new();
        for value in &accept {
            headers.append(
                "accept-encoding",
                axum::http::HeaderValue::from_bytes(value.as_bytes()).unwrap(),
            );
        }
        let encoding = crate::file_rpc_requests::response_encoding(&headers);
        for records in [
            Records::Empty,
            Records::Many(vec![]),
            Records::One(entry()),
            Records::One(large.clone()),
        ] {
            let (records, slots) = retain(records);
            let payload = file_protocol::encode_response(view(&records), 65536).unwrap();
            let response = send_with_encoding(records, view, limits(), deadline(), encoding)
                .await
                .unwrap();
            let input = serde_json::json!({"Accept":accept,"Data":hex(&payload)});
            oracle.check(
                &format!("G\t{}", hex(input.to_string().as_bytes())),
                &semantic_response(response).await,
            );
            released(slots).await;
            count += 1;
        }
        let response = rejected(
            Rejection::PathExpansion(crate::upload_response::PathExpansion::new("user")),
            limits(),
            deadline(),
            Arc::new(()),
        )
        .await
        .unwrap();
        let payload = serde_json::json!({"Name":"user","Path":"~other"});
        let input = serde_json::json!({"Accept":accept,"Data":hex(payload.to_string().as_bytes()),"Error":true});
        oracle.check(
            &format!("G\t{}", hex(input.to_string().as_bytes())),
            &semantic_response(response).await,
        );
        count += 1;
    }
    oracle.finish();
    eprintln!(
        "{count} fixed Connect gzip negotiation/status/header/decoded-payload cases; DEFLATE bytes are not compared."
    );
}
