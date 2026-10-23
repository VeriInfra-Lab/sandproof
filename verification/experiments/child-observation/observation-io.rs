//! Mechanical calls for the original objects. Decisions stay in `observation_core`.
use super::{POLL_SLICE, REAP_GRACE, observation_core::Operations, reaper::Slot};
use std::{
    io,
    process::{Child, ExitStatus},
    time::Instant,
};
pub(super) struct SystemIo;
impl Operations for SystemIo {
    type Child = Child;
    type Slot = Slot;
    type Status = ExitStatus;
    type Error = io::Error;
    type Instant = Instant;
    fn try_wait(&mut self, child: &mut Child) -> io::Result<Option<ExitStatus>> {
        child.try_wait()
    }
    fn kill(&mut self, child: &mut Child) -> io::Result<()> {
        child.kill()
    }
    fn close_pipes(&mut self, child: &mut Child) {
        drop(child.stdin.take());
        drop(child.stdout.take());
        drop(child.stderr.take());
    }
    fn now(&mut self) -> Instant {
        Instant::now()
    }
    fn add_grace(&mut self, now: Instant) -> Instant {
        now + REAP_GRACE
    }
    fn pause(&mut self) {
        std::thread::sleep(POLL_SLICE);
    }
    fn retain(&mut self, slot: Slot, child: Child) {
        slot.retain(child);
    }
    fn unknown_exit(&mut self) -> io::Error {
        io::Error::new(io::ErrorKind::TimedOut, "host helper exit remains unknown")
    }
}

/// Construct only the empty mechanical adapter; no OS operation occurs here.
#[must_use]
pub(super) fn system() -> SystemIo {
    SystemIo
}
