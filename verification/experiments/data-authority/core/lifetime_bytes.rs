//! Functional byte equality for the stored lifetime owner.
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
