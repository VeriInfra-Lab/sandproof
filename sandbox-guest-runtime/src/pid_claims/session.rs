use super::{Error, PidClaims, SESSION, Stage};

impl PidClaims {
    /// Burn a nonzero service session for this VM before publishing its daemon.
    /// The caller passes this value to the sole active registry and command
    /// dispatcher. A failed later startup or lost return never returns a
    /// durably issued number. The PID bitmap is preserved, even after restart.
    ///
    /// This does not adopt old processes, authenticate a session or replace the
    /// separate permanent refusal to reuse a public PID. Only known-new VM
    /// provisioning creates a zero counter; old formats are not migrated.
    /// # Errors
    /// Counter exhaustion, original storage refusal, uncertain write or a
    /// poisoned writer. Failure/panic during persistence poisons both session
    /// allocation and PID claims until a complete valid image can be reopened.
    pub fn allocate_session(&mut self) -> Result<u64, Error> {
        self.allocate_session_with(|_| Ok(()))
    }

    pub(super) fn allocate_session_with(
        &mut self,
        mut checkpoint: impl FnMut(Stage) -> Result<(), Error>,
    ) -> Result<u64, Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        let mut bytes = [0; 8];
        bytes.copy_from_slice(&self.image[SESSION]);
        let next = u64::from_le_bytes(bytes)
            .checked_add(1)
            .ok_or(Error::SessionExhausted)?;
        self.poisoned = true;
        self.verify()?;
        self.image[SESSION].copy_from_slice(&next.to_le_bytes());
        self.persist_change(SESSION, Stage::SessionWritten, &mut checkpoint)?;
        Ok(next)
    }
}
