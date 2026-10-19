// Fixed protobuf v1.36.12 JSON Unicode/escape failure ordering.
// Copyright 2018 The Go Authors. All rights reserved.
// BSD license: ../PROTOBUF-LICENSE.
use super::{Failure, JsonReason, rune_length};

pub(in super::super) fn length(input: &[u8], start: usize) -> Result<usize, Failure> {
    let mut at = 1;
    while let Some(byte) = input.get(at) {
        match byte {
            b'"' => return Ok(at + 1),
            0..=0x1f => return Err(Failure::at(JsonReason::InvalidCharacter(*byte), start)),
            b'\\' => {
                let Some(escape) = input.get(at + 1) else {
                    return Err(Failure::at(JsonReason::UnexpectedEof, start));
                };
                match escape {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => at += 2,
                    b'u' => {
                        let high = hex(input, at, start)?;
                        at += 6;
                        if (0xd800..=0xdfff).contains(&high) {
                            let low = hex(input, at, start)?;
                            if !input[at..].starts_with(b"\\u")
                                || !(0xd800..=0xdbff).contains(&high)
                                || !(0xdc00..=0xdfff).contains(&low)
                            {
                                return Err(escape_error(start, at, 6));
                            }
                            at += 6;
                        }
                    }
                    _ => return Err(escape_error(start, at, 2)),
                }
            }
            _ => {
                at += rune_length(&input[at..])
                    .ok_or_else(|| Failure::at(JsonReason::InvalidUtf8, start))?;
            }
        }
    }
    Err(Failure::at(JsonReason::UnexpectedEof, start))
}

fn hex(input: &[u8], at: usize, start: usize) -> Result<u16, Failure> {
    let Some(digits) = input.get(at..at + 6) else {
        return Err(Failure::at(JsonReason::UnexpectedEof, start));
    };
    let mut value = 0_u16;
    for byte in &digits[2..] {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return Err(escape_error(start, at, 6)),
        };
        value = value * 16 + u16::from(digit);
    }
    Ok(value)
}
fn escape_error(start: usize, at: usize, len: usize) -> Failure {
    Failure {
        reason: JsonReason::InvalidEscape,
        position: start,
        raw: start + at..start + at + len,
    }
}
