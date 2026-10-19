use super::{
    Error, Limits,
    date::{self, LocalZone, Timestamp},
    trim,
};

/// The caller passes first values, preserving exactly empty versus nonempty.
#[derive(Default, Clone, Copy)]
pub struct Conditions<'a> {
    pub if_match: &'a [u8],
    pub if_unmodified_since: &'a [u8],
    pub if_none_match: &'a [u8],
    pub if_modified_since: &'a [u8],
    pub if_range: &'a [u8],
    pub range: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precondition {
    /// True retains Range; false ignores it after If-Range mismatch/invalidity.
    Proceed { use_range: bool },
    /// 304 removes type/length/encoding; also removes Last-Modified if `ETag` set.
    NotModified { keep_last_modified: bool },
    /// 412 with no entity reads. Original entity metadata remains set.
    Failed,
}

/// Evaluate BEFORE MIME sniffing/length seek or any entity reads, after the
/// caller opens the original descriptor. `ETag` is the trusted response value,
/// empty for fixed envd; it is not an extra client-supplied selector.
/// # Errors
/// Invalid budgets or total selected header / scanned tag count exhaustion.
pub fn precondition(
    method: &[u8],
    headers: Conditions<'_>,
    modified: Timestamp,
    etag: &[u8],
    limits: Limits,
    local: &impl LocalZone,
) -> Result<Precondition, Error> {
    limits.check(&[
        headers.if_match,
        headers.if_unmodified_since,
        headers.if_none_match,
        headers.if_modified_since,
        headers.if_range,
        headers.range,
        etag,
        method,
    ])?;
    let get_head = method == b"GET" || method == b"HEAD";
    // A nonempty If-Match (even malformed) suppresses If-Unmodified-Since.
    if !headers.if_match.is_empty() {
        if !tags_match(headers.if_match, etag, true, limits.items)? {
            return Ok(Precondition::Failed);
        }
    } else if !headers.if_unmodified_since.is_empty()
        && !modified.unspecified()
        && let Some(date) = date::parse(headers.if_unmodified_since, local)
        && modified.truncate() > date
    {
        return Ok(Precondition::Failed);
    }

    if !headers.if_none_match.is_empty() {
        if tags_match(headers.if_none_match, etag, false, limits.items)? {
            return Ok(if get_head {
                Precondition::NotModified {
                    keep_last_modified: etag.is_empty(),
                }
            } else {
                Precondition::Failed
            });
        }
    } else if get_head
        && !modified.unspecified()
        && let Some(date) = date::parse(headers.if_modified_since, local)
        && modified.truncate() <= date
    {
        return Ok(Precondition::NotModified {
            keep_last_modified: etag.is_empty(),
        });
    }
    // Date If-Range must use the same local zone snapshot as other conditions.
    let use_range = if !headers.range.is_empty() && !headers.if_range.is_empty() && get_head {
        match scan_tag(headers.if_range) {
            Some((tag, _)) => strong(tag, etag),
            None if modified.go_zero() => false,
            None => date::parse(headers.if_range, local)
                .is_some_and(|date| date.seconds() == modified.seconds()),
        }
    } else {
        true
    };
    Ok(Precondition::Proceed { use_range })
}

fn strong(a: &[u8], b: &[u8]) -> bool {
    a == b && a.first() == Some(&b'"')
}
fn weak(a: &[u8], b: &[u8]) -> bool {
    a.strip_prefix(b"W/").unwrap_or(a) == b.strip_prefix(b"W/").unwrap_or(b)
}

fn tags_match(
    mut value: &[u8],
    etag: &[u8],
    strong_only: bool,
    limit: usize,
) -> Result<bool, Error> {
    let mut scanned = 0;
    loop {
        value = trim(value);
        if value.is_empty() {
            return Ok(false);
        }
        scanned += 1;
        if scanned > limit {
            return Err(Error::ItemLimit);
        }
        if value[0] == b',' {
            value = &value[1..];
            continue;
        }
        if value[0] == b'*' {
            return Ok(true);
        }
        let Some((tag, rest)) = scan_tag(value) else {
            return Ok(false);
        };
        if if strong_only {
            strong(tag, etag)
        } else {
            weak(tag, etag)
        } {
            return Ok(true);
        }
        value = rest;
    }
}

fn scan_tag(value: &[u8]) -> Option<(&[u8], &[u8])> {
    let value = trim(value);
    let start = if value.starts_with(b"W/") { 2 } else { 0 };
    if value.len() < start + 2 || value[start] != b'"' {
        return None;
    }
    for (index, &byte) in value.iter().enumerate().skip(start + 1) {
        if byte == b'"' {
            return Some((&value[..=index], &value[index + 1..]));
        }
        if !(byte == 0x21 || (0x23..=0x7e).contains(&byte) || byte >= 0x80) {
            return None;
        }
    }
    None
}
