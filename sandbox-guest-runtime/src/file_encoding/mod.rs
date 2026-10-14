//! Fixed envd file Content-Encoding/Accept-Encoding selection, not a router.
//! Callers pass the FIRST HTTP header value (Go Header.Get semantics), including
//! opaque bytes. Request/header allocation, auth, stat and response work belong
//! to the caller. Component limits are explicit; there are no production defaults.
// Behavior ported from e2b-dev/runtime f455fd58fa3f289e78ce1de690c35968d8d29322,
// packages/envd/internal/api/encoding.go (Apache-2.0, E2B-LICENSE).
mod quality;
mod sort;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Identity,
    Gzip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidConfiguration,
    HeaderLimit,
    ItemLimit,
    Unsupported,
    NotAcceptable,
}

/// Bounds raw header bytes and comma-separated entries before allocation.
/// The caller must separately bound all request headers, including duplicates.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub header_bytes: usize,
    pub items: usize,
}
impl Limits {
    fn check(self, header: &[u8]) -> Result<(), Error> {
        if self.header_bytes == 0
            || self.header_bytes > isize::MAX as usize / 16
            || self.items == 0
            || self.items > isize::MAX as usize / 16
        {
            return Err(Error::InvalidConfiguration);
        }
        if header.len() > self.header_bytes {
            return Err(Error::HeaderLimit);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Name {
    Identity,
    Gzip,
    Wildcard,
    Other,
}
#[derive(Clone, Copy)]
struct Item {
    name: Name,
    quality: f64,
}

fn name(value: &[u8]) -> Name {
    let value = trim(value);
    if eq_name(value, b"identity") {
        Name::Identity
    } else if eq_name(value, b"gzip") {
        Name::Gzip
    } else if value == b"*" {
        Name::Wildcard
    } else {
        Name::Other
    }
}
fn eq_name(mut value: &[u8], expected: &[u8]) -> bool {
    for byte in expected {
        if *byte == b'i' && value.starts_with(b"\xc4\xb0") {
            // Go unicode simple lower maps capital dotted I to ASCII i.
            value = &value[2..];
        } else if value
            .first()
            .is_some_and(|value| value.eq_ignore_ascii_case(byte))
        {
            value = &value[1..];
        } else {
            return false;
        }
    }
    value.is_empty()
}
fn item(value: &[u8]) -> Item {
    let mut parts = trim(value).split(|byte| *byte == b';');
    let name = name(parts.next().unwrap_or_default());
    let mut quality = 1.0;
    for param in parts {
        let param = trim(param);
        if param.len() >= 2
            && param[0].eq_ignore_ascii_case(&b'q')
            && param[1] == b'='
            && let Some(value) = quality::parse(&param[2..])
        {
            quality = value;
        }
    }
    Item { name, quality }
}

/// Missing or exactly empty selects identity; otherwise strip semicolon
/// parameters and accept only identity/gzip. Comma lists are not split here.
/// # Errors
/// Invalid component bounds, header byte budget or unsupported coding.
pub fn content(header: &[u8], limits: Limits) -> Result<Encoding, Error> {
    limits.check(header)?;
    if header.is_empty() {
        return Ok(Encoding::Identity);
    }
    match name(
        trim(header)
            .split(|byte| *byte == b';')
            .next()
            .unwrap_or_default(),
    ) {
        Name::Identity => Ok(Encoding::Identity),
        Name::Gzip => Ok(Encoding::Gzip),
        _ => Err(Error::Unsupported),
    }
}

/// Selected coding and the separate identity veto used by conditional GET.
/// An explicit nonzero identity item can be selected even when another zero
/// item vetoes identity for range/conditional fallback in the fixed endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accepted {
    pub encoding: Encoding,
    pub identity_acceptable: bool,
}

/// Reproduces the pinned Go parser, including float and unstable tie behavior.
/// This is deliberately not a new strict RFC quality-value validator.
/// # Errors
/// Invalid bounds, raw bytes/item budget, or no acceptable coding.
pub fn accept(header: &[u8], limits: Limits) -> Result<Accepted, Error> {
    limits.check(header)?;
    if header.is_empty() {
        return Ok(Accepted {
            encoding: Encoding::Identity,
            identity_acceptable: true,
        });
    }
    let count = header.split(|byte| *byte == b',').count();
    if count > limits.items {
        return Err(Error::ItemLimit);
    }
    let mut items = Vec::with_capacity(count);
    items.extend(header.split(|byte| *byte == b',').map(item));
    let mut identity_rejected = false;
    let mut identity_explicit = false;
    let mut wildcard_rejected = false;
    for item in &items {
        match item.name {
            Name::Identity if item.quality == 0.0 => identity_rejected = true,
            Name::Identity => identity_explicit = true,
            Name::Wildcard if item.quality == 0.0 => wildcard_rejected = true,
            _ => {}
        }
    }
    identity_rejected |= wildcard_rejected && !identity_explicit;
    sort::sort(&mut items);
    for item in items {
        if item.quality == 0.0 {
            continue;
        }
        let encoding = match item.name {
            Name::Gzip => Encoding::Gzip,
            Name::Wildcard if identity_rejected => Encoding::Gzip,
            Name::Identity | Name::Wildcard => Encoding::Identity,
            Name::Other => continue,
        };
        return Ok(Accepted {
            encoding,
            identity_acceptable: !identity_rejected,
        });
    }
    if identity_rejected {
        Err(Error::NotAcceptable)
    } else {
        Ok(Accepted {
            encoding: Encoding::Identity,
            identity_acceptable: true,
        })
    }
}

/// Fixed `GetFiles` forces identity for any NONEMPTY first Range,
/// If-Modified-Since, If-None-Match or If-Range value, even malformed/whitespace.
/// If-Match and If-Unmodified-Since are intentionally absent from this rule.
/// Caller must apply this AFTER successful stat/directory checking and before
/// open; a successful selection does not implement `ServeContent` conditions.
/// # Errors
/// Original accept errors, or identity forbidden for conditional/range work.
pub fn download(header: &[u8], force_identity: bool, limits: Limits) -> Result<Encoding, Error> {
    let accepted = accept(header, limits)?;
    if force_identity {
        if !accepted.identity_acceptable {
            return Err(Error::NotAcceptable);
        }
        Ok(Encoding::Identity)
    } else {
        Ok(accepted.encoding)
    }
}

// strings.TrimSpace's Unicode White_Space set, preserving opaque non-UTF8
// bytes elsewhere. Lowercasing need only recognize the three ASCII names.
fn trim(mut value: &[u8]) -> &[u8] {
    const SPACES: &[&[u8]] = &[
        b"\t",
        b"\n",
        b"\x0b",
        b"\x0c",
        b"\r",
        b" ",
        b"\xc2\x85",
        b"\xc2\xa0",
        b"\xe1\x9a\x80",
        b"\xe2\x80\x80",
        b"\xe2\x80\x81",
        b"\xe2\x80\x82",
        b"\xe2\x80\x83",
        b"\xe2\x80\x84",
        b"\xe2\x80\x85",
        b"\xe2\x80\x86",
        b"\xe2\x80\x87",
        b"\xe2\x80\x88",
        b"\xe2\x80\x89",
        b"\xe2\x80\x8a",
        b"\xe2\x80\xa8",
        b"\xe2\x80\xa9",
        b"\xe2\x80\xaf",
        b"\xe2\x81\x9f",
        b"\xe3\x80\x80",
    ];
    while let Some(space) = SPACES.iter().find(|space| value.starts_with(space)) {
        value = &value[space.len()..];
    }
    while let Some(space) = SPACES.iter().find(|space| value.ends_with(space)) {
        value = &value[..value.len() - space.len()];
    }
    value
}
