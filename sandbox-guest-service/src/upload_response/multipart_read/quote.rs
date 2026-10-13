//! Streaming Go 1.26.8 strconv.Quote, including malformed UTF-8 and `IsPrint`.
// Copyright 2009 The Go Authors. All rights reserved.
// BSD license: ../../../../sandbox-guest-runtime/src/multipart/GO-LICENSE.
use super::tables::{NOT_PRINT16, NOT_PRINT32, PRINT16, PRINT32};
use crate::{file_download::Error, go_json::Sink};

pub(super) fn quoted(bytes: &[u8], put: &mut Sink<'_>) -> Result<(), Error> {
    put(b"\"")?;
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            match c {
                '"' => put(b"\\\"")?,
                '\\' => put(b"\\\\")?,
                c if printable(c as u32) => put(c.encode_utf8(&mut [0; 4]).as_bytes())?,
                '\u{7}' => put(b"\\a")?,
                '\u{8}' => put(b"\\b")?,
                '\u{c}' => put(b"\\f")?,
                '\n' => put(b"\\n")?,
                '\r' => put(b"\\r")?,
                '\t' => put(b"\\t")?,
                '\u{b}' => put(b"\\v")?,
                c if c < ' ' || c == '\u{7f}' => escaped(c as u32, b'x', 2, put)?,
                c if c < '\u{10000}' => escaped(c as u32, b'u', 4, put)?,
                c => escaped(c as u32, b'U', 8, put)?,
            }
        }
        for byte in chunk.invalid() {
            escaped(u32::from(*byte), b'x', 2, put)?;
        }
    }
    put(b"\"")
}
fn escaped(value: u32, kind: u8, digits: usize, put: &mut Sink<'_>) -> Result<(), Error> {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = [0u8; 10];
    out[0] = b'\\';
    out[1] = kind;
    for index in 0..digits {
        out[index + 2] = HEX[((value >> (4 * (digits - index - 1))) & 15) as usize];
    }
    put(&out[..digits + 2])
}
fn printable(value: u32) -> bool {
    if value <= 0xff {
        return (0x20..=0x7e).contains(&value) || (value >= 0xa1 && value != 0xad);
    }
    let (ranges, exceptions, exception) = if value < 0x0001_0000 {
        (PRINT16, NOT_PRINT16, value)
    } else {
        (PRINT32, NOT_PRINT32, value - 0x0001_0000)
    };
    let i = ranges.partition_point(|v| *v < value);
    i < ranges.len()
        && ranges[i & !1] <= value
        && value <= ranges[i | 1]
        && (value >= 0x0002_0000 || exceptions.binary_search(&exception).is_err())
}
