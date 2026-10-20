//! Test-only attachment to the original `child_process` module.
use super::*;
use std::{io::{BufRead, BufReader}, process::Stdio};

fn blocked() -> Child {
    let mut child = Command::new("/bin/sh")
        .args(["-c", "printf 'ready\\n'; IFS= read -r line"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap()).read_line(&mut line).unwrap();
    assert_eq!(line, "ready\n");
    assert!(child.try_wait().unwrap().is_none());
    child
}

#[test]
fn actual_stop_observes_original_child_before_success() {
    let _serial = crate::linux::PROCESS_TEST_LOCK.lock().unwrap();
    let mut child = OwnedChild::new(blocked());
    let id = child.id();
    let status = child.stop_and_reap().unwrap();
    assert_eq!(child.id(), id);
    assert_eq!(child.try_wait().unwrap(), Some(status));
    assert!(child.slot.is_some());
    assert!(child.stdin.is_none() && child.stdout.is_none() && child.stderr.is_none());
}

#[test]
fn already_observed_exit_returns_exact_status_without_closing_pipes() {
    let _serial = crate::linux::PROCESS_TEST_LOCK.lock().unwrap();
    let mut raw = blocked();
    raw.stdin.as_mut().unwrap().write_all(b"finish\n").unwrap();
    let status = raw.wait().unwrap();
    let mut child = OwnedChild::new(raw);
    assert_eq!(child.stop_and_reap().unwrap(), status);
    assert!(child.stdout.is_some() && child.stderr.is_some());
    assert!(child.slot.is_some());
}

#[test]
fn zero_argument_observation_can_change_between_calls() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static OBSERVATION: AtomicUsize = AtomicUsize::new(0);
    fn observe() -> usize {
        OBSERVATION.fetch_add(1, Ordering::SeqCst)
    }
    let first = observe();
    let second = observe();
    assert_eq!(second, first.checked_add(1).unwrap());
}
