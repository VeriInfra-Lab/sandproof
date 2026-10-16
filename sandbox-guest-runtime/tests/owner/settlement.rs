use super::*;
use rustix::process::{Pid, PidfdFlags, WaitId, WaitIdOptions, pidfd_open, waitid};
use std::io::Write;
use std::os::fd::AsFd;

#[tokio::test]
async fn execution_expiry_settles_real_main_exit_while_descendant_keeps_output_open() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let process = prepare(
        directory.path(),
        "mkfifo finish; ( read done < finish ) & printf '%d\n' \"$!\"; read done; exit 0",
    )
    .await;
    let parent = kernel_guard(&process);
    let started_at = Instant::now();
    let expires_at = started_at + Duration::from_mins(1);
    let (command, mut stream) = owners.reserve().unwrap().start_registered(
        process,
        session().await.authorize(Some("token")).await.unwrap(),
        ExecutionDeadline::from_duration(started_at, Duration::from_mins(1)).unwrap(),
    );
    let command = StopOnDrop(command);
    command.0.started().await.unwrap();
    let mut pid = Vec::new();
    while !pid.ends_with(b"\n") {
        pid.extend(data(&mut stream).await);
        assert!(pid.len() <= 32);
    }
    let pid: i32 = std::str::from_utf8(&pid).unwrap().trim().parse().unwrap();
    let child = KernelGuard(pidfd_open(Pid::from_raw(pid).unwrap(), PidfdFlags::empty()).unwrap());
    let mut finish = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("finish"))
        .unwrap();
    command
        .0
        .write_input(command.0.identity(), b"done\n")
        .await
        .unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            match waitid(
                WaitId::PidFd(parent.0.as_fd()),
                WaitIdOptions::EXITED | WaitIdOptions::NOWAIT | WaitIdOptions::NOHANG,
            ) {
                Err(rustix::io::Errno::CHILD) => break,
                Ok(_) => tokio::task::yield_now().await,
                Err(error) => panic!("original wait observation: {error}"),
            }
        }
    })
    .await
    .unwrap();
    assert!(command.0.status().completion.is_none());
    assert!(command.0.status().deadline_action.is_none());
    tokio::time::pause();
    tokio::time::advance(expires_at.saturating_duration_since(Instant::now())).await;
    tokio::time::resume();
    let result = timeout(Duration::from_secs(5), command.0.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.code, 0);
    assert_eq!(result.signal, None);
    assert_eq!(
        command.0.status().deadline_action,
        Some(DeadlineAction::KillFailed)
    );
    assert!(matches!(stream.next().await, Some(Ok(OutputEvent::End(exit))) if exit == result));
    let exit = tokio::io::unix::AsyncFd::new(child.0.as_fd()).unwrap();
    let mut waiting = Box::pin(exit.readable());
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(waiting);
    finish.write_all(b"done\n").unwrap();
    let _ready = timeout(Duration::from_secs(5), exit.readable())
        .await
        .unwrap()
        .unwrap();
    drop(stream);
    drop(command);
    assert!(owners.reserve().is_ok());
}
