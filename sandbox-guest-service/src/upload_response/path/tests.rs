use super::*;
use crate::{
    file_download::tests::reference::{Reference, hex},
    file_download::{Download, IdentityLimits, IdentityRequest},
    upload_response::{self, Limits, Rejection},
};
use axum::{
    http::{HeaderMap, HeaderValue, Method},
    response::Response,
};
use http_body_util::BodyExt;
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;

async fn response(
    method: Method,
    path: &str,
    name: &str,
    limit: usize,
) -> Result<Response, String> {
    let fault = PathExpansion::new(name);
    let deadline = Instant::now() + Duration::from_secs(30);
    if method == Method::POST {
        return upload_response::rejected(
            Rejection::PathExpansion(fault),
            Limits {
                wire_bytes: limit,
                chunk_bytes: 37,
            },
            deadline,
            Arc::new(()),
        )
        .await
        .map_err(|e| format!("{e:?}"));
    }
    Download::path_expansion(
        fault,
        path.into(),
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
            path_bytes: 8192,
        },
        deadline,
        Arc::new(()),
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

#[tokio::test]
async fn expansion_envelopes_keep_outer_query_inner_empty_path_and_exact_budget() {
    let name = "雪'\"<&>\u{2028}";
    for path in ["~other/never", "", "~'\"<&>\n\0"] {
        for method in [Method::GET, Method::POST] {
            let ready = response(method.clone(), path, name, 8192).await.unwrap();
            assert_eq!(ready.status(), 400);
            for absent in [
                "vary",
                "content-disposition",
                "last-modified",
                "content-encoding",
            ] {
                assert!(!ready.headers().contains_key(absent));
            }
            let bytes = collect(ready).await;
            let message = if method == Method::POST {
                format!(
                    "error resolving path: failed to expand path '' for user '{name}': cannot expand user-specific home dir"
                )
            } else {
                format!(
                    "error expanding and resolving path '{path}': failed to expand path '' for user '{name}': cannot expand user-specific home dir"
                )
            };
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
                serde_json::json!({"code":400,"message":message})
            );
            assert_eq!(
                collect(
                    response(method.clone(), path, name, bytes.len())
                        .await
                        .unwrap()
                )
                .await,
                bytes
            );
            assert!(
                response(method.clone(), path, name, bytes.len() - 1)
                    .await
                    .unwrap_err()
                    .contains("ResponseLimit")
            );
            if method == Method::GET {
                assert!(
                    response(Method::HEAD, path, name, bytes.len() - 1)
                        .await
                        .unwrap_err()
                        .contains("ResponseLimit")
                );
                let head = response(Method::HEAD, path, name, bytes.len())
                    .await
                    .unwrap();
                assert_eq!(head.headers()["content-length"], bytes.len().to_string());
                assert!(collect(head).await.is_empty());
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn fixed_go_expand_and_resolve_errors_match_http_bytes() {
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    let paths = [
        "~other".to_owned(),
        "~'\"<&>\n\0".into(),
        String::new(),
        format!("~{}", "x".repeat(2100)),
    ];
    let names = ["user", "雪'\"<&>\u{2028}", ""];
    let mut cases = 0;
    for path in paths {
        for name in names {
            for method in [Method::GET, Method::HEAD, Method::POST] {
                let ready = response(method.clone(), &path, name, 65536).await.unwrap();
                let mut output = vec![ready.status().as_u16().to_string()];
                for header in ["content-type", "content-length", "x-content-type-options"] {
                    output.push(hex(ready
                        .headers()
                        .get(header)
                        .map_or(&[], HeaderValue::as_bytes)));
                }
                output.push(hex(&collect(ready).await));
                reference.check(
                    &format!(
                        "X\t{method}\t{}\t{}\t{}",
                        hex(name.as_bytes()),
                        hex(path.as_bytes()),
                        hex(b"~default")
                    ),
                    &output.join("\t"),
                );
                cases += 1;
            }
        }
    }
    reference.finish();
    eprintln!("Fixed Go path expansion: {cases} status/header/body cases.");
}
