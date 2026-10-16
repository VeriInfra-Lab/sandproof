use super::*;
use std::io::Write;

// The guard unblocks the original private reader even if an assertion fails.
struct ReleaseFifo(std::fs::File);
impl Drop for ReleaseFifo {
    fn drop(&mut self) {
        let _ = self.0.write_all(b"done\n");
    }
}
async fn fifo(fixture: &Fixture, name: &str) -> ReleaseFifo {
    timeout(Duration::from_secs(5), async {
        loop {
            match rustix::fs::open(
                fixture.directory.path().join(name),
                rustix::fs::OFlags::WRONLY
                    | rustix::fs::OFlags::NONBLOCK
                    | rustix::fs::OFlags::NOFOLLOW,
                rustix::fs::Mode::empty(),
            ) {
                Ok(fd) => break ReleaseFifo(std::fs::File::from(fd)),
                Err(rustix::io::Errno::NXIO) => tokio::task::yield_now().await,
                Err(error) => panic!("private FIFO: {error}"),
            }
        }
    })
    .await
    .unwrap()
}
fn guard(pid: u32) -> KernelGuard {
    KernelGuard(
        pidfd_open(
            Pid::from_raw(i32::try_from(pid).unwrap()).unwrap(),
            PidfdFlags::empty(),
        )
        .unwrap(),
    )
}
async fn descendant(output: &mut BodyDataStream) -> KernelGuard {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\n") {
        let event = json_frame(&frame(output).await, 0);
        bytes.extend(
            STANDARD
                .decode(event["event"]["data"]["stdout"].as_str().unwrap())
                .unwrap(),
        );
        assert!(bytes.len() <= 32);
    }
    guard(std::str::from_utf8(&bytes).unwrap().trim().parse().unwrap())
}
async fn original_reaped(parent: &KernelGuard) {
    timeout(Duration::from_secs(5), async {
        loop {
            match waitid(
                WaitId::PidFd(parent.0.as_fd()),
                WaitIdOptions::EXITED | WaitIdOptions::NOWAIT | WaitIdOptions::NOHANG,
            ) {
                Err(rustix::io::Errno::CHILD) => break,
                Ok(_) => tokio::task::yield_now().await,
                Err(error) => panic!("original pidfd wait observation: {error}"),
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn reaped_original_pidfd_signal_does_not_target_descendant_holding_output() {
    for signal in ["SIGNAL_SIGTERM", "SIGNAL_SIGKILL"] {
        reaped_signal(signal).await;
    }
}

async fn reaped_signal(signal: &str) {
    let fixture = fixture().await;
    let registry = registry();
    let ordinary = requests(1);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    let mut output = http.handle(request("Start", &json!({"process":{
        "cmd":"/bin/sh", "args":["-c","mkfifo parent finish; ( read done < finish ) & printf '%d\n' \"$!\"; read done < parent; exit 0"]
    }}))).await.unwrap().into_body().into_data_stream();
    let command = started(&mut output, &registry).await;
    let pid = command.0.identity().pid;
    let parent = guard(pid);
    let child = descendant(&mut output).await;
    let finish = fifo(&fixture, "finish").await;
    drop(fifo(&fixture, "parent").await);
    // Observe ECHILD without reaping: the original owner already waited, but
    // the descendant's pipes still prevent End and keep the same record live.
    original_reaped(&parent).await;
    let invalid = http
        .handle(request(
            "SendSignal",
            &json!({"process":{"pid":pid},"signal":99}),
        ))
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::NOT_IMPLEMENTED);
    drop(bytes(invalid).await);
    assert!(command.0.status().completion.is_none());
    assert!(registry.live(pid).is_ok());
    let reply = http
        .handle(request(
            "SendSignal",
            &json!({"process":{"pid":pid},"signal":signal}),
        ))
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = bytes(reply).await;
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({"code":"internal","message":"error sending signal: os: process already finished"})
    );
    let last = bytes.clone();
    drop(bytes);
    full(&control);
    rotates(&fixture).await;
    assert_eq!(
        timeout(Duration::from_secs(5), command.0.wait())
            .await
            .unwrap()
            .unwrap()
            .code,
        0
    );
    // A failed native signal still cancels output waiting, as fixed Go
    // does. Main wait is real, while the descendant remains blocked/live.
    end(&mut output).await;
    let exit = tokio::io::unix::AsyncFd::new(child.0.as_fd()).unwrap();
    let mut ready = Box::pin(exit.readable());
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(ready.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(ready);
    drop(exit);
    drop(last);
    timeout(Duration::from_secs(5), async {
        loop {
            if control.admit().is_ok() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        command
            .0
            .signal(command.0.identity(), ProcessSignal::Terminate),
        Err(CommandError::Unavailable)
    );
    drop(finish);
    drop(output);
    clean(&fixture, &registry, command).await;
    // Retain the descendant's exact pidfd through its real exit too.
    let exit = tokio::io::unix::AsyncFd::new(child.0.as_fd()).unwrap();
    let _ready = timeout(Duration::from_secs(5), exit.readable())
        .await
        .unwrap()
        .unwrap();
    assert!(ordinary.admit().is_ok());
    assert!(control.admit().is_ok());
}

#[tokio::test]
async fn ignored_term_keeps_output_until_main_wait_and_does_not_invent_exit() {
    let fixture = fixture().await;
    let registry = registry();
    let ordinary = requests(1);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    let mut output = http.handle(request("Start", &json!({"process":{
        "cmd":"/bin/sh", "args":["-c","trap '' TERM; mkfifo next finish; printf ready; read done < next; printf after; read done < finish"]
    }}))).await.unwrap().into_body().into_data_stream();
    let command = started(&mut output, &registry).await;
    data(&mut output, "stdout", b"ready").await;
    let finish = fifo(&fixture, "finish");
    let reply = http
        .handle(request(
            "SendSignal",
            &json!({
                "process":{"pid":command.0.identity().pid},"signal":"SIGNAL_SIGTERM"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::OK);
    drop(bytes(reply).await);
    assert!(command.0.status().completion.is_none());
    drop(fifo(&fixture, "next").await);
    data(&mut output, "stdout", b"after").await;
    assert!(command.0.status().completion.is_none());
    drop(finish.await);
    end(&mut output).await;
    drop(output);
    clean(&fixture, &registry, command).await;
}
