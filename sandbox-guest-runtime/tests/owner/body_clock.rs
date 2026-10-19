use super::*;

async fn ready(body: &mut BodyDataStream) -> Bytes {
    // Tokio's millisecond timer wheel may wake one millisecond after the exact
    // Instant. Virtual time may advance only this resolution, never a fresh
    // ten-second interval; the previous implementation still fails this check.
    timeout(Duration::from_millis(2), body.next())
        .await
        .expect("due keepalive postponed beyond timer resolution")
        .expect("missing keepalive")
        .expect("failed keepalive")
}
fn at_tick(expected: Instant) {
    let now = Instant::now();
    assert!(now >= expected);
    assert!(now.duration_since(expected) <= Duration::from_millis(2));
}
fn ping(bytes: &Bytes) {
    assert_eq!(json_frame(bytes, 0), json!({"event":{"keepalive":{}}}));
}
async fn idle(body: &mut BodyDataStream) {
    pending(Box::pin(body.next()).as_mut()).await;
}
async fn finish(command: &StopOnDrop) {
    tokio::time::resume();
    assert!(command.0.status().completion.is_none());
    assert!(command.0.status().deadline_action.is_none());
    let mut reconnected = command.0.subscribe(command.0.identity()).unwrap();
    command
        .0
        .write_input(command.0.identity(), b"still-open")
        .await
        .unwrap();
    assert_eq!(data(&mut reconnected).await, b"still-open");
    command.0.close_input(command.0.identity()).unwrap();
    assert_eq!(command.0.wait().await.unwrap().code, 0);
}

#[tokio::test]
async fn held_start_delayed_poll_and_keepalive_release_keep_one_periodic_schedule() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let (command, subscription) = started("exec /bin/cat", &owners, directory.path()).await;
    tokio::time::pause();
    let start = Instant::now();
    let mut body = subscription
        .into_body(Arc::new(()), limits(), start + Duration::from_secs(100))
        .unwrap()
        .into_data_stream();
    let first = frame(&mut body).await;
    let last = first.clone();
    drop(first);
    idle(&mut body).await;
    tokio::time::advance(Duration::from_secs(25)).await;
    idle(&mut body).await; // Start is still retained: timer has not begun.
    drop(last); // t=25: timer starts at release, not the next body poll.
    tokio::time::advance(Duration::from_secs(12)).await;
    let first_ping = ready(&mut body).await; // t=37; original tick was due at 35.
    ping(&first_ping);
    let last = first_ping.clone();
    drop(first_ping);
    tokio::time::advance(Duration::from_secs(14)).await;
    idle(&mut body).await; // t=51: retained keepalive still prevents any prefetch.
    drop(last);
    ping(&ready(&mut body).await); // Consume overdue 45 once; next is 55.
    idle(&mut body).await; // No burst of missed keepalive frames.
    tokio::time::advance(Duration::from_secs(3)).await;
    idle(&mut body).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    ping(&ready(&mut body).await); // t=55, not release+10=61.
    at_tick(start + Duration::from_secs(55));
    drop(body);
    finish(&command).await;
}

#[tokio::test]
async fn data_last_owner_restarts_clock_even_when_body_is_not_polled_at_release() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let (command, subscription) = started("exec /bin/cat", &owners, directory.path()).await;
    let mut body = subscription
        .into_body(
            Arc::new(()),
            limits(),
            Instant::now() + Duration::from_secs(100),
        )
        .unwrap()
        .into_data_stream();
    drop(frame(&mut body).await);
    command
        .0
        .write_input(command.0.identity(), b"data")
        .await
        .unwrap();
    let data = frame(&mut body).await;
    assert_eq!(
        json_frame(&data, 0),
        json!({"event":{"data":{"stdout":"ZGF0YQ=="}}})
    );
    let last = data.clone();
    drop(data);
    tokio::time::pause();
    let start = Instant::now();
    tokio::time::advance(Duration::from_secs(15)).await;
    idle(&mut body).await;
    drop(last); // Data release at 15 supersedes all old periodic ticks.
    tokio::time::advance(Duration::from_secs(9)).await;
    idle(&mut body).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    ping(&ready(&mut body).await); // t=25, independent of delayed poll at 24.
    at_tick(start + Duration::from_secs(25));
    drop(body);
    finish(&command).await;
}
