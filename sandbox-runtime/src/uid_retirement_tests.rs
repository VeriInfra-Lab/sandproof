use super::*;

struct OwnedProcess(std::process::Child);
impl Drop for OwnedProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct OwnedThread {
    release: Option<std::sync::mpsc::Sender<()>>,
    join: Option<std::thread::JoinHandle<()>>,
}
impl Drop for OwnedThread {
    fn drop(&mut self) {
        let _ = self.release.take().unwrap().send(());
        self.join.take().unwrap().join().unwrap();
    }
}

fn observe(root: &Path, uid: u32) -> Result<(), RuntimeError> {
    let mut budget = Budget {
        deadline: Instant::now() + Duration::from_secs(2),
        entries: 0,
        limit: 100,
    };
    require_uid_unheld(root, uid, &mut budget)
}

fn status(root: &Path, pid: u32, tid: u32, bytes: &[u8]) {
    let task = root
        .join(pid.to_string())
        .join("task")
        .join(tid.to_string());
    fs::create_dir_all(&task).unwrap();
    fs::write(task.join("status"), bytes).unwrap();
}

#[test]
fn all_four_uid_roles_block_release_including_zombies() {
    let root = tempfile::tempdir().unwrap();
    for role in 0..4 {
        let mut values = [71; 4];
        values[role] = 93;
        let bytes = format!(
            "Name:\towned\nState:\tZ (zombie)\nUid:\t{}\t{}\t{}\t{}\n",
            values[0], values[1], values[2], values[3]
        );
        status(root.path(), 11, 11, bytes.as_bytes());
        assert!(observe(root.path(), 93).is_err());
    }
}

#[test]
fn a_nonleader_task_with_another_uid_cannot_be_ignored() {
    let root = tempfile::tempdir().unwrap();
    status(root.path(), 11, 11, b"Uid:\t71\t71\t71\t71\n");
    status(root.path(), 11, 12, b"Uid:\t71\t93\t71\t71\n");
    assert!(observe(root.path(), 93).is_err());
    status(root.path(), 11, 12, b"Uid:\t71\t71\t71\t71\n");
    assert!(observe(root.path(), 93).is_ok());
}

#[test]
fn malformed_missing_or_duplicate_uid_fields_remain_unknown() {
    let root = tempfile::tempdir().unwrap();
    for bytes in [
        b"Name: task\n".as_slice(),
        b"Uid: 71 71 71\n",
        b"Uid: 71 71 71 71 71\n",
        b"Uid: 71 +71 71 71\n",
        b"Uid: 71 x 71 71\n",
        b"Uid: 71 4294967296 71 71\n",
        b"Uid: 71 71 71 71\nUid: 71 71 71 71\n",
    ] {
        status(root.path(), 11, 11, bytes);
        assert!(observe(root.path(), 93).is_err());
    }
}

#[test]
fn non_utf8_process_names_do_not_hide_valid_uid_fields() {
    let root = tempfile::tempdir().unwrap();
    status(
        root.path(),
        11,
        11,
        b"Name:\t\xff\xfe\nUid:\t71\t71\t71\t71\n",
    );
    assert!(observe(root.path(), 93).is_ok());
    status(
        root.path(),
        11,
        11,
        b"Name:\t\xff\xfe\nUid:\t93\t93\t93\t93\n",
    );
    assert!(observe(root.path(), 93).is_err());
}

#[test]
fn missing_status_or_tasks_for_present_directories_is_not_absence() {
    let root = tempfile::tempdir().unwrap();
    let process = root.path().join("11");
    fs::create_dir(&process).unwrap();
    assert!(observe(root.path(), 93).is_err());
    let task = process.join("task/11");
    fs::create_dir_all(&task).unwrap();
    assert!(observe(root.path(), 93).is_err());
    fs::create_dir(task.join("status")).unwrap();
    assert!(observe(root.path(), 93).is_err());
}

#[test]
fn an_actually_vanished_task_is_distinct_from_a_status_read_failure() {
    let root = tempfile::tempdir().unwrap();
    assert!(require_task_unused(&root.path().join("vanished"), 93).is_ok());
    let failure = io::Error::from(io::ErrorKind::PermissionDenied);
    assert!(directory_vanished(&root.path().join("vanished"), &failure).is_err());
}

#[test]
fn missing_proc_root_and_reserved_uids_are_refused() {
    let root = tempfile::tempdir().unwrap();
    assert!(observe(&root.path().join("absent"), 93).is_err());
    assert!(observe(root.path(), 0).is_err());
    assert!(observe(root.path(), u32::MAX).is_err());
    assert!(observe(root.path(), 93).is_ok());
}

#[test]
fn process_disappearance_and_replacement_are_checked_after_enumeration() {
    let root = tempfile::tempdir().unwrap();
    status(root.path(), 11, 11, b"Uid: 93 93 93 93\n");
    let mut budget = Budget {
        deadline: Instant::now() + Duration::from_secs(2),
        entries: 0,
        limit: 100,
    };
    scan_uid_with(root.path(), 93, &mut budget, |task| {
        fs::remove_file(task.join("status"))?;
        fs::remove_dir(task)?;
        Ok(())
    })
    .unwrap();
    status(root.path(), 11, 11, b"Uid: 71 71 71 71\n");
    let mut budget = Budget {
        deadline: Instant::now() + Duration::from_secs(2),
        entries: 0,
        limit: 100,
    };
    assert!(
        scan_uid_with(root.path(), 93, &mut budget, |task| {
            fs::write(task.join("status"), b"Uid: 71 71 93 71\n")?;
            Ok(())
        })
        .is_err()
    );
}

#[test]
fn entry_and_time_budgets_return_unresolved() {
    let root = tempfile::tempdir().unwrap();
    status(root.path(), 11, 11, b"Uid: 71 71 71 71\n");
    let mut entries = Budget {
        deadline: Instant::now() + Duration::from_secs(2),
        entries: 0,
        limit: 1,
    };
    assert!(require_uid_unheld(root.path(), 93, &mut entries).is_err());
    let mut expired = Budget {
        deadline: Instant::now(),
        entries: 0,
        limit: 100,
    };
    assert!(matches!(
        require_uid_unheld(root.path(), 93, &mut expired),
        Err(RuntimeError::TimedOut(_))
    ));
}

#[test]
fn status_symlinks_and_oversized_metadata_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let task = root.path().join("11/task/11");
    fs::create_dir_all(&task).unwrap();
    let foreign = root.path().join("foreign");
    fs::write(&foreign, b"Uid: 71 71 71 71\n").unwrap();
    std::os::unix::fs::symlink(&foreign, task.join("status")).unwrap();
    assert!(observe(root.path(), 93).is_err());
    fs::remove_file(task.join("status")).unwrap();
    fs::write(
        task.join("status"),
        vec![b'x'; usize::try_from(MAX_STATUS_BYTES).unwrap() + 1],
    )
    .unwrap();
    assert!(observe(root.path(), 93).is_err());
}

#[test]
fn restricted_or_ambiguous_proc_views_are_refused() {
    assert!(single_namespace_pid(b"NSpid: 71\n", 71).is_ok());
    for value in [
        b"Name: task\n".as_slice(),
        b"NSpid: 71 1\n",
        b"NSpid: 72\n",
        b"NSpid: 71\nNSpid: 71\n",
    ] {
        assert!(single_namespace_pid(value, 71).is_err());
    }
    let normal = b"11 1 0:1 / /proc rw,nosuid,nodev,noexec - proc proc rw\n";
    assert!(unrestricted_proc_mount(normal).is_ok());
    assert!(
        unrestricted_proc_mount(&[normal.as_slice(), b"11 1 0:1 / /proc rw\n"].concat()).is_err()
    );
    for options in ["hidepid=1", "hidepid=2", "hidepid=invisible", "subset=pid"] {
        let line = format!("11 1 0:1 / /proc rw - proc proc rw,{options}\n");
        assert!(unrestricted_proc_mount(line.as_bytes()).is_err());
    }
    assert!(unrestricted_proc_mount(b"11 1 0:1 / /proc rw - tmpfs tmpfs rw\n").is_err());
    assert!(unrestricted_proc_mount(b"").is_err());
    assert!(unrestricted_proc_mount(&[normal.as_slice(), normal.as_slice()].concat()).is_err());
}

#[test]
fn real_owned_process_and_unreaped_zombie_hold_uid_until_disappearance() {
    use std::process::{Command, Stdio};
    let _serial = super::super::PROCESS_TEST_LOCK.lock().unwrap();
    let mut child = OwnedProcess(
        Command::new("/bin/sh")
            .args(["-c", "IFS= read -r finish"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let source = super::super::RuntimeProcessIdentity::capture(child.0.id()).unwrap();
    let task = Path::new("/proc")
        .join(child.0.id().to_string())
        .join("task")
        .join(child.0.id().to_string());
    let uid = rustix::process::geteuid().as_raw();
    assert!(require_task_unused(&task, uid).is_err());
    let pid = rustix::process::Pid::from_raw(i32::try_from(child.0.id()).unwrap()).unwrap();
    let pidfd = rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()).unwrap();
    drop(child.0.stdin.take());
    assert!(super::super::wait_pidfd_exit(&pidfd, Duration::from_secs(5)).unwrap());
    assert!(source.is_alive().unwrap());
    assert!(require_task_unused(&task, uid).is_err());
    child.0.wait().unwrap();
    assert!(!source.is_alive().unwrap());
    assert!(require_task_unused(&task, uid).is_ok());
}

#[test]
fn a_real_nonleader_thread_is_observed_until_joined() {
    let _serial = super::super::PROCESS_TEST_LOCK.lock().unwrap();
    let (ready_send, ready_receive) = std::sync::mpsc::channel();
    let (release_send, release_receive) = std::sync::mpsc::channel();
    let join = std::thread::spawn(move || {
        let task = Path::new("/proc").join(fs::read_link("/proc/thread-self").unwrap());
        ready_send.send(task).unwrap();
        let _ = release_receive.recv();
    });
    let thread = OwnedThread {
        release: Some(release_send),
        join: Some(join),
    };
    let task = ready_receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(require_task_unused(&task, rustix::process::geteuid().as_raw()).is_err());
    drop(thread);
    assert!(require_task_unused(&task, rustix::process::geteuid().as_raw()).is_ok());
}
