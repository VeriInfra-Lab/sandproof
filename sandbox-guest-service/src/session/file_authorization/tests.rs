use super::*;
use crate::file_authorization::rejected;
use crate::file_download::tests::reference::{Reference, hex};
use crate::{InitLimits, Metadata, ServiceError};
use http_body_util::BodyExt;
use sha2::Sha512;
use std::time::Duration;
use std::{
    future::{Future, poll_fn},
    pin::Pin,
    sync::{
        Mutex as StdMutex,
        atomic::{AtomicI64, Ordering},
    },
    task::Poll,
};
use tokio::sync::Semaphore;

#[derive(Clone)]
struct Meta(Arc<StdMutex<String>>);
impl MetadataSource for Meta {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        Ok(Metadata {
            instance_id: "vm".into(),
            template_id: "tpl".into(),
            access_token_sha512: Sha512::digest(self.0.lock().unwrap().as_bytes()).into(),
        })
    }
}
fn make() -> (Session<Meta>, Meta) {
    let source = Meta(Arc::new(StdMutex::new("token".into())));
    (
        Session::new(
            source.clone(),
            InitLimits {
                body_bytes: 4096,
                token_bytes: 128,
                state_bytes: 1024,
                env_entries: 16,
                in_flight: 2,
                timeout: Duration::from_secs(5),
            },
        )
        .unwrap(),
        source,
    )
}
async fn session() -> Session<Meta> {
    let (s, _) = make();
    s.initialize(br#"{"accessToken":"token"}"#).await.unwrap();
    s
}
fn limits() -> Limits {
    Limits {
        headers: crate::file_upload::HeaderLimits {
            bytes: 4096,
            fields: 16,
        },
        query_bytes: 4096,
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
fn query() -> Query {
    Query {
        path: Some(b"/file".to_vec()),
        username: Some(b"app".to_vec()),
        signature: None,
        signature_expiration: Some(100),
    }
}
fn signed(token: &[u8], operation: Operation) -> Query {
    let mut q = query();
    q.signature = Some(signature(token, &q, operation).to_vec());
    q
}
fn headers(token: &[u8]) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        "x-access-token",
        axum::http::HeaderValue::from_bytes(token).unwrap(),
    );
    h
}
async fn pending<T>(mut future: Pin<&mut impl Future<Output = T>>) {
    assert!(
        poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx)))
            .await
            .is_pending()
    );
}

#[tokio::test]
async fn token_priority_and_empty_header_fallback_keep_original_validation_order() {
    let s = session().await;
    let mut q = query();
    q.signature = Some(vec![255]);
    q.signature_expiration = Some(-1);
    let a = s
        .authorize_file_clock(
            &headers(b"token"),
            q,
            Operation::Read,
            limits(),
            (deadline(), || panic!("token must bypass signing/clock")),
        )
        .await
        .unwrap();
    assert_eq!(a.path(), Some("/file"));
    assert_eq!(a.username(), Some("app"));
    assert_eq!(a.operation(), Operation::Read);
    assert_eq!(a.defaults().user, "root");
    drop(a);
    assert_eq!(
        s.authorize_file_clock(
            &headers(b"wrong"),
            signed(b"token", Operation::Read),
            Operation::Read,
            limits(),
            (deadline(), || panic!("bad token cannot fall back"))
        )
        .await
        .err(),
        Some(Error::Denied(Denial::TokenMismatch))
    );
    for h in [HeaderMap::new(), headers(b"")] {
        assert!(
            s.authorize_file_clock(
                &h,
                signed(b"token", Operation::Read),
                Operation::Read,
                limits(),
                (deadline(), || Ok(100))
            )
            .await
            .is_ok()
        );
    }
    assert_eq!(
        s.authorize_file_clock(
            &HeaderMap::new(),
            query(),
            Operation::Read,
            limits(),
            (deadline(), || panic!("no signature"))
        )
        .await
        .err(),
        Some(Error::Denied(Denial::MissingSignature))
    );
}

#[tokio::test]
async fn signature_binds_exact_query_operation_and_expiry_before_text_conversion() {
    let s = session().await;
    for change in 0..5 {
        let mut q = signed(b"token", Operation::Read);
        match change {
            0 => q.path = Some(b"/other".to_vec()),
            1 => q.username = None,
            2 => q.signature_expiration = None,
            3 => q.signature.as_mut().unwrap().push(b'='),
            _ => {
                q.signature.as_mut().unwrap()[0] = b'V';
            }
        }
        assert_eq!(
            s.authorize_file_clock(
                &HeaderMap::new(),
                q,
                Operation::Read,
                limits(),
                (deadline(), || panic!("signature first"))
            )
            .await
            .err(),
            Some(Error::Denied(Denial::InvalidSignature))
        );
    }
    assert_eq!(
        s.authorize_file_clock(
            &HeaderMap::new(),
            signed(b"token", Operation::Write),
            Operation::Read,
            limits(),
            (deadline(), || Ok(100))
        )
        .await
        .err(),
        Some(Error::Denied(Denial::InvalidSignature))
    );
    assert_eq!(
        s.authorize_file_clock(
            &HeaderMap::new(),
            signed(b"token", Operation::Read),
            Operation::Read,
            limits(),
            (deadline(), || Ok(101))
        )
        .await
        .err(),
        Some(Error::Denied(Denial::ExpiredSignature))
    );
    assert_eq!(
        s.authorize_file_clock(
            &HeaderMap::new(),
            signed(b"token", Operation::Read),
            Operation::Read,
            limits(),
            (deadline(), || Err(Error::ClockUnavailable))
        )
        .await
        .err(),
        Some(Error::ClockUnavailable)
    );
    let mut q = query();
    q.path = Some(vec![255]);
    q.signature = Some(signature(b"token", &q, Operation::Read).to_vec());
    assert_eq!(
        s.authorize_file_clock(
            &HeaderMap::new(),
            q,
            Operation::Read,
            limits(),
            (deadline(), || Ok(100))
        )
        .await
        .err(),
        Some(Error::UnsupportedText(
            sandproof_guest_runtime::file_query::Parameter::Path
        ))
    );
}

#[tokio::test]
async fn protected_pre_init_duplicates_and_all_retained_bytes_fail_closed() {
    let (s, _) = make();
    assert_eq!(
        s.authorize_file(
            &headers(b"token"),
            query(),
            Operation::Read,
            limits(),
            deadline()
        )
        .await
        .err(),
        Some(Error::Denied(Denial::Uninitialized))
    );
    s.initialize(br#"{"accessToken":"token"}"#).await.unwrap();
    let mut h = headers(b"token");
    h.append("x-access-token", "token".parse().unwrap());
    assert_eq!(
        s.authorize_file(&h, query(), Operation::Read, limits(), deadline())
            .await
            .err(),
        Some(Error::Denied(Denial::DuplicateToken))
    );
    let mut l = limits();
    l.headers.fields = 1;
    assert_eq!(
        s.authorize_file(&h, query(), Operation::Read, l, deadline())
            .await
            .err(),
        Some(Error::ResourceExhausted)
    );
    let mut l = limits();
    l.query_bytes = 7;
    assert_eq!(
        s.authorize_file(&headers(b"token"), query(), Operation::Read, l, deadline())
            .await
            .err(),
        Some(Error::ResourceExhausted)
    );
    l.query_bytes = 8;
    assert!(
        s.authorize_file(&headers(b"token"), query(), Operation::Read, l, deadline())
            .await
            .is_ok()
    );
    l.query_bytes = 0;
    assert_eq!(
        s.authorize_file(&headers(b"token"), query(), Operation::Read, l, deadline())
            .await
            .err(),
        Some(Error::InvalidConfiguration)
    );
    assert_eq!(
        s.authorize_file(
            &headers(&[255]),
            query(),
            Operation::Read,
            limits(),
            deadline()
        )
        .await
        .err(),
        Some(Error::Denied(Denial::TokenMismatch))
    );
    assert_eq!(
        s.authorize_file(
            &headers(b"token"),
            query(),
            Operation::Read,
            limits(),
            Instant::now()
        )
        .await
        .err(),
        Some(Error::DeadlineExceeded)
    );
}

#[tokio::test]
async fn failed_rotation_keeps_original_signing_secret_and_restart_grants_nothing() {
    let (s, source) = make();
    s.initialize(br#"{"accessToken":"token"}"#).await.unwrap();
    *source.0.lock().unwrap() = "new".into();
    assert_eq!(
        s.initialize(br#"{"accessToken":"new","envVars":{"BAD=KEY":"x"}}"#)
            .await,
        Err(ServiceError::InvalidArgument)
    );
    assert!(
        s.authorize_file_clock(
            &HeaderMap::new(),
            signed(b"token", Operation::Read),
            Operation::Read,
            limits(),
            (deadline(), || Ok(100))
        )
        .await
        .is_ok()
    );
    assert_eq!(
        s.authorize_file_clock(
            &HeaderMap::new(),
            signed(b"new", Operation::Read),
            Operation::Read,
            limits(),
            (deadline(), || Ok(100))
        )
        .await
        .err(),
        Some(Error::Denied(Denial::InvalidSignature))
    );
    let (restarted, _) = make();
    assert_eq!(
        restarted
            .authorize_file_clock(
                &HeaderMap::new(),
                signed(b"token", Operation::Read),
                Operation::Read,
                limits(),
                (deadline(), || Ok(100))
            )
            .await
            .err(),
        Some(Error::Denied(Denial::Uninitialized))
    );
}

#[tokio::test]
async fn signed_permit_blocks_rotation_and_wall_clock_is_read_after_snapshot_wait() {
    let (s, source) = make();
    s.initialize(br#"{"accessToken":"token"}"#).await.unwrap();
    let original = s
        .authorize_file_clock(
            &HeaderMap::new(),
            signed(b"token", Operation::Read),
            Operation::Read,
            limits(),
            (deadline(), || Ok(100)),
        )
        .await
        .unwrap();
    *source.0.lock().unwrap() = "new".into();
    let mut rotation = Box::pin(s.initialize(br#"{"accessToken":"new","defaultUser":"changed"}"#));
    pending(rotation.as_mut()).await;
    let now = AtomicI64::new(99);
    let h = HeaderMap::new();
    let mut request = Box::pin(s.authorize_file_clock(
        &h,
        signed(b"new", Operation::Read),
        Operation::Read,
        limits(),
        (deadline(), || Ok(now.load(Ordering::SeqCst))),
    ));
    pending(request.as_mut()).await;
    assert_eq!(original.defaults().user, "root");
    now.store(101, Ordering::SeqCst);
    drop(original);
    rotation.await.unwrap();
    assert_eq!(
        request.await.err(),
        Some(Error::Denied(Denial::ExpiredSignature))
    );
    assert_eq!(
        s.authorize_file_clock(
            &h,
            signed(b"token", Operation::Read),
            Operation::Read,
            limits(),
            (deadline(), || Ok(100))
        )
        .await
        .err(),
        Some(Error::Denied(Denial::InvalidSignature))
    );
    let fresh = s
        .authorize_file_clock(
            &h,
            signed(b"new", Operation::Read),
            Operation::Read,
            limits(),
            (deadline(), || Ok(100)),
        )
        .await
        .unwrap();
    assert_eq!(fresh.defaults().user, "changed");
}

#[tokio::test(start_paused = true)]
async fn authorization_lock_wait_uses_original_deadline_and_cancel_does_not_pin_a_reader() {
    let (s, source) = make();
    s.initialize(br#"{"accessToken":"token"}"#).await.unwrap();
    let original = s.authorize(Some("token")).await.unwrap();
    *source.0.lock().unwrap() = "new".into();
    let mut rotation = Box::pin(s.initialize(br#"{"accessToken":"new"}"#));
    pending(rotation.as_mut()).await;
    let h = headers(b"new");
    let mut request = Box::pin(s.authorize_file(
        &h,
        query(),
        Operation::Read,
        limits(),
        Instant::now() + Duration::from_secs(1),
    ));
    pending(request.as_mut()).await;
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(request.await.err(), Some(Error::DeadlineExceeded));
    let mut cancelled =
        Box::pin(s.authorize_file(&h, query(), Operation::Read, limits(), deadline()));
    pending(cancelled.as_mut()).await;
    drop(cancelled);
    drop(original);
    rotation.await.unwrap();
    assert!(s.authorize(Some("new")).await.is_ok());
}

#[tokio::test]
async fn denial_reply_is_bounded_json_and_retains_capacity_until_last_byte_clone() {
    let slots = Arc::new(Semaphore::new(1));
    let owner = Arc::new(Arc::clone(&slots).try_acquire_owned().unwrap());
    let response = rejected(
        Denial::TokenMismatch,
        crate::upload_response::Limits {
            wire_bytes: 4096,
            chunk_bytes: 4096,
        },
        deadline(),
        owner,
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(
        response.headers()["content-type"],
        "application/json; charset=utf-8"
    );
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let mut body = response.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    assert_eq!(
        bytes.as_ref(),
        b"{\"code\":401,\"message\":\"access token present in header but does not match\"}\n"
    );
    let copy = bytes.clone();
    drop(bytes);
    drop(body);
    assert_eq!(slots.available_permits(), 0);
    drop(copy);
    let _slot = tokio::time::timeout(Duration::from_secs(5), slots.acquire())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rejected(
            Denial::MissingSignature,
            crate::upload_response::Limits {
                wire_bytes: 1,
                chunk_bytes: 1
            },
            deadline(),
            Arc::new(())
        )
        .await
        .err(),
        Some(crate::upload_response::Error::ResponseLimit)
    );
}

#[tokio::test]
#[ignore = "explicit fixed Go file signing oracle; set SANDPROOF_GUEST_AUTH_REFERENCE"]
async fn fixed_go_file_authorization_reference() {
    let s = session().await;
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_AUTH_REFERENCE");
    let mut count = 0;
    for operation in [Operation::Read, Operation::Write] {
        for path in [
            None,
            Some(""),
            Some("/a:+%"),
            Some("relative"),
            Some("é/💬"),
        ] {
            for user in [None, Some(""), Some("app"), Some("u:ser")] {
                for exp in [
                    None,
                    Some(i64::MIN),
                    Some(-1),
                    Some(0),
                    Some(100),
                    Some(101),
                    Some(i64::MAX),
                ] {
                    let mut q = Query {
                        path: path.map(|v| v.as_bytes().to_vec()),
                        username: user.map(|v| v.as_bytes().to_vec()),
                        signature: None,
                        signature_expiration: exp,
                    };
                    let sign = signature(b"token", &q, operation).to_vec();
                    let op = std::str::from_utf8(operation.name()).unwrap();
                    let user =
                        user.map_or_else(|| "-".into(), |u| format!("+{}", hex(u.as_bytes())));
                    let exp = exp.map_or_else(|| "-".into(), |v| v.to_string());
                    let prefix = format!(
                        "{}\t{}\t{user}\t{exp}\t{op}",
                        hex(b"token"),
                        hex(path.unwrap_or("").as_bytes())
                    );
                    oracle.check(
                        &format!("S\t{prefix}\t-\t100"),
                        std::str::from_utf8(&sign).unwrap(),
                    );
                    for mode in 0..4 {
                        let mut h = HeaderMap::new();
                        let extra = match mode {
                            1 => {
                                h = headers(b"token");
                                format!("\t{}", hex(b"token"))
                            }
                            2 => {
                                h = headers(b"wrong");
                                format!("\t{}", hex(b"wrong"))
                            }
                            _ => String::new(),
                        };
                        q.signature = Some(if mode == 3 {
                            b"bad".to_vec()
                        } else {
                            sign.clone()
                        });
                        let sig = format!("+{}", hex(q.signature.as_ref().unwrap()));
                        let candidate = Query {
                            path: q.path.clone(),
                            username: q.username.clone(),
                            signature: q.signature.clone(),
                            signature_expiration: q.signature_expiration,
                        };
                        let observed = match s
                            .authorize_file_clock(
                                &h,
                                candidate,
                                operation,
                                limits(),
                                (deadline(), || Ok(100)),
                            )
                            .await
                        {
                            Ok(_) => "OK".into(),
                            Err(Error::Denied(d)) => {
                                format!("ERR\t{}", hex(d.message().as_bytes()))
                            }
                            Err(e) => panic!("unmapped {e:?}"),
                        };
                        oracle.check(&format!("V\t{prefix}\t{sig}\t100{extra}"), &observed);
                        count += 1;
                    }
                }
            }
        }
    }
    oracle.finish();
    eprintln!(
        "Fixed Go file auth: {} signatures and {count} validation cases.",
        count / 4
    );
}
