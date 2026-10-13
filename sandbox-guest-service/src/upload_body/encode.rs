use super::{GzipBody, GzipError, Owner, check_active};
use sandproof_guest_runtime::gzip::{EncodeLimits, Encoder};
use std::io::{self, BufRead, BufReader, Read};
use tokio::{sync::watch, time::Instant};

#[cfg(test)]
mod tests;

impl GzipBody {
    /// Open a blocking source under an existing admission/authorization guard
    /// and encode until its actual EOF. Opening runs on the same owned worker
    /// as compression. No source bytes are read until the consumer asks.
    ///
    /// `chunk_bytes` bounds both the source buffer and each output chunk;
    /// fixed codec workspace is separate. Only consumed output releases
    /// backpressure. Cancellation cannot interrupt a running syscall: the
    /// worker retains the guard until that call and source destruction finish.
    /// # Errors
    /// Invalid limits, source opening, original deadline; later reads also
    /// report source errors and raw/compressed limits without successful EOF.
    pub async fn encode<R, F>(
        open: F,
        limits: EncodeLimits,
        deadline: Instant,
        owner: Owner,
    ) -> Result<Self, GzipError>
    where
        R: Read + 'static,
        F: FnOnce() -> Result<R, GzipError> + Send + 'static,
    {
        if limits.chunk_bytes == 0 || limits.chunk_bytes > isize::MAX as usize / 16 {
            drop(open);
            return Err(GzipError::InvalidConfiguration);
        }
        Self::start(deadline, owner, move |_, cancelled| {
            let source = open()?;
            check_active(deadline, &cancelled)?;
            let reader = Reader {
                inner: BufReader::with_capacity(limits.chunk_bytes, source),
                cancelled,
                deadline,
            };
            Encoder::new(reader, limits).map_err(GzipError::from)
        })
        .await
    }
}

struct Reader<R> {
    inner: BufReader<R>,
    cancelled: watch::Receiver<bool>,
    deadline: Instant,
}
impl<R: Read> BufRead for Reader<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        check_active(self.deadline, &self.cancelled).map_err(io::Error::other)?;
        // Check after the actual blocking read as well as before buffered
        // consumption. Expiry/cancellation during a syscall cannot yield EOF.
        let bytes = self.inner.fill_buf()?;
        check_active(self.deadline, &self.cancelled).map_err(io::Error::other)?;
        Ok(bytes)
    }
    fn consume(&mut self, count: usize) {
        self.inner.consume(count);
    }
}
impl<R: Read> Read for Reader<R> {
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
