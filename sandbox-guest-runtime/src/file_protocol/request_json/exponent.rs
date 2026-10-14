//! The fixed protobuf tokenizer consumes an exponent marker without digits
//! only when a following delimiter is present. Its integer conversion treats
//! a bare e/E as absent, but rejects a sign without digits. Unknown fields skip
//! both forms. Normalize these tokens for Serde without changing string input,
//! field order, byte offsets or the original length/budget.
use serde_json::value::RawValue;

pub(super) enum Repair {
    Bare,
    Signed,
}

impl Repair {
    pub(super) fn apply(self, token: &mut [u8]) {
        match self {
            Self::Bare => token[token.len() - 1] = b' ',
            Self::Signed => {
                // Every known field in these five file messages is either a
                // string or uint32. A negative integer is rejected by both;
                // discarded fields still accept it. Do not reuse this adapter
                // for schemas with signed/floating-point fields.
                token.fill(b' ');
                token[..2].copy_from_slice(b"-1");
            }
        }
    }
}

/// Consume each numeric-looking span once, even when no repair is needed.
/// Advancing only one digit would rescan long mantissas quadratically.
pub(super) fn scan(input: &[u8]) -> (usize, Option<Repair>) {
    let mut end = usize::from(input.first() == Some(&b'-'));
    digits(input, &mut end);
    if input.get(end) == Some(&b'.') {
        end += 1;
        digits(input, &mut end);
    }
    let mantissa_end = end;
    if !matches!(input.get(end), Some(b'e' | b'E')) {
        return (end.max(1), None);
    }
    end += 1;
    let signed = matches!(input.get(end), Some(b'+' | b'-'));
    end += usize::from(signed);
    let exponent_start = end;
    digits(input, &mut end);
    // EOF does not provide the lookahead required by the original parser.
    // Go's delimiter class is broader than valid JSON punctuation; Serde still
    // rejects any invalid punctuation/sequence after the repaired token.
    let delimiter = input.get(end).is_some_and(|byte| {
        !byte.is_ascii_alphanumeric() && !matches!(byte, b'-' | b'+' | b'.' | b'_')
    });
    let repair = (end == exponent_start
        && delimiter
        && serde_json::from_slice::<&RawValue>(&input[..mantissa_end]).is_ok())
    .then_some(if signed { Repair::Signed } else { Repair::Bare });
    (end, repair)
}

fn digits(input: &[u8], end: &mut usize) {
    while input.get(*end).is_some_and(u8::is_ascii_digit) {
        *end += 1;
    }
}
