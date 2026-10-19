use super::*;
use axum::body::{BodyDataStream, Bytes};
use base64::Engine;
use futures_util::StreamExt;
use sandproof_guest_service::command::{CommandBodyError, CommandBodyLimits, CommandExit};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Semaphore;

#[path = "body_clock.rs"]
mod clock;

fn limits() -> CommandBodyLimits {
    CommandBodyLimits {
        message_bytes: 2048,
        keepalive: Duration::from_secs(10),
    }
}
fn body(subscription: CommandSubscription) -> BodyDataStream {
    subscription
        .into_body(
            Arc::new(()),
            limits(),
            Instant::now() + Duration::from_secs(30),
        )
        .unwrap()
        .into_data_stream()
}
fn json_frame(bytes: &[u8], flags: u8) -> Value {
    assert_eq!(bytes[0], flags);
    assert_eq!(
        u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize,
        bytes.len() - 5
    );
    serde_json::from_slice(&bytes[5..]).unwrap()
}
async fn frame(body: &mut BodyDataStream) -> Bytes {
    timeout(Duration::from_secs(5), body.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}
async fn pending<F: Future>(future: std::pin::Pin<&mut F>) {
    let mut future = future;
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx)))
            .await
            .is_pending()
    );
}
async fn started(
    script: &str,
    owners: &CommandOwners,
    directory: &Path,
) -> (StopOnDrop, CommandSubscription) {
    let pair = start(
        owners,
        directory,
        script,
        session().await.authorize(Some("token")).await.unwrap(),
        deadline(Duration::ZERO),
    )
    .await;
    pair.0.0.started().await.unwrap();
    pair
}

#[tokio::test]
async fn body_orders_binary_output_nonzero_end_and_independent_connect_finish() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let (command, subscription) = started(
        r"printf '\000\377A'; printf err >&2; exit 17",
        &owners,
        directory.path(),
    )
    .await;
    let mut response = body(subscription);
    assert_eq!(
        json_frame(&frame(&mut response).await, 0),
        json!({"event":{"start":{"pid":command.0.identity().pid}}})
    );
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    loop {
        let value = json_frame(&frame(&mut response).await, 0);
        if let Some(data) = value["event"].get("data") {
            for (name, output) in [("stdout", &mut stdout), ("stderr", &mut stderr)] {
                if let Some(encoded) = data[name].as_str() {
                    output.extend(
                        base64::engine::general_purpose::STANDARD
                            .decode(encoded)
                            .unwrap(),
                    );
                }
            }
        } else {
            assert_eq!(
                value,
                json!({"event":{"end":{"exitCode":17,"exited":true,"status":"exit status 17","error":"exit status 17"}}})
            );
            break;
        }
    }
    assert_eq!(stdout, [0, 255, b'A']);
    assert_eq!(stderr, b"err");
    assert_eq!(command.0.wait().await.unwrap().code, 17);
    assert_eq!(json_frame(&frame(&mut response).await, 2), json!({}));
    assert!(response.next().await.is_none());
    // A new connection returns Start and the original retained End, no replay.
    let mut reconnect = body(command.0.subscribe(command.0.identity()).unwrap());
    assert!(
        json_frame(&frame(&mut reconnect).await, 0)["event"]
            .get("start")
            .is_some()
    );
    assert_eq!(
        json_frame(&frame(&mut reconnect).await, 0)["event"]["end"]["exitCode"],
        17
    );
    assert_eq!(json_frame(&frame(&mut reconnect).await, 2), json!({}));
    assert!(reconnect.next().await.is_none());
}

#[tokio::test]
async fn cumulative_output_exceeds_message_budget_and_empty_input_is_not_close() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let (command, subscription) = started("exec /bin/cat", &owners, directory.path()).await;
    let mut response = body(subscription);
    drop(frame(&mut response).await);
    command
        .0
        .write_input(command.0.identity(), b"")
        .await
        .unwrap();
    for sequence in 0..16 {
        let input: Vec<_> = (0..512)
            .map(|n| u8::try_from((n + sequence) % 256).unwrap())
            .collect();
        command
            .0
            .write_input(command.0.identity(), &input)
            .await
            .unwrap();
        let mut output = Vec::new();
        while output.len() < input.len() {
            let value = json_frame(&frame(&mut response).await, 0);
            output.extend(
                base64::engine::general_purpose::STANDARD
                    .decode(value["event"]["data"]["stdout"].as_str().unwrap())
                    .unwrap(),
            );
        }
        assert_eq!(output, input);
    }
    command.0.close_input(command.0.identity()).unwrap();
    assert_eq!(
        json_frame(&frame(&mut response).await, 0),
        json!({"event":{"end":{"exited":true,"status":"exit status 0"}}})
    );
    assert_eq!(json_frame(&frame(&mut response).await, 2), json!({}));
    assert!(response.next().await.is_none());
    assert_eq!(command.0.wait().await.unwrap().code, 0);
}

#[tokio::test]
async fn held_bytes_block_read_ahead_and_keep_request_subscriber_and_record_charged() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(OwnerLimits {
        subscribers: 1,
        ..budgets()
    })
    .unwrap();
    let (command, subscription) = started("exec /bin/cat", &owners, directory.path()).await;
    let requests = Arc::new(Semaphore::new(1));
    let request = Arc::new(requests.clone().try_acquire_owned().unwrap());
    let mut response = subscription
        .into_body(request, limits(), Instant::now() + Duration::from_secs(30))
        .unwrap()
        .into_data_stream();
    let start = frame(&mut response).await;
    let held = start.clone();
    drop(start);
    let mut next = Box::pin(response.next());
    pending(next.as_mut()).await;
    drop(next);
    drop(held);
    command
        .0
        .write_input(command.0.identity(), b"owned")
        .await
        .unwrap();
    let output = frame(&mut response).await;
    let retained = output.clone();
    drop(output);
    drop(response);
    assert_eq!(requests.available_permits(), 0);
    assert!(matches!(
        command.0.subscribe(command.0.identity()),
        Err(CommandError::ResourceExhausted)
    ));
    command.0.close_input(command.0.identity()).unwrap();
    command.0.wait().await.unwrap();
    drop(command);
    assert!(matches!(
        owners.reserve(),
        Err(CommandError::ResourceExhausted)
    ));
    assert_eq!(
        json_frame(&retained, 0),
        json!({"event":{"data":{"stdout":"b3duZWQ="}}})
    );
    drop(retained);
    assert_eq!(requests.available_permits(), 1);
    assert!(owners.reserve().is_ok());
}

#[tokio::test]
async fn keepalive_and_held_frame_expiry_do_not_stop_command_or_close_input() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let (command, subscription) = started("exec /bin/cat", &owners, directory.path()).await;
    tokio::time::pause();
    let expires = Instant::now() + Duration::from_secs(20);
    let mut response = subscription
        .into_body(Arc::new(()), limits(), expires)
        .unwrap()
        .into_data_stream();
    drop(frame(&mut response).await);
    let mut next = Box::pin(response.next());
    pending(next.as_mut()).await;
    tokio::time::advance(Duration::from_secs(9)).await;
    pending(next.as_mut()).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    let ping = next.await.unwrap().unwrap();
    assert_eq!(json_frame(&ping, 0), json!({"event":{"keepalive":{}}}));
    let mut next = Box::pin(response.next());
    pending(next.as_mut()).await;
    tokio::time::advance(Duration::from_secs(10)).await;
    assert!(next.await.unwrap().is_err());
    drop(ping);
    tokio::time::resume();
    assert!(command.0.status().completion.is_none());
    assert!(command.0.status().deadline_action.is_none());
    let mut reconnected = command.0.subscribe(command.0.identity()).unwrap();
    command
        .0
        .write_input(command.0.identity(), b"still-alive")
        .await
        .unwrap();
    assert_eq!(data(&mut reconnected).await, b"still-alive");
    command.0.close_input(command.0.identity()).unwrap();
    assert_eq!(command.0.wait().await.unwrap().code, 0);
}

#[tokio::test]
async fn lagged_stream_never_emits_a_successful_finish() {
    for message_bytes in [2048, 40] {
        let directory = tempfile::tempdir().unwrap();
        let owners = CommandOwners::new(OwnerLimits {
            output_bytes: 1,
            ..budgets()
        })
        .unwrap();
        let (command, subscription) = started("exec /bin/cat", &owners, directory.path()).await;
        let mut response = subscription
            .into_body(
                Arc::new(()),
                CommandBodyLimits {
                    message_bytes,
                    ..limits()
                },
                Instant::now() + Duration::from_secs(30),
            )
            .unwrap()
            .into_data_stream();
        drop(frame(&mut response).await);
        // The small byte budget guarantees eviction, independent of read chunking.
        command
            .0
            .write_input(command.0.identity(), b"x")
            .await
            .unwrap();
        let first = frame(&mut response).await;
        command
            .0
            .write_input(command.0.identity(), b"y")
            .await
            .unwrap();
        command.0.close_input(command.0.identity()).unwrap();
        command.0.wait().await.unwrap();
        drop(first); // held x consumes the sole subscriber byte until actual drop
        assert!(response.next().await.unwrap().is_err());
        assert!(response.next().await.is_none());
    }
}

#[tokio::test]
async fn oversized_frame_aborts_only_the_response_and_never_invents_end() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let (command, subscription) = started("exec /bin/cat", &owners, directory.path()).await;
    let mut response = subscription
        .into_body(
            Arc::new(()),
            CommandBodyLimits {
                message_bytes: 40,
                ..limits()
            },
            Instant::now() + Duration::from_secs(30),
        )
        .unwrap()
        .into_data_stream();
    drop(frame(&mut response).await);
    command
        .0
        .write_input(command.0.identity(), &[0; 512])
        .await
        .unwrap();
    let mut failed = false;
    while let Some(result) = timeout(Duration::from_secs(5), response.next())
        .await
        .unwrap()
    {
        match result {
            Err(_) => {
                failed = true;
                break;
            }
            Ok(bytes) => {
                assert!(json_frame(&bytes, 0)["event"].get("data").is_some());
            }
        }
    }
    assert!(failed);
    assert!(response.next().await.is_none());
    assert!(command.0.status().completion.is_none());
    command.0.close_input(command.0.identity()).unwrap();
    assert_eq!(command.0.wait().await.unwrap().code, 0);
}

#[test]
fn observed_exit_encoding_preserves_signal_core_flag_and_nonzero_failure() {
    for (signal, name) in [
        (6, "aborted"),
        (9, "killed"),
        (15, "terminated"),
        (31, "bad system call"),
        (64, "signal 64"),
    ] {
        for core_dumped in [false, true] {
            let status = format!(
                "signal: {name}{}",
                if core_dumped { " (core dumped)" } else { "" }
            );
            let exit = CommandExit {
                code: -1,
                signal: Some(signal),
                core_dumped,
            };
            let encoded = exit.encode_end(1024).unwrap();
            assert_eq!(
                json_frame(&encoded, 0),
                json!({"event":{"end":{"exitCode":-1,"status":status,"error":status}}})
            );
            assert!(exit.encode_end(encoded.len() - 6).is_err());
        }
    }
    let encoded = CommandExit {
        code: 255,
        signal: None,
        core_dumped: false,
    }
    .encode_end(1024)
    .unwrap();
    assert_eq!(
        json_frame(&encoded, 0),
        json!({"event":{"end":{"exitCode":255,"exited":true,"status":"exit status 255","error":"exit status 255"}}})
    );
}

#[tokio::test]
async fn invalid_limits_and_expired_body_do_not_take_over_execution() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let (command, original) = started("exec /bin/cat", &owners, directory.path()).await;
    drop(original);
    for invalid in [
        CommandBodyLimits {
            message_bytes: 0,
            ..limits()
        },
        CommandBodyLimits {
            message_bytes: usize::MAX,
            ..limits()
        },
        CommandBodyLimits {
            keepalive: Duration::ZERO,
            ..limits()
        },
        CommandBodyLimits {
            keepalive: Duration::MAX,
            ..limits()
        },
    ] {
        assert!(matches!(
            command
                .0
                .subscribe(command.0.identity())
                .unwrap()
                .into_body(
                    Arc::new(()),
                    invalid,
                    Instant::now() + Duration::from_secs(1)
                ),
            Err(CommandBodyError::InvalidArgument)
        ));
    }
    assert!(matches!(
        command
            .0
            .subscribe(command.0.identity())
            .unwrap()
            .into_body(Arc::new(()), limits(), Instant::now()),
        Err(CommandBodyError::DeadlineExceeded)
    ));
    command.0.close_input(command.0.identity()).unwrap();
    assert_eq!(command.0.wait().await.unwrap().code, 0);
}
