#![forbid(unsafe_code)]

//! Transparent calls to the fixed real standard BTreeSet.
//! These are API preflights, not replacements for the production validators.
use std::collections::BTreeSet;

pub fn insert_u64(seen: &mut BTreeSet<u64>, value: u64) -> bool {
    seen.insert(value)
}

pub fn insert_str<'a>(seen: &mut BTreeSet<&'a str>, value: &'a str) -> bool {
    seen.insert(value)
}

pub fn insert_lane<'a>(
    seen: &mut BTreeSet<(&'a str, u16)>,
    node: &'a str,
    slot: u16,
) -> bool {
    seen.insert((node, slot))
}

pub fn repeat_str<'a>(seen: &mut BTreeSet<&'a str>, value: &'a str) -> (bool, bool) {
    let first = seen.insert(value);
    let second = seen.insert(value);
    (first, second)
}

pub fn new_then_repeat_str(value: &str) -> (bool, bool) {
    let mut seen = BTreeSet::new();
    let first = seen.insert(value);
    let second = seen.insert(value);
    (first, second)
}
