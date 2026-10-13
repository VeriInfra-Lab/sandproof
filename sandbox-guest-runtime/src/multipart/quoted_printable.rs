//! Fixed Go quoted-printable reader behavior, including partial errors. The
//! Go reader uses a 4096-byte physical line buffer; this decoder does too.
// Adapted from Go 1.26.8 mime/quotedprintable.
// Copyright 2012 The Go Authors. All rights reserved. BSD terms are in GO-LICENSE.
use super::Error;

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
        let mut output = Decoded {
            bytes: Vec::new(),
            error: None,
        };
        if self.failed || self.ended {
            output.error = Some(Error::Poisoned);
            return output;
        }
        if input.len() > self.chunk_bytes {
            self.failed = true;
            output.error = Some(Error::ResourceExhausted);
            return output;
        }
        for byte in input {
            // ReadSlice checks the underlying terminal error before declaring
            // its buffer full: exactly 4096 bytes at valid EOF still succeeds.
            if self.line.len() == 4096 {
                output.error = Some(
                    decode_line(&self.line, false, &mut output.bytes).unwrap_or(Error::Malformed),
                );
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
                    output.error = error;
                    break;
                }
            }
        }
        output
    }
    /// Call at a valid part end. A raw transport error is not a valid end.
    pub fn finish(&mut self) -> Decoded {
        self.end(None)
    }
    /// Preserve the decodable prefix on a failed underlying reader, then report
    /// failure. This never turns a truncated part into successful EOF.
    pub fn abort(&mut self, error: Error) -> Decoded {
        self.end(Some(error))
    }
    fn end(&mut self, upstream: Option<Error>) -> Decoded {
        if self.failed || self.ended {
            return Decoded {
                bytes: Vec::new(),
                error: Some(Error::Poisoned),
            };
        }
        self.ended = true;
        let mut bytes = Vec::new();
        let error = decode_line(&self.line, upstream.is_none(), &mut bytes).or(upstream);
        self.line.clear();
        self.failed = error.is_some();
        Decoded { bytes, error }
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
fn decode_line(line: &[u8], eof: bool, output: &mut Vec<u8>) -> Option<Error> {
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
            pending = Some(Error::Malformed);
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
                return Some(Error::Malformed);
            }
        } else if b < 32 && !matches!(b, b'\t' | b'\r' | b'\n') || b == 127 {
            return Some(Error::Malformed);
        }
        output.push(b);
        n += 1;
    }
    output.extend_from_slice(newline);
    pending
}
