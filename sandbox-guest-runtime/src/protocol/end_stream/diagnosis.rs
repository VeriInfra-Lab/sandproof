//! Fixed Go encoding/json observations, only after actual EOS validation fails.
//! Observations contain finite schema facts, a byte, or an admitted offset.
use std::fmt;

mod scanner;
mod schema;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndStreamReason {
    Eof,
    Syntax {
        byte: u8,
        context: &'static str,
    },
    Type {
        value: &'static str,
        field: &'static str,
        target: &'static str,
    },
    Base64 {
        offset: usize,
    },
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EndStreamError(EndStreamReason);
impl EndStreamError {
    #[must_use]
    pub const fn reason(self) -> EndStreamReason {
        self.0
    }
}
impl fmt::Debug for EndStreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EndStreamError").finish_non_exhaustive()
    }
}

pub(super) fn observe(payload: &[u8]) -> Option<EndStreamError> {
    // json.Unmarshal validates the entire input before decoding any field.
    if let Some(error) = scanner::observe(payload) {
        return Some(EndStreamError(error));
    }
    let normalized = super::normalize::json(payload).ok()?;
    schema::observe(&normalized).map(EndStreamError)
}
