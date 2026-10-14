use std::fmt;
use std::io::{self, BufRead, Read};

#[derive(Clone, Copy, Debug)]
pub(super) enum Limit {
    Encoded,
    Decoded,
}
impl fmt::Display for Limit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gzip input budget exceeded: {self:?}")
    }
}
impl std::error::Error for Limit {}

pub(super) struct Input<R> {
    reader: R,
    remaining: u64,
    chunk: usize,
    kind: Limit,
}
impl<R> Input<R> {
    pub(super) fn new(reader: R, remaining: u64, chunk: usize, kind: Limit) -> Self {
        Self {
            reader,
            remaining,
            chunk,
            kind,
        }
    }
}
impl<R: BufRead> BufRead for Input<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let bytes = self.reader.fill_buf()?;
        // Always observe real EOF. A budget ending exactly at the source end
        // succeeds; a limit cannot manufacture EOF or hide trailing input.
        if !bytes.is_empty() && self.remaining == 0 {
            return Err(io::Error::other(self.kind));
        }
        let length = bytes
            .len()
            .min(self.chunk)
            .min(usize::try_from(self.remaining).unwrap_or(usize::MAX));
        Ok(&bytes[..length])
    }
    fn consume(&mut self, count: usize) {
        self.remaining -= count as u64;
        self.reader.consume(count);
    }
}
impl<R: BufRead> Read for Input<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let bytes = self.fill_buf()?;
        let count = bytes.len().min(output.len());
        output[..count].copy_from_slice(&bytes[..count]);
        self.consume(count);
        Ok(count)
    }
}
