//! Fixed Go quoted-printable reader behavior, including partial errors. The
//! Go reader uses a 4096-byte physical line buffer; this decoder does too.
// Adapted from Go 1.26.8 mime/quotedprintable.
// Copyright 2012 The Go Authors. All rights reserved. BSD terms are in GO-LICENSE.
use super::Error;

/// Terminal observation accompanying the last raw bytes of a part read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartReadEnd {
    More,
    Eof,
    Failed(Error),
}

/// Observed fixed-Go reader failures, distinct from caller budgets or misuse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuotedPrintableError {
    UnexpectedEof,
    InvalidHex(u8),
    InvalidUnescaped(u8),
    /// The actual suffix after trimming only leading SP/TAB; at most 4096 bytes.
    InvalidSoftBreak(Vec<u8>),
    LineTooLong,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuotedPrintableReadError {
    Decode(Error),
    Encoding(QuotedPrintableError),
}
impl QuotedPrintableReadError {
    #[must_use]
    pub fn coarse(self) -> Error {
        match self {
            Self::Decode(error) => error,
            Self::Encoding(_) => Error::Malformed,
        }
    }
}
pub struct DecodedDetailed {
    pub bytes: Vec<u8>,
    /// Write/observe these bytes before reporting the terminal failure.
    pub error: Option<QuotedPrintableReadError>,
}
impl DecodedDetailed {
    fn coarse(self) -> Decoded {
        Decoded {
            bytes: self.bytes,
            error: self.error.map(QuotedPrintableReadError::coarse),
        }
    }
}
pub struct Decoded {
    pub bytes: Vec<u8>,
    /// Write/observe the preceding decoded prefix, then fail the part.
    pub error: Option<Error>,
}
#[derive(Debug)]
pub struct QuotedPrintable {
    line: Vec<u8>,
    chunk_bytes: usize,
    failed: bool,
    ended: bool,
}
impl QuotedPrintable {
    pub(super) fn read_capacity(&self) -> usize {
        4096 - self.line.len()
    }
    /// # Errors
    /// Invalid input-chunk budget.
    pub fn new(chunk_bytes: usize) -> Result<Self, Error> {
        if chunk_bytes == 0 || chunk_bytes > isize::MAX as usize / 16 {
            return Err(Error::InvalidConfiguration);
        }
        Ok(Self {
            line: Vec::with_capacity(4096),
            chunk_bytes,
            failed: false,
            ended: false,
        })
    }
    /// Bounded by input chunk plus one physical line; never accumulates a file.
    pub fn feed(&mut self, input: &[u8]) -> Decoded {
        self.feed_detailed(input).coarse()
    }
    /// Like `feed`, retaining the actual bounded reader diagnostic.
    pub fn feed_detailed(&mut self, input: &[u8]) -> DecodedDetailed {
        let mut output = DecodedDetailed {
            bytes: Vec::new(),
            error: None,
        };
        if self.failed || self.ended {
            output.error = Some(QuotedPrintableReadError::Decode(Error::Poisoned));
            return output;
        }
        if input.len() > self.chunk_bytes {
            self.failed = true;
            output.error = Some(QuotedPrintableReadError::Decode(Error::ResourceExhausted));
            return output;
        }
        for byte in input {
            // ReadSlice checks the underlying terminal error before declaring
            // its buffer full: exactly 4096 bytes at valid EOF still succeeds.
            if self.line.len() == 4096 {
                output.error = Some(QuotedPrintableReadError::Encoding(
                    decode_line(&self.line, false, &mut output.bytes)
                        .unwrap_or(QuotedPrintableError::LineTooLong),
                ));
                self.line.clear();
                self.failed = true;
                break;
            }
            self.line.push(*byte);
            if *byte == b'\n' {
                let error = decode_line(&self.line, false, &mut output.bytes);
                self.line.clear();
                if error.is_some() {
                    self.failed = true;
                    output.error = error.map(QuotedPrintableReadError::Encoding);
                    break;
                }
            }
        }
        output
    }
    /// Decode raw part bytes with the terminal fact from that same read. Unlike
    /// the legacy event-only API, a full physical line cannot wait for another
    /// read to discover EOF. Feed artificial fragments with More, except that
    /// the final fragment must carry any observed EOF/failure.
    pub fn feed_reader(&mut self, input: &[u8], end: PartReadEnd) -> DecodedDetailed {
        let mut output = self.feed_detailed(input);
        if output.error.is_some() {
            return output;
        }
        match end {
            PartReadEnd::More if self.line.len() == 4096 => {
                output.error = Some(QuotedPrintableReadError::Encoding(
                    decode_line(&self.line, false, &mut output.bytes)
                        .unwrap_or(QuotedPrintableError::LineTooLong),
                ));
                self.line.clear();
                self.failed = true;
            }
            PartReadEnd::More => {}
            PartReadEnd::Eof | PartReadEnd::Failed(_) => {
                let upstream = if let PartReadEnd::Failed(error) = end {
                    Some(error)
                } else {
                    None
                };
                output.error = decode_line(&self.line, upstream.is_none(), &mut output.bytes)
                    .map(QuotedPrintableReadError::Encoding)
                    .or(upstream.map(QuotedPrintableReadError::Decode));
                self.line.clear();
                self.ended = true;
                self.failed = output.error.is_some();
            }
        }
        output
    }

    /// Call at a valid part end. A raw transport error is not a valid end.
    pub fn finish(&mut self) -> Decoded {
        self.finish_detailed().coarse()
    }
    /// Detailed success-end path. An underlying reader failure must use abort.
    pub fn finish_detailed(&mut self) -> DecodedDetailed {
        self.end(None)
    }
    /// Preserve the decodable prefix on a failed underlying reader, then report
    /// failure. This never turns a truncated part into successful EOF.
    pub fn abort(&mut self, error: Error) -> Decoded {
        self.abort_detailed(error).coarse()
    }
    /// Flush on source failure without accepting a trailing '=' as valid EOF.
    /// An encoding error can precede the supplied source failure; otherwise the
    /// original component error is retained. Local admission failures are not
    /// source observations and must not be presented as Go reader diagnostics.
    pub fn abort_detailed(&mut self, error: Error) -> DecodedDetailed {
        self.end(Some(error))
    }
    fn end(&mut self, upstream: Option<Error>) -> DecodedDetailed {
        if self.failed || self.ended {
            return DecodedDetailed {
                bytes: Vec::new(),
                error: Some(QuotedPrintableReadError::Decode(Error::Poisoned)),
            };
        }
        self.ended = true;
        let mut bytes = Vec::new();
        // A failed source is reported after the previous Data event, so a
        // full unterminated line reaches the reader's buffer error first. A
        // valid PartEnd remains explicit EOF (see the documented Go source
        // buffer alignment difference for that exact limit).
        let error = decode_line(&self.line, upstream.is_none(), &mut bytes)
            .or_else(|| {
                (upstream.is_some() && self.line.len() == 4096)
                    .then_some(QuotedPrintableError::LineTooLong)
            })
            .map(QuotedPrintableReadError::Encoding)
            .or(upstream.map(QuotedPrintableReadError::Decode));
        self.line.clear();
        self.failed = error.is_some();
        DecodedDetailed { bytes, error }
    }
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
fn decode_line(line: &[u8], eof: bool, output: &mut Vec<u8>) -> Option<QuotedPrintableError> {
    let end = line
        .iter()
        .rposition(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        .map_or(0, |n| n + 1);
    let mut data = &line[..end];
    let mut pending = None;
    let mut newline: &[u8] = b"";
    if data.ends_with(b"=") {
        data = &data[..data.len() - 1];
        let tail = super::trim_start_space(&line[end..]);
        if !(tail.starts_with(b"\n")
            || tail.starts_with(b"\r\n")
            || (tail.is_empty() && !data.is_empty() && eof))
        {
            pending = Some(QuotedPrintableError::InvalidSoftBreak(tail.to_vec()));
        }
    } else if line.ends_with(b"\n") {
        newline = if line.ends_with(b"\r\n") {
            b"\r\n"
        } else {
            b"\n"
        };
    }
    let mut n = 0;
    while n < data.len() {
        let b = data[n];
        if b == b'=' {
            let pair = data
                .get(n + 1)
                .copied()
                .and_then(hex)
                .zip(data.get(n + 2).copied().and_then(hex));
            if let Some((high, low)) = pair {
                output.push(high * 16 + low);
                n += 3;
                continue;
            }
            if data.get(n + 1).is_none_or(|b| matches!(b, b'\r' | b'\n')) {
                return Some(if data.len() - n < 3 {
                    QuotedPrintableError::UnexpectedEof
                } else {
                    QuotedPrintableError::InvalidHex(data[n + 1])
                });
            }
        } else if b < 32 && !matches!(b, b'\t' | b'\r' | b'\n') || b == 127 {
            return Some(QuotedPrintableError::InvalidUnescaped(b));
        }
        output.push(b);
        n += 1;
    }
    output.extend_from_slice(newline);
    pending
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_read_eof_and_failure_are_distinct_from_a_later_terminal_read() {
        for end in [
            PartReadEnd::More,
            PartReadEnd::Eof,
            PartReadEnd::Failed(Error::Truncated),
        ] {
            let mut qp = QuotedPrintable::new(4096).unwrap();
            let output = qp.feed_reader(&vec![b'x'; 4096], end);
            assert_eq!(output.bytes.len(), 4096);
            assert_eq!(
                output.error,
                match end {
                    PartReadEnd::More => Some(QuotedPrintableReadError::Encoding(
                        QuotedPrintableError::LineTooLong
                    )),
                    PartReadEnd::Eof => None,
                    PartReadEnd::Failed(error) => Some(QuotedPrintableReadError::Decode(error)),
                }
            );
            assert_eq!(
                qp.feed_reader(b"late", PartReadEnd::More).error,
                Some(QuotedPrintableReadError::Decode(Error::Poisoned))
            );
        }
    }
    #[test]
    fn detailed_errors_keep_prefix_precedence_and_poison_legacy_entry() {
        for (data, expected, prefix) in [
            (
                b"a\0rest".as_slice(),
                QuotedPrintableError::InvalidUnescaped(0),
                b"a".as_slice(),
            ),
            (b"a=\rX", QuotedPrintableError::InvalidHex(13), b"a"),
            (b"a==", QuotedPrintableError::UnexpectedEof, b"a"),
            (
                b"a=\r\r",
                QuotedPrintableError::InvalidSoftBreak(b"\r\r".to_vec()),
                b"a",
            ),
            // A body error wins over the already observed malformed suffix.
            (b"a\0=\r\r", QuotedPrintableError::InvalidUnescaped(0), b"a"),
        ] {
            for failed_source in [false, true] {
                let mut qp = QuotedPrintable::new(32).unwrap();
                let output = qp.feed_detailed(data);
                assert!(output.bytes.is_empty() && output.error.is_none());
                let output = if failed_source {
                    qp.abort_detailed(Error::Truncated)
                } else {
                    qp.finish_detailed()
                };
                assert_eq!(output.bytes, prefix);
                assert_eq!(
                    output.error,
                    Some(QuotedPrintableReadError::Encoding(expected.clone()))
                );
                assert_eq!(qp.feed(b"next").error, Some(Error::Poisoned));
                assert_eq!(qp.finish().error, Some(Error::Poisoned));
            }
        }
    }
    #[test]
    fn detailed_line_boundary_source_error_and_local_budget_stay_distinct() {
        for length in [4096, 4097] {
            let mut qp = QuotedPrintable::new(8192).unwrap();
            let output = qp.feed_detailed(&vec![b'x'; length]);
            if length == 4097 {
                assert_eq!(output.bytes.len(), 4096);
                assert_eq!(
                    output.error,
                    Some(QuotedPrintableReadError::Encoding(
                        QuotedPrintableError::LineTooLong
                    ))
                );
            } else {
                assert!(output.bytes.is_empty() && output.error.is_none());
                let output = qp.finish_detailed();
                assert_eq!(output.bytes.len(), 4096);
                assert!(output.error.is_none());
            }
        }
        let mut full = QuotedPrintable::new(4096).unwrap();
        full.feed_detailed(&vec![b'x'; 4096]);
        let failed = full.abort_detailed(Error::Truncated);
        assert_eq!(failed.bytes.len(), 4096);
        assert_eq!(
            failed.error,
            Some(QuotedPrintableReadError::Encoding(
                QuotedPrintableError::LineTooLong
            ))
        );
        for (data, encoding) in [(b"prefix=".as_slice(), true), (b"prefix=00", false)] {
            let mut qp = QuotedPrintable::new(32).unwrap();
            qp.feed_detailed(data);
            let output = qp.abort_detailed(Error::Truncated);
            assert_eq!(
                output.error,
                Some(if encoding {
                    QuotedPrintableReadError::Encoding(QuotedPrintableError::InvalidSoftBreak(
                        vec![],
                    ))
                } else {
                    QuotedPrintableReadError::Decode(Error::Truncated)
                })
            );
            assert_eq!(
                output.bytes,
                if encoding {
                    b"prefix".as_slice()
                } else {
                    b"prefix\0"
                }
            );
        }
        let mut qp = QuotedPrintable::new(2).unwrap();
        assert_eq!(
            qp.feed_detailed(b"bad").error,
            Some(QuotedPrintableReadError::Decode(Error::ResourceExhausted))
        );
        assert_eq!(qp.abort(Error::Truncated).error, Some(Error::Poisoned));
    }
}
