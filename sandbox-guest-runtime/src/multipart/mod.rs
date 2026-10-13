//! Bounded incremental multipart framing for the fixed guest upload contract.
//! No filesystem, authorization, async runtime or HTTP listener. The caller
//! must consume events before feeding more input and independently own returned
//! allocations. Explicit End is distinct from transport EOF or a failed parser.
// Framing follows Go 1.26.8 mime/multipart (Copyright 2010 The Go Authors).
// All rights reserved.
// Adaptations are distributed under the BSD terms in GO-LICENSE.
mod headers;
mod quoted_printable;
#[cfg(test)]
mod tests;
pub(crate) use headers::{MediaError, media_type};
pub use headers::{Part, boundary};
pub use quoted_printable::{Decoded, QuotedPrintable};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidConfiguration,
    ResourceExhausted,
    Malformed,
    Truncated,
    /// EOF inside MIME headers. Go can return io.EOF here; this component
    /// requires an explicit closing delimiter before request success.
    TruncatedHeaders,
    Poisoned,
    InputPending,
}

/// Component budgets with no production defaults. Headers include all raw
/// bytes/fields, even those ignored by file upload. Total includes MIME framing,
/// ignored parts and any epilogue the caller feeds after End.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub chunk_bytes: usize,
    pub boundary_bytes: usize,
    pub header_bytes: usize,
    pub header_count: usize,
    pub line_bytes: usize,
    pub parts: usize,
    pub total_bytes: u64,
}
impl Limits {
    fn capacity(self) -> Result<usize, Error> {
        if [
            self.chunk_bytes,
            self.boundary_bytes,
            self.header_bytes,
            self.header_count,
            self.line_bytes,
            self.parts,
        ]
        .into_iter()
        .any(|n| n == 0 || n > isize::MAX as usize / 16)
            || self.total_bytes == 0
        {
            return Err(Error::InvalidConfiguration);
        }
        self.header_bytes
            .max(self.line_bytes)
            .max(self.boundary_bytes + 6)
            .checked_add(self.chunk_bytes)
            .filter(|n| *n <= isize::MAX as usize / 8)
            .ok_or(Error::InvalidConfiguration)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Part(Part),
    /// Raw field bytes; apply `QuotedPrintable` if the Part requests it.
    Data(Vec<u8>),
    PartEnd,
    End,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Preamble,
    Headers,
    Body,
    Boundary,
    End,
    Failed,
}

pub struct Decoder {
    limits: Limits,
    buffer: Vec<u8>,
    offset: usize,
    dash: Vec<u8>,
    delimiter: Vec<u8>,
    parts: usize,
    total: u64,
    body_start: bool,
    phase: Phase,
    wants_input: bool,
    eof: bool,
}

impl Decoder {
    /// # Errors
    /// Invalid/unrepresentable budgets or empty/oversized boundary.
    pub fn new(boundary: &str, limits: Limits) -> Result<Self, Error> {
        let capacity = limits.capacity()?;
        if boundary.is_empty() {
            return Err(Error::Malformed);
        }
        if boundary.len() > limits.boundary_bytes {
            return Err(Error::ResourceExhausted);
        }
        let dash = format!("--{boundary}").into_bytes();
        let delimiter = format!("\r\n--{boundary}").into_bytes();
        Ok(Self {
            limits,
            buffer: Vec::with_capacity(capacity),
            offset: 0,
            dash,
            delimiter,
            parts: 0,
            total: 0,
            body_start: true,
            phase: Phase::Preamble,
            wants_input: true,
            eof: false,
        })
    }

    /// A chunk never exceeds `chunk_bytes`. Feed only after `next_event` returned
    /// None, or after End while draining bounded epilogue/HTTP transport.
    /// # Errors
    /// Misuse, failed parser, size exhaustion or input after transport EOF.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let result = self.feed_inner(bytes);
        if result.is_err() {
            self.phase = Phase::Failed;
        }
        result
    }
    fn feed_inner(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if self.phase == Phase::Failed {
            return Err(Error::Poisoned);
        }
        if self.eof || (!self.wants_input && self.phase != Phase::End) {
            return Err(Error::InputPending);
        }
        if bytes.len() > self.limits.chunk_bytes
            || bytes.len() as u64 > self.limits.total_bytes - self.total
        {
            return Err(Error::ResourceExhausted);
        }
        self.total += bytes.len() as u64;
        if self.phase != Phase::End {
            self.buffer.drain(..self.offset);
            self.offset = 0;
            if bytes.len() > self.limits.capacity()? - self.buffer.len() {
                return Err(Error::ResourceExhausted);
            }
            self.buffer.extend_from_slice(bytes);
        }
        self.wants_input = false;
        Ok(())
    }

    /// Report actual transport EOF, then drain `next_event`. This alone never
    /// means a complete multipart request.
    pub fn end_input(&mut self) {
        self.eof = true;
    }

    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.phase == Phase::End
    }

    /// # Errors
    /// Invalid headers/boundaries, explicit budget exhaustion or early EOF.
    /// After any error this decoder cannot be reused.
    pub fn next_event(&mut self) -> Result<Option<Event>, Error> {
        let result = self.advance();
        match &result {
            Ok(None) => self.wants_input = true,
            Err(_) => self.phase = Phase::Failed,
            Ok(Some(_)) => {}
        }
        result
    }

    fn advance(&mut self) -> Result<Option<Event>, Error> {
        loop {
            let bytes = &self.buffer[self.offset..];
            match self.phase {
                Phase::Failed => return Err(Error::Poisoned),
                Phase::End => return Ok(None),
                Phase::Preamble | Phase::Boundary => match self.read_boundary()? {
                    Some(Phase::Headers) => self.phase = Phase::Headers,
                    Some(Phase::End) => return Ok(Some(Event::End)),
                    _ => return Ok(None),
                },
                Phase::Headers => {
                    // MIME header lines accept LF/CRLF regardless of body mode.
                    let mut end = 0;
                    let mut complete = false;
                    for line in bytes.split_inclusive(|b| *b == b'\n') {
                        if !line.ends_with(b"\n") {
                            break;
                        }
                        end += line.len();
                        if line == b"\n" || line == b"\r\n" {
                            complete = true;
                            break;
                        }
                    }
                    if end > self.limits.header_bytes
                        || (!complete && bytes.len() > self.limits.header_bytes)
                    {
                        return Err(Error::ResourceExhausted);
                    }
                    if !complete {
                        return if self.eof {
                            Err(Error::TruncatedHeaders)
                        } else {
                            Ok(None)
                        };
                    }
                    let part = headers::parse(&bytes[..end], self.limits.header_count)?;
                    self.offset += end;
                    self.phase = Phase::Body;
                    self.body_start = true;
                    return Ok(Some(Event::Part(part)));
                }
                Phase::Body => {
                    let scan = scan_body(
                        bytes,
                        &self.dash,
                        &self.delimiter,
                        self.body_start,
                        self.eof,
                    );
                    if scan.bytes != 0 {
                        let n = scan.bytes.min(self.limits.chunk_bytes);
                        let data = bytes[..n].to_vec();
                        self.offset += n;
                        self.body_start = false;
                        return Ok(Some(Event::Data(data)));
                    }
                    if scan.boundary {
                        if bytes.starts_with(&self.delimiter) {
                            self.offset += if self.delimiter[0] == b'\n' { 1 } else { 2 };
                        }
                        self.phase = Phase::Boundary;
                        return Ok(Some(Event::PartEnd));
                    }
                    return if self.eof {
                        Err(Error::Truncated)
                    } else {
                        Ok(None)
                    };
                }
            }
        }
    }
    fn read_boundary(&mut self) -> Result<Option<Phase>, Error> {
        loop {
            let bytes = &self.buffer[self.offset..];
            let end = match bytes.iter().position(|b| *b == b'\n') {
                Some(n) => n + 1,
                None if self.eof => bytes.len(),
                None if bytes.len() > self.limits.line_bytes => {
                    return Err(Error::ResourceExhausted);
                }
                None => return Ok(None),
            };
            if end > self.limits.line_bytes {
                return Err(Error::ResourceExhausted);
            }
            let line = &bytes[..end];
            let Some(rest) = line.strip_prefix(self.dash.as_slice()) else {
                if self.eof && !line.ends_with(b"\n") {
                    return Err(Error::Truncated);
                }
                if self.phase == Phase::Boundary {
                    return Err(Error::Malformed);
                }
                self.offset += end;
                continue;
            };
            let newline = if self.delimiter[0] == b'\n' {
                b"\n".as_slice()
            } else {
                b"\r\n".as_slice()
            };
            if let Some(tail) = rest.strip_prefix(b"--") {
                let tail = trim_start_space(tail);
                if tail.is_empty() || tail == newline {
                    self.offset = self.buffer.len();
                    self.phase = Phase::End;
                    return Ok(Some(Phase::End));
                }
            }
            let tail = trim_start_space(rest);
            let first_lf = self.parts == 0 && tail == b"\n";
            if tail == newline || first_lf {
                if self.parts == self.limits.parts {
                    return Err(Error::ResourceExhausted);
                }
                if first_lf {
                    self.delimiter.remove(0);
                }
                self.parts += 1;
                self.offset += end;
                return Ok(Some(Phase::Headers));
            }
            if self.eof && !line.ends_with(b"\n") {
                return Err(Error::Truncated);
            }
            if self.phase == Phase::Boundary {
                return Err(Error::Malformed);
            }
            self.offset += end;
        }
    }
}

fn trim_start_space(bytes: &[u8]) -> &[u8] {
    let n = bytes
        .iter()
        .take_while(|b| matches!(b, b' ' | b'\t'))
        .count();
    &bytes[n..]
}

struct Scan {
    bytes: usize,
    boundary: bool,
}
fn after_prefix(bytes: &[u8], length: usize, eof: bool) -> Option<bool> {
    match bytes.get(length) {
        None => {
            if eof {
                Some(true)
            } else {
                None
            }
        }
        Some(b' ' | b'\t' | b'\r' | b'\n') => Some(true),
        Some(b'-') => match bytes.get(length + 1) {
            Some(b'-') => Some(true),
            None if !eof => None,
            _ => Some(false),
        },
        _ => Some(false),
    }
}
fn scan_body(bytes: &[u8], dash: &[u8], delimiter: &[u8], at_start: bool, eof: bool) -> Scan {
    if at_start {
        if bytes.starts_with(dash) {
            return match after_prefix(bytes, dash.len(), eof) {
                Some(false) => Scan {
                    bytes: dash.len(),
                    boundary: false,
                },
                Some(true) => Scan {
                    bytes: 0,
                    boundary: true,
                },
                None => Scan {
                    bytes: 0,
                    boundary: false,
                },
            };
        }
        if dash.starts_with(bytes) {
            return Scan {
                bytes: 0,
                boundary: false,
            };
        }
    }
    if let Some(n) = bytes
        .windows(delimiter.len())
        .position(|window| window == delimiter)
    {
        return match after_prefix(&bytes[n..], delimiter.len(), eof) {
            Some(false) => Scan {
                bytes: n + delimiter.len(),
                boundary: false,
            },
            Some(true) => Scan {
                bytes: n,
                boundary: true,
            },
            None => Scan {
                bytes: n,
                boundary: false,
            },
        };
    }
    if let Some(n) = bytes.iter().rposition(|b| *b == delimiter[0])
        && delimiter.starts_with(&bytes[n..])
    {
        return Scan {
            bytes: n,
            boundary: false,
        };
    }
    Scan {
        bytes: bytes.len(),
        boundary: false,
    }
}
