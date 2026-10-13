//! Fixed Go 1.26.8 `ServeContent` condition/range decisions. No I/O, routing,
//! MIME sniffing or implicit success for a planned range. The caller preserves
//! auth/stat/open/precondition/seek order and must execute all planned reads.
// Adapted from Go net/http/fs.go (Copyright 2009 The Go Authors).
// BSD license: ../multipart/GO-LICENSE.
mod conditions;
pub mod date;
mod errno;
pub use errno::LinuxErrno;
mod ranges;
pub use conditions::{Conditions, Precondition, precondition};
pub use ranges::{ByteRange, RangePlan, ranges};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidConfiguration,
    HeaderLimit,
    ItemLimit,
    InvalidTimestamp,
    ArithmeticOverflow,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub header_bytes: usize,
    pub items: usize,
}
impl Limits {
    fn validate(self) -> Result<(), Error> {
        if self.header_bytes == 0
            || self.header_bytes > isize::MAX as usize / 16
            || self.items == 0
            || self.items > isize::MAX as usize / 16
        {
            return Err(Error::InvalidConfiguration);
        }
        Ok(())
    }
    fn check(self, headers: &[&[u8]]) -> Result<(), Error> {
        self.validate()?;
        let total = headers
            .iter()
            .try_fold(0_usize, |size, value| size.checked_add(value.len()));
        if total.is_none_or(|size| size > self.header_bytes) {
            return Err(Error::HeaderLimit);
        }
        Ok(())
    }
}

fn trim(mut value: &[u8]) -> &[u8] {
    let space = |byte: u8| matches!(byte, b' ' | b'\t' | b'\r' | b'\n');
    while value.first().is_some_and(|b| space(*b)) {
        value = &value[1..];
    }
    while value.last().is_some_and(|b| space(*b)) {
        value = &value[..value.len() - 1];
    }
    value
}
