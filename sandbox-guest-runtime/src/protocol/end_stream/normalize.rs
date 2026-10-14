//! encoding/json accepts invalid UTF-8 and unpaired UTF-16 surrogates inside
//! strings, replacing them with U+FFFD. Protobuf JSON has stricter rules, so
//! this bounded normalization is specific to EOS metadata validation.
use super::ProtocolError;

pub(super) fn json(input: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    let limit = input
        .len()
        .checked_mul(3)
        .filter(|n| isize::try_from(*n).is_ok())
        .ok_or(ProtocolError::ResourceExhausted)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(input.len())
        .map_err(|_| ProtocolError::ResourceExhausted)?;
    let mut offset = 0;
    let mut string = false;
    let mut depth = 0_usize;
    while let Some(&byte) = input.get(offset) {
        let mut length = 1;
        let mut replacement: Option<&[u8]> = None;
        if string {
            match byte {
                b'"' => string = false,
                b'\\' => {
                    length = 2.min(input.len() - offset);
                    if let Some(value) = unicode(&input[offset..]) {
                        length = 6;
                        if (0xd800..=0xdbff).contains(&value) {
                            if unicode(&input[offset + 6..])
                                .is_some_and(|next| (0xdc00..=0xdfff).contains(&next))
                            {
                                length = 12;
                            } else {
                                replacement = Some(br"\ufffd");
                            }
                        } else if (0xdc00..=0xdfff).contains(&value) {
                            replacement = Some(br"\ufffd");
                        }
                    }
                }
                0x80..=0xff => {
                    let width = match byte {
                        0xc2..=0xdf => 2,
                        0xe0..=0xef => 3,
                        0xf0..=0xf4 => 4,
                        _ => 0,
                    };
                    if width > 0
                        && input
                            .get(offset..offset + width)
                            .is_some_and(|bytes| std::str::from_utf8(bytes).is_ok())
                    {
                        length = width;
                    } else {
                        replacement = Some("\u{fffd}".as_bytes());
                    }
                }
                _ => {}
            }
        } else {
            match byte {
                b'"' => string = true,
                b'[' | b'{' => {
                    depth += 1;
                    if depth > 10_000 {
                        return Err(ProtocolError::InvalidArgument);
                    }
                }
                b']' | b'}' => {
                    depth = depth.checked_sub(1).ok_or(ProtocolError::InvalidArgument)?;
                }
                _ => {}
            }
        }
        append(
            &mut output,
            replacement.unwrap_or(&input[offset..offset + length]),
            limit,
        )?;
        offset += length;
    }
    Ok(output)
}
fn unicode(bytes: &[u8]) -> Option<u16> {
    let value = bytes.get(..6)?;
    if &value[..2] != br"\u" {
        return None;
    }
    value[2..].iter().try_fold(0_u16, |number, byte| {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return None,
        };
        Some(number * 16 + u16::from(digit))
    })
}
fn append(output: &mut Vec<u8>, bytes: &[u8], limit: usize) -> Result<(), ProtocolError> {
    let required = output
        .len()
        .checked_add(bytes.len())
        .filter(|n| *n <= limit)
        .ok_or(ProtocolError::ResourceExhausted)?;
    if required > output.capacity() {
        let capacity = output.capacity().saturating_mul(2).max(required).min(limit);
        output
            .try_reserve_exact(capacity - output.len())
            .map_err(|_| ProtocolError::ResourceExhausted)?;
    }
    output.extend_from_slice(bytes);
    Ok(())
}
