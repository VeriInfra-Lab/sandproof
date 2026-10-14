use super::*;
use axum::body::Body;
use http_body_util::BodyExt;
use std::time::Duration;
use tokio::sync::Semaphore;

mod reference;

fn limits() -> Limits {
    Limits {
        header_bytes: 4096,
        body: reply::Limits {
            wire_bytes: 16384,
            chunk_bytes: 13,
        },
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
fn source(
    method: &Method,
    content: &str,
    version: &str,
    name: Option<&str>,
    owner: Owner,
) -> Source {
    let h = HeaderMap::from_iter([
        ("content-type".parse().unwrap(), content.parse().unwrap()),
        (
            "connect-protocol-version".parse().unwrap(),
            version.parse().unwrap(),
        ),
    ]);
    Source {
        user: name.map(UnknownUser::new),
        request_error: RequestError::Unavailable,
        snapshot: Snapshot::capture(&h, method, 4096),
        deadline: deadline(),
        _owner: owner,
    }
}
fn retain() -> (Owner, Arc<Semaphore>) {
    let slots = Arc::new(Semaphore::new(1));
    (Arc::new(slots.clone().try_acquire_owned().unwrap()), slots)
}
async fn released(slots: Arc<Semaphore>) {
    let _permit = tokio::time::timeout(Duration::from_secs(5), slots.acquire_owned())
        .await
        .unwrap()
        .unwrap();
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut output = Vec::new();
    while let Some(frame) = body.frame().await {
        output.extend_from_slice(&frame.unwrap().into_data().unwrap());
    }
    output
}

#[tokio::test]
async fn middleware_status_and_canonical_type_do_not_reuse_handler_negotiation() {
    for method in [Method::POST, Method::GET, Method::HEAD, Method::PUT] {
        for content in [
            "application/json",
            "APPLICATION/CONNECT+JSON; charset=UTF-8",
            "application/connect+json;extra=x",
            "invalid",
        ] {
            for version in ["", "1", "bad"] {
                let response = send(
                    source(&method, content, version, Some("absent"), Arc::new(())),
                    limits(),
                )
                .await
                .unwrap();
                let stream = method == Method::POST
                    && content != "application/json"
                    && content != "invalid"
                    && version != "bad";
                assert_eq!(response.status(), if stream { 200 } else { 401 });
                for absent in [
                    "content-encoding",
                    "connect-content-encoding",
                    "accept-encoding",
                    "connect-accept-encoding",
                    "x-content-type-options",
                ] {
                    assert!(!response.headers().contains_key(absent));
                }
                let length: usize = response.headers()["content-length"]
                    .to_str()
                    .unwrap()
                    .parse()
                    .unwrap();
                let bytes = drain(response.into_body()).await;
                if method == Method::HEAD {
                    assert!(bytes.is_empty());
                    assert!(length > 0);
                    continue;
                }
                assert_eq!(bytes.len(), length);
                let payload = if stream {
                    assert_eq!(bytes[0], 2);
                    assert_eq!(
                        u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize,
                        bytes.len() - 5
                    );
                    &bytes[5..]
                } else {
                    bytes.as_slice()
                };
                let value: serde_json::Value = serde_json::from_slice(payload).unwrap();
                let value = if stream { &value["error"] } else { &value };
                assert_eq!(value["code"], "unauthenticated");
                assert_eq!(value["message"], "invalid username: 'absent'");
            }
        }
    }
}

#[tokio::test]
async fn dynamic_header_and_last_body_references_each_retain_original_capacity() {
    let (owner, slots) = retain();
    let response = send(
        source(
            &Method::POST,
            "application/connect+json;extra=x",
            "1",
            Some("absent"),
            owner,
        ),
        Limits {
            body: reply::Limits {
                chunk_bytes: 16384,
                ..limits().body
            },
            ..limits()
        },
    )
    .await
    .unwrap();
    let header = response.headers()["content-type"].clone();
    let mut body = response.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = bytes.clone();
    drop(body);
    drop(bytes);
    assert_eq!(slots.available_permits(), 0);
    drop(copy);
    assert_eq!(slots.available_permits(), 0);
    drop(header);
    released(slots).await;
}

#[tokio::test]
async fn absent_observation_protocol_gaps_limits_and_expiry_preserve_original_failure() {
    for case in 0..7 {
        let (owner, slots) = retain();
        let mut input = source(
            &Method::POST,
            if case == 1 {
                "application/grpc+json"
            } else {
                "application/connect+json"
            },
            "1",
            if case == 0 {
                None
            } else {
                Some("private-account")
            },
            owner,
        );
        let mut budget = limits();
        let expected = match case {
            0 | 1 => Error::UnclassifiedFault,
            2 => {
                budget.header_bytes = 0;
                Error::InvalidConfiguration
            }
            3 => {
                budget.header_bytes = 1;
                Error::ResponseLimit
            }
            4 => {
                budget.body.wire_bytes = 1;
                Error::ResponseLimit
            }
            5 => {
                input.deadline = Instant::now();
                Error::DeadlineExceeded
            }
            _ => {
                budget.body.chunk_bytes = 0;
                Error::InvalidConfiguration
            }
        };
        let original = input.deadline;
        let failure = send(input, budget).await.unwrap_err();
        assert_eq!(failure.error(), expected);
        assert_eq!(failure.deadline(), original);
        assert_eq!(failure.request_error(), RequestError::Unavailable);
        assert!(!format!("{failure:?}").contains("private-account"));
        assert_eq!(slots.available_permits(), 0);
        drop(failure);
        released(slots).await;
    }
}
