//! Bounded blocking gzip codecs. These do not open files or own HTTP requests.
//!
//! Callers must retain authorization/capacity until blocking reads actually end,
//! enforce their original deadline on the input, and release each output chunk.
//! Only `Ok(None)` means validated completion; dropping a codec is cancellation.
//! The caller's `BufRead` allocation and the fixed DEFLATE library workspace are
//! separate from the one configured output chunk allocated here.

mod header;
mod input;
#[cfg(test)]
mod tests;

use crc32fast::Hasher;
use flate2::{Compression, Decompress, FlushDecompress, Status};
use input::Input;
use std::fmt;
use std::io::{self, BufRead, Read};

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecodeLimits {
    pub chunk_bytes: usize,
    pub encoded_bytes: u64,
    pub decoded_bytes: u64,
    pub header_bytes: usize,
    pub members: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct EncodeLimits {
    pub chunk_bytes: usize,
    pub decoded_bytes: u64,
    pub encoded_bytes: u64,
}

#[derive(Debug)]
pub enum Error {
    InvalidConfiguration,
    EncodedLimit,
    DecodedLimit,
    HeaderLimit,
    MemberLimit,
    EmptyHeader,
    Header,
    Deflate,
    Checksum,
    Truncated,
    Io(io::Error),
    Failed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "gzip input: {error}"),
            error => write!(f, "gzip stream: {error:?}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let Self::Io(error) = self {
            Some(error)
        } else {
            None
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        if let Some(limit) = error
            .get_ref()
            .and_then(|e| e.downcast_ref::<input::Limit>())
        {
            return match limit {
                input::Limit::Encoded => Self::EncodedLimit,
                input::Limit::Decoded => Self::DecodedLimit,
            };
        }
        if error.kind() == io::ErrorKind::UnexpectedEof {
            Self::Truncated
        } else {
            Self::Io(error)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Body,
    Trailer,
    End,
    Failed,
}

pub struct Decoder<R> {
    input: Input<R>,
    codec: Decompress,
    crc: Hasher,
    member_size: u32,
    members: usize,
    decoded: u64,
    limits: DecodeLimits,
    state: State,
    pending: Option<Error>,
}

impl<R: BufRead> Decoder<R> {
    /// Validates the first header before returning. Perform this before opening
    /// or truncating an upload target. Empty input is an initialization error.
    ///
    /// # Errors
    /// Rejects invalid budgets, header/input limits, malformed or incomplete
    /// headers, and source errors.
    pub fn new(reader: R, limits: DecodeLimits) -> Result<Self, Error> {
        if limits.chunk_bytes == 0 || limits.header_bytes < 10 || limits.members == 0 {
            return Err(Error::InvalidConfiguration);
        }
        let mut input = Input::new(
            reader,
            limits.encoded_bytes,
            limits.chunk_bytes,
            input::Limit::Encoded,
        );
        // Go NewReader distinguishes zero members (io.EOF) from a partial
        // first header (io.ErrUnexpectedEOF). Only observed EOF counts: source
        // errors and exhausted budgets must remain failures of their own kind.
        let empty = loop {
            match input.fill_buf() {
                // The original read_exact retried Interrupted. Peeking must
                // preserve that behavior without consuming the first byte.
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.into()),
                Ok(bytes) => break bytes.is_empty(),
            }
        };
        if empty {
            return Err(Error::EmptyHeader);
        }
        header::read(&mut input, limits.header_bytes)?;
        Ok(Self {
            input,
            codec: Decompress::new(false),
            crc: Hasher::new(),
            member_size: 0,
            members: 1,
            decoded: 0,
            limits,
            state: State::Body,
            pending: None,
        })
    }

    /// Returns at most one configured chunk. A decoded prefix may precede an
    /// error on the next call; none of those bytes constitute a successful file.
    ///
    /// # Errors
    /// Any invalid member/trailer, budget violation or source error poisons the
    /// decoder permanently. Later calls return `Failed`, never completion.
    pub fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Error> {
        let result = self.next();
        if result.is_err() {
            self.state = State::Failed;
        }
        result
    }

    fn next(&mut self) -> Result<Option<Vec<u8>>, Error> {
        if let Some(error) = self.pending.take() {
            return Err(error);
        }
        if self.state == State::Failed {
            return Err(Error::Failed);
        }
        loop {
            match self.state {
                State::End => return Ok(None),
                State::Trailer => self.finish_member()?,
                State::Body => {
                    let remaining = self.limits.decoded_bytes - self.decoded;
                    // At the exact limit, one private byte distinguishes a
                    // complete stream from further output. It is never emitted.
                    let size = usize::try_from(remaining)
                        .unwrap_or(usize::MAX)
                        .min(self.limits.chunk_bytes)
                        .max(1);
                    let mut output = vec![0; size];
                    let before_in = self.codec.total_in();
                    let before_out = self.codec.total_out();
                    let source = self.input.fill_buf()?;
                    let eof = source.is_empty();
                    let status = self
                        .codec
                        .decompress(source, &mut output, FlushDecompress::None);
                    let consumed = usize::try_from(self.codec.total_in() - before_in)
                        .expect("consumed input fits the source slice");
                    let produced = usize::try_from(self.codec.total_out() - before_out)
                        .expect("produced output fits the output slice");
                    self.input.consume(consumed);
                    output.truncate(produced);
                    if produced != 0 && remaining == 0 {
                        return Err(Error::DecodedLimit);
                    }
                    self.decoded += produced as u64;
                    self.member_size = self
                        .member_size
                        .wrapping_add(u32::try_from(produced & 0xffff_ffff).expect("masked u32"));
                    self.crc.update(&output);
                    match status {
                        Ok(Status::StreamEnd) => self.state = State::Trailer,
                        Err(_) => self.pending = Some(Error::Deflate),
                        Ok(_) if consumed == 0 && produced == 0 => {
                            self.pending = Some(if eof {
                                Error::Truncated
                            } else {
                                Error::Deflate
                            });
                        }
                        Ok(_) => {}
                    }
                    if produced != 0 {
                        return Ok(Some(output));
                    }
                    if let Some(error) = self.pending.take() {
                        return Err(error);
                    }
                }
                State::Failed => return Err(Error::Failed),
            }
        }
    }

    fn finish_member(&mut self) -> Result<(), Error> {
        let mut trailer = [0; 8];
        self.input.read_exact(&mut trailer)?;
        let crc = u32::from_le_bytes(trailer[..4].try_into().expect("four bytes"));
        let size = u32::from_le_bytes(trailer[4..].try_into().expect("four bytes"));
        if crc != self.crc.clone().finalize() || size != self.member_size {
            return Err(Error::Checksum);
        }
        if self.input.fill_buf()?.is_empty() {
            self.state = State::End;
        } else {
            if self.members == self.limits.members {
                return Err(Error::MemberLimit);
            }
            header::read(&mut self.input, self.limits.header_bytes)?;
            self.members += 1;
            self.codec.reset(false);
            self.crc.reset();
            self.member_size = 0;
            self.state = State::Body;
        }
        Ok(())
    }
}

pub struct Encoder<R: BufRead> {
    codec: flate2::bufread::GzEncoder<Input<R>>,
    limits: EncodeLimits,
    emitted: u64,
    state: State,
}

impl<R: BufRead> Encoder<R> {
    /// Creates a single member with no filename/comment and a zero timestamp.
    /// Compression bytes are not promised to equal Go's DEFLATE implementation.
    ///
    /// # Errors
    /// A zero chunk size is invalid. Input and output totals may be zero.
    pub fn new(reader: R, limits: EncodeLimits) -> Result<Self, Error> {
        if limits.chunk_bytes == 0 {
            return Err(Error::InvalidConfiguration);
        }
        let input = Input::new(
            reader,
            limits.decoded_bytes,
            limits.chunk_bytes,
            input::Limit::Decoded,
        );
        Ok(Self {
            codec: flate2::bufread::GzEncoder::new(input, Compression::default()),
            limits,
            emitted: 0,
            state: State::Body,
        })
    }

    /// # Errors
    /// Source failures and input/output limits permanently poison the encoder.
    /// An emitted prefix without `Ok(None)` must be treated as an incomplete
    /// response, even if the recipient has already consumed some bytes.
    pub fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Error> {
        if self.state == State::Failed {
            return Err(Error::Failed);
        }
        if self.state == State::End {
            return Ok(None);
        }
        let result = self.next();
        if result.is_err() {
            self.state = State::Failed;
        }
        result
    }

    fn next(&mut self) -> Result<Option<Vec<u8>>, Error> {
        let remaining = self.limits.encoded_bytes - self.emitted;
        let size = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(self.limits.chunk_bytes)
            .max(1);
        let mut output = vec![0; size];
        let count = self.codec.read(&mut output)?;
        if count == 0 {
            self.state = State::End;
            return Ok(None);
        }
        if remaining == 0 {
            return Err(Error::EncodedLimit);
        }
        self.emitted += count as u64;
        output.truncate(count);
        Ok(Some(output))
    }
}
