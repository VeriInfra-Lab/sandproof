//! Original stop/reap control flow with explicit mutable OS observations.
//! This is called by the isolated `OwnedChild` candidate, not a replacement VM.
pub(super) trait Operations {
    type Child;
    type Slot;
    type Status;
    type Error;
    type Instant: Copy + PartialOrd;
    fn try_wait(&mut self, child: &mut Self::Child) -> Result<Option<Self::Status>, Self::Error>;
    fn kill(&mut self, child: &mut Self::Child) -> Result<(), Self::Error>;
    fn close_pipes(&mut self, child: &mut Self::Child);
    fn now(&mut self) -> Self::Instant;
    fn add_grace(&mut self, now: Self::Instant) -> Self::Instant;
    fn pause(&mut self);
    fn retain(&mut self, slot: Self::Slot, child: Self::Child);
    fn unknown_exit(&mut self) -> Self::Error;
}

pub(super) fn stop<I: Operations>(
    owned_child: &mut Option<I::Child>,
    owned_slot: &mut Option<I::Slot>,
    io: &mut I,
) -> Result<I::Status, I::Error> {
    let Some(child) = owned_child else {
        panic!("owned child");
    };
    if let Ok(Some(status)) = io.try_wait(child) {
        return Ok(status);
    }
    let _ = io.kill(child);
    io.close_pipes(child);
    let now = io.now();
    let deadline = io.add_grace(now);
    loop {
        if let Ok(Some(status)) = io.try_wait(child) {
            return Ok(status);
        }
        if io.now() >= deadline {
            let Some(child) = owned_child.take() else {
                panic!("owned child");
            };
            let Some(slot) = owned_slot.take() else {
                panic!("helper admission");
            };
            io.retain(slot, child);
            return Err(io.unknown_exit());
        }
        io.pause();
    }
}
