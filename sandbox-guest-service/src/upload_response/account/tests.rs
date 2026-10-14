use super::*;
use crate::{
    file_download::tests::reference::{Reference, hex},
    file_download::{Download, IdentityLimits, IdentityRequest},
    upload_response::{self, Limits, Rejection},
};
use axum::{
    body::Bytes,
    http::{HeaderMap, HeaderValue, Method},
    response::Response,
};
use http_body_util::BodyExt;
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;

async fn response(
    method: Method,
    name: &str,
    limit: usize,
    deadline: Instant,
    owner: crate::owned_body::Owner,
) -> Result<Response, String> {
    let error = UnknownUser::new(name);
    if method == Method::POST {
        return upload_response::rejected(
            Rejection::UnknownUser(error),
            Limits {
                wire_bytes: limit,
                chunk_bytes: 37,
            },
            deadline,
            owner,
        )
        .await
        .map_err(|e| format!("{e:?}"));
    }
    Download::unknown_user(
        error,
        IdentityRequest {
            method,
            headers: HeaderMap::new(),
            response: HeaderMap::new(),
            content_type: HeaderValue::from_static("text/plain").into(),
            disposition_path: None,
            boundary: None,
        },
        IdentityLimits {
            plan: crate::file_download::Limits {
                request: sandproof_guest_runtime::file_download::Limits {
                    header_bytes: 8192,
                    items: 32,
                },
                response_header_bytes: 8192,
                wire_bytes: limit as u64,
            },
            chunk_bytes: 37,
            file_bytes: 8192,
            path_bytes: 4096,
        },
        deadline,
        owner,
    )
    .await
    .map(Download::into_response)
    .map_err(|e| format!("{e:?}"))
}
async fn collect(response: Response) -> Vec<u8> {
    let mut body = response.into_body();
    let mut out = Vec::new();
    while let Some(frame) = body.frame().await {
        let bytes = frame.unwrap().into_data().unwrap();
        assert!(bytes.len() <= 37);
        out.extend_from_slice(&bytes);
    }
    out
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}

#[tokio::test]
async fn missing_account_responses_bound_escaped_bytes_and_retain_original_owner() {
    let name = "雪'\"<&>\n\0\u{2028}";
    let full = collect(
        response(Method::POST, name, 8192, deadline(), Arc::new(()))
            .await
            .unwrap(),
    )
    .await;
    let value: serde_json::Value = serde_json::from_slice(&full).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"code":401,"message":format!("error looking up user '{name}': user: unknown user {name}")})
    );
    for method in [Method::GET, Method::HEAD, Method::POST] {
        assert!(
            response(
                method.clone(),
                name,
                full.len() - 1,
                deadline(),
                Arc::new(())
            )
            .await
            .unwrap_err()
            .contains("ResponseLimit")
        );
        let slots = Arc::new(tokio::sync::Semaphore::new(1));
        let owner = Arc::new(Arc::clone(&slots).try_acquire_owned().unwrap());
        let ready = response(method.clone(), name, full.len(), deadline(), owner)
            .await
            .unwrap();
        assert_eq!(ready.status(), 401);
        for absent in [
            "vary",
            "content-disposition",
            "last-modified",
            "content-encoding",
        ] {
            assert!(!ready.headers().contains_key(absent));
        }
        assert_eq!(
            ready.headers()["content-type"],
            "application/json; charset=utf-8"
        );
        assert_eq!(ready.headers()["content-length"], full.len().to_string());
        assert_eq!(slots.available_permits(), 0);
        if method == Method::HEAD {
            assert!(collect(ready).await.is_empty());
        } else {
            let mut body = ready.into_body();
            let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
            let retained: Bytes = bytes.clone();
            drop(bytes);
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(31)).await;
            assert!(body.frame().await.unwrap().is_err());
            tokio::time::resume();
            drop(body);
            assert_eq!(slots.available_permits(), 0);
            drop(retained);
        }
        let _released = tokio::time::timeout(Duration::from_secs(5), slots.acquire_owned())
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn fixed_go_missing_account_status_headers_body_and_head_match() {
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    let names = [
        String::new(),
        "missing".into(),
        "雪'\"<&>\n\r:\0\u{2028}\u{2029}".into(),
        // The name occurs twice: 71 + 2*n crosses Go's 2048-byte buffer.
        "x".repeat(987),
        "x".repeat(988),
        "x".repeat(989),
        "x".repeat(2048),
    ];
    let mut cases = 0;
    for name in names {
        for method in [Method::GET, Method::HEAD, Method::POST] {
            let ready = response(method.clone(), &name, 65536, deadline(), Arc::new(()))
                .await
                .unwrap();
            let mut output = vec![ready.status().as_u16().to_string()];
            for header in ["content-type", "content-length", "x-content-type-options"] {
                output.push(hex(ready
                    .headers()
                    .get(header)
                    .map_or(&[], HeaderValue::as_bytes)));
            }
            output.push(hex(&collect(ready).await));
            reference.check(
                &format!("U\t{method}\t{}", hex(name.as_bytes())),
                &output.join("\t"),
            );
            cases += 1;
        }
    }
    reference.finish();
    eprintln!("Fixed Go unknown account: {cases} status/header/body cases, including HEAD.");
}
