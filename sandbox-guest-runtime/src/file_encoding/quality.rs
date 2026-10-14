// Copyright 2009 The Go Authors. All rights reserved.
// Adapted readFloat/atofHex/underscoreOK from Go 1.26.8 internal/strconv.
// BSD terms: ../multipart/GO-LICENSE. Decimal rounding uses Rust's f64 parser.

pub(super) fn parse(raw: &[u8]) -> Option<f64> {
    if raw.eq_ignore_ascii_case(b"nan") {
        return Some(f64::NAN);
    }
    let (negative, unsigned) = match raw.first() {
        Some(b'-') => (true, &raw[1..]),
        Some(b'+') => (false, &raw[1..]),
        _ => (false, raw),
    };
    if unsigned.eq_ignore_ascii_case(b"inf") || unsigned.eq_ignore_ascii_case(b"infinity") {
        return Some(if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        });
    }
    if !underscore_ok(raw) {
        return None;
    }
    // The temporary normalization never exceeds the caller-bounded header.
    let clean: Vec<_> = unsigned.iter().copied().filter(|b| *b != b'_').collect();
    if clean.starts_with(b"0x") || clean.starts_with(b"0X") {
        return hex(&clean[2..], negative);
    }
    let clean = std::str::from_utf8(&clean).ok()?;
    if clean.starts_with(['+', '-']) {
        return None;
    }
    let value = clean.parse::<f64>().ok()?;
    // strconv overflow is an error (ignored q parameter), unlike literal Inf.
    // Signed NaN is not accepted by Go's special parser either.
    if !value.is_finite() {
        return None;
    }
    Some(if negative { -value } else { value })
}

fn underscore_ok(mut value: &[u8]) -> bool {
    if matches!(value.first(), Some(b'+' | b'-')) {
        value = &value[1..];
    }
    let hex = value.starts_with(b"0x") || value.starts_with(b"0X");
    let prefix = value.len() >= 2
        && value[0] == b'0'
        && matches!(value[1].to_ascii_lowercase(), b'b' | b'o' | b'x');
    let mut previous_digit = prefix;
    let mut underscore = false;
    for &byte in &value[if prefix { 2 } else { 0 }..] {
        if byte.is_ascii_digit() || hex && byte.is_ascii_hexdigit() {
            previous_digit = true;
            underscore = false;
        } else if byte == b'_' {
            if !previous_digit {
                return false;
            }
            previous_digit = false;
            underscore = true;
        } else {
            if underscore {
                return false;
            }
            previous_digit = false;
        }
    }
    !underscore
}

fn hex(value: &[u8], negative: bool) -> Option<f64> {
    let mut mantissa = 0_u64;
    let mut digits = 0_i64;
    let mut kept = 0_i64;
    let mut point = 0_i64;
    let mut dot = false;
    let mut saw_digit = false;
    let mut truncated = false;
    let mut index = 0;
    while let Some(&byte) = value.get(index) {
        if byte == b'.' {
            if dot {
                return None;
            }
            dot = true;
            point = digits;
        } else if let Some(digit) = char::from(byte).to_digit(16) {
            saw_digit = true;
            if digit == 0 && digits == 0 {
                point -= 1;
            } else {
                digits += 1;
                if kept < 16 {
                    mantissa = mantissa * 16 + u64::from(digit);
                    kept += 1;
                } else if digit != 0 {
                    truncated = true;
                }
            }
        } else {
            break;
        }
        index += 1;
    }
    if !saw_digit || !matches!(value.get(index), Some(b'p' | b'P')) {
        return None;
    }
    if !dot {
        point = digits;
    }
    index += 1;
    let exponent_sign = if value.get(index) == Some(&b'-') {
        index += 1;
        -1
    } else {
        if value.get(index) == Some(&b'+') {
            index += 1;
        }
        1
    };
    let mut exponent = 0_i64;
    let start = index;
    while let Some(byte) = value.get(index).filter(|byte| byte.is_ascii_digit()) {
        if exponent < 10000 {
            exponent = exponent * 10 + i64::from(*byte - b'0');
        }
        index += 1;
    }
    if index == start || index != value.len() {
        return None;
    }
    let exponent = if mantissa == 0 {
        0
    } else {
        (point - kept) * 4 + exponent * exponent_sign
    };
    hex_bits(mantissa, exponent, negative, truncated)
}

fn hex_bits(mut mantissa: u64, mut exponent: i64, negative: bool, truncated: bool) -> Option<f64> {
    exponent += 52;
    while mantissa != 0 && mantissa >> 54 == 0 {
        mantissa <<= 1;
        exponent -= 1;
    }
    if truncated {
        mantissa |= 1;
    }
    while mantissa >> 55 != 0 {
        mantissa = mantissa >> 1 | mantissa & 1;
        exponent += 1;
    }
    while mantissa > 1 && exponent < -1024 {
        mantissa = mantissa >> 1 | mantissa & 1;
        exponent += 1;
    }
    let mut round = mantissa & 3;
    mantissa >>= 2;
    round |= mantissa & 1;
    exponent += 2;
    if round == 3 {
        mantissa += 1;
        if mantissa == 1 << 53 {
            mantissa >>= 1;
            exponent += 1;
        }
    }
    if mantissa >> 52 == 0 {
        exponent = -1023;
    }
    if exponent > 1023 {
        return None;
    }
    let mut bits = mantissa & ((1 << 52) - 1);
    bits |= u64::try_from(exponent + 1023).ok()? << 52;
    if negative {
        bits |= 1 << 63;
    }
    Some(f64::from_bits(bits))
}
