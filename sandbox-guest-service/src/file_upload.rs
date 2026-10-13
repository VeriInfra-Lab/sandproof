//! Upload request header decisions. The caller preserves encoding/body-header
//! -> account lookup -> metadata -> content-type order and bounds HTTP parsing.
use axum::http::HeaderMap;
use sandproof_guest_runtime::file_upload::{self, Error};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
pub struct HeaderLimits {
    pub bytes: usize,
    pub fields: usize,
}

/// Check the complete map, including repeated/ignored fields, before cloning.
/// # Errors
/// Invalid configuration or exceeded accepted-header budget.
pub fn check_headers(headers: &HeaderMap, limits: HeaderLimits) -> Result<(), Error> {
    if limits.bytes == 0
        || limits.fields == 0
        || limits.bytes > isize::MAX as usize / 16
        || limits.fields > isize::MAX as usize / 16
    {
        return Err(Error::InvalidConfiguration);
    }
    if headers.len() > limits.fields {
        return Err(Error::HeaderLimit);
    }
    headers.iter().try_fold(0usize, |total, (key, value)| {
        total
            .checked_add(key.as_str().len())
            .and_then(|n| n.checked_add(value.len()))
            .filter(|n| *n <= limits.bytes)
            .ok_or(Error::HeaderLimit)
    })?;
    Ok(())
}

/// Select the first value of each canonical X-Metadata-* field. HTTP `HeaderMap`
/// already normalizes case; the empty suffix is ignored as in fixed envd.
/// # Errors
/// Invalid metadata or aggregate payload, before copying any oversized value.
pub fn metadata(headers: &HeaderMap) -> Result<BTreeMap<String, String>, Error> {
    let mut values = BTreeMap::new();
    let mut total = 0usize;
    for name in headers.keys() {
        let Some(key) = name
            .as_str()
            .strip_prefix("x-metadata-")
            .filter(|key| !key.is_empty())
        else {
            continue;
        };
        let Some(value) = headers.get(name) else {
            continue;
        };
        file_upload::validate_entry(key.as_bytes(), value.as_bytes())?;
        total = total
            .checked_add(9 + key.len())
            .and_then(|n| n.checked_add(value.len()))
            .filter(|n| *n <= 4096)
            .ok_or(Error::InvalidMetadata)?;
        values.insert(
            key.to_owned(),
            std::str::from_utf8(value.as_bytes())
                .map_err(|_| Error::InvalidMetadata)?
                .to_owned(),
        );
    }
    Ok(values)
}
#[cfg(test)]
mod tests;
