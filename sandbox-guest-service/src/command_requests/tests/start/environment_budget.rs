use super::*;
use crate::command_requests::http::{Rpc, start::Rejection};
use axum::http::Method;
use sandproof_guest_runtime::protocol::StartLimits;

#[derive(Clone, Copy)]
struct Case {
    user: &'static str,
    basic: Option<&'static str>,
    cwd: Option<&'static str>,
    invalid_name: bool,
    over_args: bool,
    expected: ContextError,
    expansion: bool,
    lookups: usize,
}
fn cases() -> [Case; 8] {
    let base = Case {
        user: "alice",
        basic: None,
        cwd: None,
        invalid_name: false,
        over_args: false,
        expected: ContextError::ResourceExhausted,
        expansion: false,
        lookups: 1,
    };
    [
        base,
        Case {
            user: "missing",
            cwd: Some("~other"),
            invalid_name: true,
            expected: ContextError::Unauthenticated,
            ..base
        },
        Case {
            user: "missing:invalid",
            expected: ContextError::Unauthenticated,
            lookups: 0,
            ..base
        },
        Case {
            user: "missing",
            basic: Some("Basic YWxpY2U6"),
            ..base
        },
        Case {
            cwd: Some("~other"),
            expected: ContextError::InvalidArgument,
            expansion: true,
            ..base
        },
        Case {
            basic: Some("Basic YWxpY2U6"),
            cwd: Some("~other"),
            invalid_name: true,
            expected: ContextError::InvalidArgument,
            expansion: true,
            ..base
        },
        Case {
            invalid_name: true,
            expected: ContextError::InvalidArgument,
            ..base
        },
        Case {
            cwd: Some("~other"),
            invalid_name: true,
            over_args: true,
            ..base
        },
    ]
}
fn body(payload: &[u8], gzip: bool) -> Body {
    if !gzip {
        return framed(payload);
    }
    let compressed = crate::file_rpc_requests::tests::gzip::compressed(payload);
    let mut wire = vec![1];
    wire.extend(u32::try_from(compressed.len()).unwrap().to_be_bytes());
    wire.extend(compressed);
    Body::from(wire)
}

fn refusal(cwd: Option<&str>, invalid_name: bool, over_args: bool) -> Request {
    if over_args {
        return Request::StartArgumentsExceeded { limit: 32 };
    }
    Request::StartEnvironmentExceeded {
        limits: StartLimits {
            arguments: 32,
            env_entries: 32,
        },
        cwd: cwd.map(str::to_owned),
        invalid_name,
    }
}

#[tokio::test]
async fn oversized_environment_preserves_accounts_paths_errors_and_request_ownership() {
    let session = session().await;
    let mut request_limits = limits();
    request_limits.body.total_bytes = 4096;
    request_limits.body.frame_bytes = 4096;
    let pool = CommandRequests::new(request_limits).unwrap();
    let fixture = StartFixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let accounts = AccountPool::new(Accounts(calls.clone()), 1, Duration::from_secs(30)).unwrap();
    let mut envs: std::collections::BTreeMap<String, String> =
        (0..64).map(|i| (format!("k{i}"), String::new())).collect();
    for gzip in [false, true] {
        for Case {
            user,
            basic,
            cwd,
            invalid_name,
            over_args,
            expected,
            expansion,
            lookups,
        } in cases()
        {
            session
                .initialize(
                    &serde_json::to_vec(&serde_json::json!({
                        "accessToken":"token", "defaultUser":user
                    }))
                    .unwrap(),
                )
                .await
                .unwrap();
            if invalid_name {
                envs.insert("z=invalid".into(), String::new());
            } else {
                envs.remove("z=invalid");
            }
            let payload = serde_json::to_vec(&serde_json::json!({"process": {
                "cmd":"/must-not-run", "envs":envs, "cwd":cwd,
                "args":vec![""; if over_args { 33 } else { 0 }]
            }}))
            .unwrap();
            let mut headers = headers();
            headers.insert("content-type", "application/connect+json".parse().unwrap());
            if gzip {
                headers.insert("connect-content-encoding", "gzip".parse().unwrap());
            }
            if let Some(basic) = basic {
                headers.insert("authorization", basic.parse().unwrap());
            }
            let before = calls.load(Ordering::SeqCst);
            let selected = pool
                .admit()
                .unwrap()
                .authenticate_http(&session, &Method::POST, headers)
                .await
                .unwrap()
                .prepare_http(&Method::POST, &accounts, context_limits())
                .await
                .unwrap()
                .select_json(&Method::POST, Rpc::Start)
                .unwrap();
            let ready = selected.decode(body(&payload, gzip)).await.unwrap();
            assert_eq!(ready.request(), &refusal(cwd, invalid_name, over_args));
            assert_eq!(
                calls.load(Ordering::SeqCst) - before,
                usize::from(basic.is_some())
            );
            let failure = ready
                .start_pid(
                    &accounts,
                    &settings(),
                    fixture.preparation(),
                    response_limits(),
                )
                .await
                .err()
                .unwrap();
            assert_eq!(failure.error(), Error::Context(expected));
            assert_eq!(
                matches!(failure.start_rejection, Some(Rejection::Expansion(_))),
                expansion
            );
            assert_eq!(calls.load(Ordering::SeqCst) - before, lookups);
            assert_eq!(fixture.registry.available_slots(), 1);
            drop(fixture.owners.reserve().unwrap());
            full(&pool);
            drop(failure);
            released(&pool).await;
        }
    }
}

#[tokio::test]
async fn direct_decode_keeps_default_cwd_clock_and_exact_original_limit_checks() {
    let session = session().await;
    session
        .initialize(br#"{"accessToken":"token","defaultUser":"alice","defaultWorkdir":"~other"}"#)
        .await
        .unwrap();
    let mut request_limits = limits();
    request_limits.body.total_bytes = 4096;
    request_limits.body.frame_bytes = 4096;
    let pool = CommandRequests::new(request_limits).unwrap();
    let fixture = StartFixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let accounts = AccountPool::new(Accounts(calls.clone()), 1, Duration::from_secs(30)).unwrap();
    let envs: std::collections::BTreeMap<String, String> =
        (0..33).map(|i| (format!("k{i}"), String::new())).collect();
    for (cwd, invalid_clock, changed_limits, expected, expansion) in [
        (
            None,
            false,
            false,
            Error::Context(ContextError::InvalidArgument),
            true,
        ),
        (
            Some(""),
            false,
            false,
            Error::Context(ContextError::InvalidArgument),
            true,
        ),
        (
            Some("/explicit"),
            false,
            false,
            Error::Context(ContextError::ResourceExhausted),
            false,
        ),
        (
            None,
            true,
            false,
            Error::Command(crate::command::CommandError::InvalidArgument),
            false,
        ),
        (None, false, true, Error::UnsupportedOperation, false),
    ] {
        let payload = serde_json::to_vec(&serde_json::json!({"process": {
            "cmd":"/must-not-run", "envs":envs, "cwd":cwd
        }}))
        .unwrap();
        let mut headers = headers();
        if invalid_clock {
            headers.insert("connect-timeout-ms", "bad".parse().unwrap());
        }
        let ready = start_ready(&pool, &session, &accounts, headers, &payload).await;
        let mut settings = settings();
        if changed_limits {
            settings.limits.env_entries += 1;
        }
        let before = calls.load(Ordering::SeqCst);
        let failure = ready
            .start_pid(
                &accounts,
                &settings,
                fixture.preparation(),
                response_limits(),
            )
            .await
            .err()
            .unwrap();
        assert_eq!(failure.error(), expected);
        assert_eq!(
            matches!(failure.start_rejection, Some(Rejection::Expansion(_))),
            expansion
        );
        assert_eq!(
            calls.load(Ordering::SeqCst) - before,
            usize::from(!changed_limits)
        );
        assert_eq!(fixture.registry.available_slots(), 1);
        drop(fixture.owners.reserve().unwrap());
        full(&pool);
        drop(failure);
        released(&pool).await;
    }
}
