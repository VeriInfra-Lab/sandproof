//! Trusted time is sampled afresh at every original check.
use super::ApplicationError;
use crate::JournalError;
use std::time::{SystemTime, UNIX_EPOCH};

pub trait Clock: Send + Sync {
    /// # Errors
    /// Invalid/unavailable epoch milliseconds fail closed.
    fn now_ms(&self) -> Result<i64, ApplicationError>;
    /// # Errors
    /// Preserves unknown-outcome handling at publication.
    fn publication_ms(&self) -> Result<i64, JournalError> {
        self.now_ms()
            .map_err(|_| JournalError::Io(std::io::Error::other("invalid service clock")))
    }
}
#[derive(Debug)]
pub struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&self) -> Result<i64, ApplicationError> {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ApplicationError::Busy)?;
        i64::try_from(duration.as_millis()).map_err(|_| ApplicationError::Busy)
    }
}
