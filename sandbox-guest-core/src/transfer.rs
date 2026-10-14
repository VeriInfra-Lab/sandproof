use crate::StreamFailure;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferError {
    InvalidConfiguration,
    TerminalTransfer,
    InputClosed,
    LimitExceeded,
    BufferFull,
    UnobservedWrite,
    Incomplete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferOutcome {
    Pending,
    /// All input ended and all accepted bytes were written. This is not an
    /// fsync, atomic replace, whole-batch transaction, or storage durability claim.
    Complete(u64),
    /// Already written bytes remain observable; there is no implicit rollback.
    Failed {
        reason: StreamFailure,
        written: u64,
    },
}

/// Bounded accounting for one file body and its actual writes.
/// Multipart/async batches instantiate this per file, not as an atomic batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    limit: u64,
    buffer: u64,
    expected: Option<u64>,
    received: u64,
    written: u64,
    eof: bool,
    finished: bool,
    failure: Option<StreamFailure>,
}

impl Transfer {
    /// # Errors
    /// Rejects zero budgets or an advertised body larger than the file budget.
    /// A zero-byte body is valid and still requires an observed end of input.
    pub fn new(limit: u64, buffer: u64, expected: Option<u64>) -> Result<Self, TransferError> {
        if limit == 0 || buffer == 0 {
            return Err(TransferError::InvalidConfiguration);
        }
        if expected.is_some_and(|length| length > limit) {
            return Err(TransferError::LimitExceeded);
        }
        Ok(Self {
            limit,
            buffer,
            expected,
            received: 0,
            written: 0,
            eof: false,
            finished: false,
            failure: None,
        })
    }

    fn check(&self) -> Result<(), TransferError> {
        if self.outcome() != TransferOutcome::Pending {
            return Err(TransferError::TerminalTransfer);
        }
        Ok(())
    }

    /// Reserve a chunk before retaining its bytes. The caller must fail the
    /// actual request on a total-length rejection; it may wait on `BufferFull`.
    /// # Errors
    /// Rejects closed input, exceeded total/advertised length or resident budget
    /// without accepting any bytes of the rejected chunk.
    pub fn receive(&mut self, bytes: u64) -> Result<(), TransferError> {
        self.check()?;
        if self.eof {
            return Err(TransferError::InputClosed);
        }
        let ceiling = self.expected.unwrap_or(self.limit);
        if bytes > ceiling - self.received {
            return Err(TransferError::LimitExceeded);
        }
        if bytes > self.buffer - self.pending_bytes() {
            return Err(TransferError::BufferFull);
        }
        self.received += bytes;
        Ok(())
    }

    /// Advance only by the number of bytes actually written, including a short
    /// write. Later I/O failure preserves this count and the partial file.
    /// # Errors
    /// Rejects a terminal transfer or writes exceeding accepted input.
    pub fn record_write(&mut self, bytes: u64) -> Result<(), TransferError> {
        self.check()?;
        if bytes > self.pending_bytes() {
            return Err(TransferError::UnobservedWrite);
        }
        self.written += bytes;
        Ok(())
    }

    /// # Errors
    /// Rejects an already terminal transfer. Short advertised input becomes a
    /// failed transfer, not complete success. Pending writes remain accounted.
    pub fn end_input(&mut self) -> Result<(), TransferError> {
        self.check()?;
        self.eof = true;
        if self
            .expected
            .is_some_and(|expected| expected != self.received)
        {
            self.failure = Some(StreamFailure::Truncated);
        }
        Ok(())
    }

    /// # Errors
    /// Rejects an already terminal transfer; never rewrites a complete result.
    pub fn fail(&mut self, reason: StreamFailure) -> Result<(), TransferError> {
        self.check()?;
        self.failure = Some(reason);
        Ok(())
    }

    /// Mark completion only after the adapter has also completed any required
    /// file close/metadata steps. Their failure can still fail a fully written
    /// body before this call. This does not assert durable or atomic storage.
    /// # Errors
    /// Rejects a terminal transfer, missing input end, or pending writes.
    pub fn finish(&mut self) -> Result<(), TransferError> {
        self.check()?;
        if !self.eof || self.pending_bytes() != 0 {
            return Err(TransferError::Incomplete);
        }
        self.finished = true;
        Ok(())
    }

    #[must_use]
    pub const fn pending_bytes(&self) -> u64 {
        self.received - self.written
    }

    #[must_use]
    pub const fn written_bytes(&self) -> u64 {
        self.written
    }

    #[must_use]
    pub const fn outcome(&self) -> TransferOutcome {
        if let Some(reason) = self.failure {
            return TransferOutcome::Failed {
                reason,
                written: self.written,
            };
        }
        if self.finished {
            TransferOutcome::Complete(self.written)
        } else {
            TransferOutcome::Pending
        }
    }
}
