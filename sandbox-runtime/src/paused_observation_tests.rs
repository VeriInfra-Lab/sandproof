//! Controlled Unix peers/records and own processes. No real VMM or root setup;
//! private frames test the same state/stream/late-check helpers, not deployment.
use super::*;
use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::net::UnixListener,
    thread,
};

const ID: &str = "paused-read-test";
fn identity() -> RuntimeProcessIdentity {
    RuntimeProcessIdentity::capture(std::process::id()).unwrap()
}
fn frame(path: &Path, source: RuntimeProcessIdentity) -> OriginalVmSource {
    let pid = Pid::from_raw(i32::try_from(source.pid).unwrap()).unwrap();
    OriginalVmSource {
        resource_id: ID.into(),
        identity: source,
        root: File::open(path).unwrap(),
        _pidfd: pidfd_open(pid, PidfdFlags::empty()).unwrap(),
    }
}
fn reply(state: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "app_name":"Firecracker","id":ID,"state":state,"vmm_version":"1.16.1"
    }))
    .unwrap()
}
fn http(body: &[u8]) -> Vec<u8> {
    let mut value =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    value.extend_from_slice(body);
    value
}
fn serve(
    listener: UnixListener,
    response: Vec<u8>,
    before: impl FnOnce() + Send + 'static,
) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 128];
        while !request.windows(4).any(|v| v == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                return request;
            }
            request.extend_from_slice(&buffer[..count]);
            assert!(request.len() < 2048);
        }
        before();
        stream.write_all(&response).unwrap();
        request
    })
}
fn owners() -> (u32, u32) {
    (
        rustix::process::geteuid().as_raw(),
        rustix::process::getegid().as_raw(),
    )
}

#[test]
fn same_connected_process_reports_paused_or_running_with_only_a_get() {
    for state in ["Paused", "Running"] {
        let root = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(root.path().join("api.socket")).unwrap();
        let source = frame(root.path(), identity());
        let server = serve(listener, http(&reply(state)), || {});
        let (uid, gid) = owners();
        let observed = inspect(source, uid, gid, |_| Ok(false)).unwrap();
        assert!(matches!(
            (state, observed),
            ("Paused", OriginalPauseObservation::Paused(_))
                | ("Running", OriginalPauseObservation::Running)
        ));
        let request = server.join().unwrap();
        assert_eq!(
            request,
            b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}

#[test]
fn malformed_error_unknown_or_missing_replies_never_report_paused_or_exited() {
    let mut bodies=vec![reply("NotStarted"),b"{}".to_vec(),b"not-json".to_vec(),
        format!("{{\"app_name\":\"Firecracker\",\"id\":\"{ID}\",\"state\":\"Paused\",\"state\":\"Running\",\"vmm_version\":\"1.16.1\"}}").into_bytes()];
    let mut wrong: serde_json::Value = serde_json::from_slice(&reply("Paused")).unwrap();
    wrong["id"] = "different".into();
    bodies.push(serde_json::to_vec(&wrong).unwrap());
    let mut responses: Vec<_> = bodies.iter().map(|b| http(b)).collect();
    responses.extend([
        Vec::new(),
        b"HTTP/1.1 500 Failed\r\nContent-Length: 0\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\n{}".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\n\r\n{}".to_vec(),
    ]);
    for response in responses {
        let root = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(root.path().join("api.socket")).unwrap();
        let server = serve(listener, response, || {});
        let (uid, gid) = owners();
        assert!(inspect(frame(root.path(), identity()), uid, gid, |_| Ok(false)).is_err());
        server.join().unwrap();
    }
}

#[test]
fn late_record_replacement_corruption_or_loss_refuses_an_otherwise_paused_reply() {
    for change in 0..3 {
        let root = tempfile::tempdir().unwrap();
        let source = identity();
        let path = root.path().join("record.json");
        let record = ProcessRecord {
            resource_id: ID.into(),
            pid: source.pid,
            start_time_ticks: source.start_time_ticks,
            boot_id: source.boot_id.clone(),
            launch_cpu_domain: vec![0],
        };
        fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        let after = path.clone();
        let listener = UnixListener::bind(root.path().join("api.socket")).unwrap();
        let server = serve(listener, http(&reply("Paused")), move || match change {
            0 => {
                let mut record = record;
                record.start_time_ticks += 1;
                fs::write(after, serde_json::to_vec(&record).unwrap()).unwrap();
            }
            1 => fs::write(after, b"partial").unwrap(),
            _ => fs::remove_file(after).unwrap(),
        });
        let (uid, gid) = owners();
        assert!(
            inspect(frame(root.path(), source.clone()), uid, gid, |_| {
                let record: ProcessRecord = serde_json::from_slice(&fs::read(&path)?)?;
                require_record(&record, ID, &source)?;
                Ok(false)
            })
            .is_err()
        );
        server.join().unwrap();
    }
}

#[test]
fn lookup_uses_the_retained_directory_not_its_replaced_path() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("original");
    fs::create_dir(&path).unwrap();
    let listener = UnixListener::bind(path.join("api.socket")).unwrap();
    let source = frame(&path, identity());
    fs::rename(&path, root.path().join("held")).unwrap();
    fs::create_dir(&path).unwrap();
    let replacement = UnixListener::bind(path.join("api.socket")).unwrap();
    replacement.set_nonblocking(true).unwrap();
    let server = serve(listener, http(&reply("Running")), || {});
    let (uid, gid) = owners();
    assert_eq!(query(&source, uid, gid).unwrap(), State::Running);
    server.join().unwrap();
    assert_eq!(
        replacement.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn wrong_uid_or_gid_and_a_different_process_endpoint_are_refused_before_get() {
    use std::process::{Command, Stdio};
    let _serial = super::super::super::PROCESS_TEST_LOCK.lock().unwrap();
    let mut child = crate::OwnedChild::spawn(
        &mut Command::new("/bin/sh")
            .args(["-c", "IFS= read -r finish", "peer-source", "--id", ID])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    )
    .unwrap();
    let other = RuntimeProcessIdentity::capture(child.id()).unwrap();
    let (uid, gid) = owners();
    for variant in 0..3 {
        let root = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(root.path().join("api.socket")).unwrap();
        let server = serve(listener, http(&reply("Paused")), || {});
        let (source, expected_uid, expected_gid) = match variant {
            0 => (identity(), uid.checked_add(1).unwrap(), gid),
            1 => (identity(), uid, gid.checked_add(1).unwrap()),
            _ => (other.clone(), uid, gid),
        };
        assert!(matches!(
            query(&frame(root.path(), source), expected_uid, expected_gid),
            Err(RuntimeError::IdentityMismatch(_))
        ));
        assert!(server.join().unwrap().is_empty());
        assert!(child.try_wait().unwrap().is_none());
    }
    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn live_pidfd_with_missing_proc_or_wrong_start_is_not_execution_exit() {
    let source = identity();
    let pid = Pid::from_raw(i32::try_from(source.pid).unwrap()).unwrap();
    let fd = pidfd_open(pid, PidfdFlags::empty()).unwrap();
    assert!(matches!(
        known_exit(&fd, None, &source),
        Err(RuntimeError::Protocol(_))
    ));
    let mut status = read_proc(source.pid).unwrap().unwrap();
    status.start_time_ticks += 1;
    assert!(matches!(
        known_exit(&fd, Some(status), &source),
        Err(RuntimeError::IdentityMismatch(_))
    ));
    assert!(!known_exit(&fd, read_proc(source.pid).unwrap(), &source).unwrap());
}

#[test]
fn silent_connected_peer_is_unknown_and_does_not_produce_a_paused_handle() {
    use std::sync::mpsc;
    let root = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(root.path().join("api.socket")).unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    struct FinishOnDrop(mpsc::Sender<()>);
    impl Drop for FinishOnDrop {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    let done = FinishOnDrop(done_tx);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut bytes = [0_u8; 128];
        stream.read(&mut bytes).unwrap();
        done_rx.recv().unwrap();
    });
    let (uid, gid) = owners();
    let result = query(&frame(root.path(), identity()), uid, gid);
    drop(done);
    server.join().unwrap();
    assert!(
        matches!(result,Err(RuntimeError::Io(ref error)) if matches!(error.kind(),std::io::ErrorKind::TimedOut|std::io::ErrorKind::WouldBlock))
    );
}

#[test]
fn original_pidfd_confirms_execution_exit_after_exact_child_is_reaped() {
    use std::process::{Command, Stdio};
    let _serial = super::super::super::PROCESS_TEST_LOCK.lock().unwrap();
    let mut child = crate::OwnedChild::spawn(
        &mut Command::new("/bin/sh")
            .args(["-c", "IFS= read -r finish", "exit-source", "--id", ID])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    )
    .unwrap();
    let source = RuntimeProcessIdentity::capture(child.id()).unwrap();
    let pid = Pid::from_raw(i32::try_from(source.pid).unwrap()).unwrap();
    let fd = pidfd_open(pid, PidfdFlags::empty()).unwrap();
    assert!(!known_exit(&fd, read_proc(source.pid).unwrap(), &source).unwrap());
    drop(child.stdin.take());
    child.wait().unwrap();
    assert!(known_exit(&fd, None, &source).unwrap());
}
