#[must_use]
#[expect(
    clippy::manual_range_contains,
    reason = "fixed Charon/Aeneas lacks ASCII/range method models; exact byte bounds are proved"
)]
pub fn is_hex(byte: u8) -> bool {
    (byte >= b'0' && byte <= b'9')
        || (byte >= b'a' && byte <= b'f')
        || (byte >= b'A' && byte <= b'F')
}

#[must_use]
pub fn token_format(token: &[u8]) -> bool {
    if token.len() != 64 {
        return false;
    }
    let mut index = 0;
    while index < token.len() {
        if !is_hex(token[index]) {
            return false;
        }
        index += 1;
    }
    true
}

#[must_use]
pub fn same_secret(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    let mut index = 0;
    while index < left.len() {
        difference |= left[index] ^ right[index];
        index += 1;
    }
    difference == 0
}
