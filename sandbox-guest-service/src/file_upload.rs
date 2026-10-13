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
    metadata_detailed(headers).map_err(|_| Error::InvalidMetadata)
}

/// Owned diagnostic contains only a selected normalized header key, never its
/// value. The request owner must enforce the complete header budget first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataFault {
    pub(crate) key: String,
    pub(crate) reason: MetadataReason,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MetadataReason {
    Entry(file_upload::MetadataViolation),
    Total(usize),
}

fn entries(headers: &HeaderMap) -> impl Iterator<Item = (&str, &[u8])> {
    headers.keys().filter_map(|name| {
        let key = name
            .as_str()
            .strip_prefix("x-metadata-")
            .filter(|key| !key.is_empty())?;
        Some((key, headers.get(name)?.as_bytes()))
    })
}

/// Validate all entries before total-size rejection or value allocation. Go's
/// map order is unspecified when several entries are invalid; any encountered
/// entry failure precedes the aggregate-size failure.
/// # Errors
/// Invalid selected key/value or the exact stored-name/value total above 4096.
pub fn metadata_detailed(headers: &HeaderMap) -> Result<BTreeMap<String, String>, MetadataFault> {
    let mut total = 0usize;
    for (key, value) in entries(headers) {
        file_upload::validate_entry_detailed(key.as_bytes(), value).map_err(|reason| {
            MetadataFault {
                key: key.to_owned(),
                reason: MetadataReason::Entry(reason),
            }
        })?;
        // Every stored prefix is shorter than its original X-Metadata- field.
        // Admitted complete-header bytes bound this sum before this call.
        total = total
            .saturating_add(9 + key.len())
            .saturating_add(value.len());
    }
    if total > 4096 {
        return Err(MetadataFault {
            key: String::new(),
            reason: MetadataReason::Total(total),
        });
    }
    Ok(entries(headers)
        .map(|(key, value)| {
            (
                key.to_owned(),
                // Detailed validation has established printable ASCII for every byte.
                value.iter().map(|&byte| char::from(byte)).collect(),
            )
        })
        .collect())
}
#[cfg(test)]
mod tests;
