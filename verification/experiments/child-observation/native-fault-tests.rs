//! Test-only attachment inside reaper: real `Child`, `Slot`, `retain` and `reap_ready`.
//! Poll faults and logical clock are controlled; no claim that Linux returned
//! the injected replies. The real `SystemIo` positive path is tested separately.
use super::*;
use super::super::{OwnedChild, observation_core::{self, Operations}};
use std::{collections::VecDeque, io::{BufRead, BufReader}, process::{Command, ExitStatus, Stdio}};

struct ExactPidCleanup(u32);
impl Drop for ExactPidCleanup {
    fn drop(&mut self) {
        let pid = rustix::process::Pid::from_raw(i32::try_from(self.0).unwrap()).unwrap();
        // Every tested branch already sent SIGKILL to this exact original child.
        // ECHILD means the original reaper already completed the wait.
        match rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::empty()) {
            Ok(Some((got, _))) => assert_eq!(got, pid),
            Err(rustix::io::Errno::CHILD) => {},
            other => panic!("unexpected exact-child cleanup: {other:?}"),
        }
    }
}

struct Controlled {
    reaper: Arc<Reaper>,
    id: u32,
    polls: VecDeque<bool>,
    ticks: VecDeque<u64>,
    history: Vec<&'static str>,
    kill_calls: usize,
}
impl Operations for Controlled {
    type Child = Child;
    type Slot = Slot;
    type Status = ExitStatus;
    type Error = io::Error;
    type Instant = u64;
    fn try_wait(&mut self, child: &mut Child) -> io::Result<Option<ExitStatus>> {
        assert_eq!(child.id(), self.id);
        self.history.push("poll");
        if self.polls.pop_front().expect("finite controlled poll plan") {
            Err(io::Error::from(io::ErrorKind::Interrupted))
        } else {
            Ok(None)
        }
    }
    fn kill(&mut self, child: &mut Child) -> io::Result<()> {
        assert_eq!(child.id(), self.id);
        self.kill_calls += 1;
        self.history.push("kill");
        child.kill()
    }
    fn close_pipes(&mut self, child: &mut Child) {
        assert_eq!(child.id(), self.id);
        drop(child.stdin.take()); self.history.push("close-stdin");
        drop(child.stdout.take()); self.history.push("close-stdout");
        drop(child.stderr.take()); self.history.push("close-stderr");
    }
    fn now(&mut self) -> u64 {
        self.history.push("now");
        self.ticks.pop_front().expect("finite controlled clock plan")
    }
    fn add_grace(&mut self, now: u64) -> u64 {
        self.history.push("deadline");
        now.checked_add(2).unwrap()
    }
    fn pause(&mut self) { self.history.push("pause"); }
    fn retain(&mut self, slot: Slot, child: Child) {
        assert_eq!(child.id(), self.id);
        assert!(Arc::ptr_eq(&slot.0, &self.reaper));
        slot.retain(child);
        self.history.push("handed-off");
    }
    fn unknown_exit(&mut self) -> io::Error {
        self.history.push("timeout-error");
        io::Error::new(io::ErrorKind::TimedOut, "host helper exit remains unknown")
    }
}

fn check_unknown(first_error: bool, later_error: bool) {
    let _serial = crate::linux::PROCESS_TEST_LOCK.lock().unwrap();
    let reaper = Arc::new(Reaper {
        active: AtomicUsize::new(0), pending: Mutex::new(Vec::with_capacity(MAX_HELPERS)),
        wake: Condvar::new(),
    });
    let mut child = Command::new("/bin/sh")
        .args(["-c", "printf 'ready\\n'; IFS= read -r line"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap()).read_line(&mut line).unwrap();
    assert_eq!(line, "ready\n");
    assert!(child.try_wait().unwrap().is_none());
    let id = child.id();
    let _cleanup = ExactPidCleanup(id);
    let mut owned = OwnedChild { child: Some(child), slot: Some(Slot::reserve(&reaper).unwrap()) };
    let mut io = Controlled { reaper: Arc::clone(&reaper), id,
        polls: VecDeque::from([first_error, later_error, !later_error]),
        ticks: VecDeque::from([0, 0, 2]), history: Vec::new(), kill_calls: 0 };
    let error = observation_core::stop(&mut owned.child, &mut owned.slot, &mut io).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(error.to_string(), "host helper exit remains unknown");
    assert_eq!(io.kill_calls, 1);
    assert!(owned.child.is_none() && owned.slot.is_none());
    // Same public property as Lean Accounted: an unknown result with no local
    // pair must have handed off that exact pair, not silently dropped it.
    assert!(io.history.ends_with(&["handed-off", "timeout-error"]),
        "unknown result lost the original Child/Slot responsibility");
    assert_eq!(io.history, ["poll", "kill", "close-stdin", "close-stdout", "close-stderr",
        "now", "deadline", "poll", "now", "pause", "poll", "now", "handed-off", "timeout-error"]);
    assert_eq!(reaper.active.load(Ordering::Acquire), 1);
    let mut pending = reaper.pending.lock().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].0.id(), id);
    assert!(Arc::ptr_eq(&pending[0].1.0, &reaper));
    pending[0].0.wait().unwrap();
    Reaper::reap_ready(&mut pending);
    assert!(pending.is_empty());
    assert_eq!(reaper.active.load(Ordering::Acquire), 0);
}
#[test]
fn unknown_after_none_preserves_exact_original_pair() { check_unknown(false, false); }
#[test]
fn unknown_after_errors_preserves_exact_original_pair() { check_unknown(true, true); }
