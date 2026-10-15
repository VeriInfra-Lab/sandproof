//! Immutable, explicitly supplied `TZif` data for fixed Go HTTP date parsing.
//! No environment reads, filesystem lookup, timezone refresh or UTC fallback.
//! Name/first-zone selection follows Go 1.26.8 time/zoneinfo.go (Copyright 2009
//! The Go Authors, BSD license: ../multipart/GO-LICENSE). tz-rs parses the
//! bounded data and evaluates its validated POSIX extension rule.
use super::date::LocalZone;
use tz::{TimeZone, TimeZoneRef, timezone::LocalTimeType};

#[cfg(unix)]
mod file;
#[cfg(test)]
mod tests;
#[cfg(unix)]
pub use file::{LoadError, LoadLimits};

/// Trusted startup budgets, separate from each request's date/header limits.
/// Input bytes also bound abbreviation/footer/leap data and parser scratch.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub input_bytes: usize,
    pub transitions: usize,
    pub zones: usize,
}
impl Limits {
    fn validate(self) -> Result<Self, Error> {
        if [self.input_bytes, self.transitions, self.zones]
            .into_iter()
            .any(|n| n == 0 || n > isize::MAX as usize / 16)
        {
            Err(Error::InvalidConfiguration)
        } else {
            Ok(self)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidConfiguration,
    InputLimit,
    TransitionLimit,
    ZoneLimit,
    InvalidData,
}

pub struct Snapshot {
    zone: TimeZone,
    first: usize,
}
impl Snapshot {
    /// Parse one already bounded, trusted timezone source. This retains only
    /// parsed data; the caller can release the original bytes. Invalid data is
    /// a startup error, never an implicit UTC snapshot. No request selects it.
    /// # Errors
    /// Invalid budgets, oversized data/counts or `TZif` rejected by the pinned
    /// parser. Supports its `TZif` v1/v2/v3 formats and valid extension rules.
    pub fn from_tzif(bytes: &[u8], limits: Limits) -> Result<Self, Error> {
        let limits = limits.validate()?;
        if bytes.len() > limits.input_bytes {
            return Err(Error::InputLimit);
        }
        // Inspect both block lengths and the selected counts BEFORE the parser
        // can allocate its transition/type vectors from the header values.
        let (version, first_length, mut counts) = block(bytes, 4)?;
        if matches!(version, b'2' | b'3') {
            let (second, _, selected) = block(&bytes[first_length..], 8)?;
            if second != version {
                return Err(Error::InvalidData);
            }
            counts = selected;
        } else if version != 0 {
            return Err(Error::InvalidData);
        }
        if counts[3] > limits.transitions {
            return Err(Error::TransitionLimit);
        }
        if counts[4] > limits.zones {
            return Err(Error::ZoneLimit);
        }
        let zone = TimeZone::from_tz_data(bytes).map_err(|_| Error::InvalidData)?;
        let first = first_zone(zone.as_ref());
        Ok(Self { zone, first })
    }

    fn lookup(&self, seconds: i64) -> &LocalTimeType {
        let zone = self.zone.as_ref();
        let transitions = zone.transitions();
        let at = transitions.partition_point(|t| t.unix_leap_time() <= seconds);
        if at == 0 && !transitions.is_empty() {
            return &zone.local_time_types()[self.first];
        }
        if at == transitions.len() && zone.extra_rule().is_some() {
            // Go ignores TZif leap records. Evaluate the footer separately so
            // the parser's normal leap-adjusted lookup cannot change our time.
            if let Ok(extension) =
                TimeZoneRef::new(&[], zone.local_time_types(), &[], zone.extra_rule())
                && let Ok(local) = extension.find_local_time_type(seconds)
            {
                return local;
            }
        }
        if at == 0 {
            return &zone.local_time_types()[self.first];
        }
        // Go keeps the last observed zone when no usable extension exists.
        &zone.local_time_types()[transitions[at - 1].local_time_type_index()]
    }
}
impl LocalZone for Snapshot {
    fn offset(&self, name: &[u8], wall_as_utc: i64) -> Option<i32> {
        let zone = self.zone.as_ref();
        for candidate in zone.local_time_types() {
            if candidate.time_zone_designation().as_bytes() == name
                && let Some(seconds) = wall_as_utc.checked_sub(i64::from(candidate.ut_offset()))
            {
                let actual = self.lookup(seconds);
                if actual.time_zone_designation().as_bytes() == name {
                    return Some(actual.ut_offset());
                }
            }
        }
        zone.local_time_types()
            .iter()
            .find(|candidate| candidate.time_zone_designation().as_bytes() == name)
            .map(LocalTimeType::ut_offset)
    }
}

// Go's pre-transition choice differs from tz-rs's unconditional type zero.
fn first_zone(zone: TimeZoneRef<'_>) -> usize {
    let transitions = zone.transitions();
    let types = zone.local_time_types();
    if !transitions.iter().any(|t| t.local_time_type_index() == 0) {
        return 0;
    }
    if let Some(first) = transitions.first() {
        let index = first.local_time_type_index();
        if types[index].is_dst()
            && let Some(index) = (0..index).rev().find(|index| !types[*index].is_dst())
        {
            return index;
        }
    }
    types.iter().position(|zone| !zone.is_dst()).unwrap_or(0)
}

fn block(bytes: &[u8], time_bytes: usize) -> Result<(u8, usize, [usize; 6]), Error> {
    let header = bytes.get(..44).ok_or(Error::InvalidData)?;
    if &header[..4] != b"TZif" {
        return Err(Error::InvalidData);
    }
    let counts = std::array::from_fn(|i| {
        let at = 20 + i * 4;
        u32::from_be_bytes(header[at..at + 4].try_into().expect("fixed header")) as usize
    });
    let length = counts
        .iter()
        .zip([1, 1, time_bytes + 4, time_bytes + 1, 6, 1])
        .try_fold(44_usize, |size, (count, width)| {
            size.checked_add(count.checked_mul(width)?)
        })
        .filter(|length| *length <= bytes.len())
        .ok_or(Error::InvalidData)?;
    Ok((header[4], length, counts))
}
