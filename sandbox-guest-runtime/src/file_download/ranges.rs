use super::{Error, Limits, trim};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub length: u64,
}
impl ByteRange {
    #[must_use]
    pub fn content_range(self, size: u64) -> String {
        // A zero suffix legitimately renders size-(size-1), including 0--1/0.
        format!(
            "bytes {}-{}/{size}",
            self.start,
            i128::from(self.start) + i128::from(self.length) - 1
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RangePlan {
    Full,
    Partial(Vec<ByteRange>),
    /// 416; only an all-nonoverlapping request gets Content-Range: bytes */size.
    Rejected {
        no_overlap: bool,
    },
}

/// Parse only AFTER preconditions and successful length seek. Preserve order,
/// overlaps and duplicates; a range-byte sum larger than size means full body.
/// A plan is not evidence that its required seeks/reads have succeeded.
/// # Errors
/// Component configuration/header/items/size overflow. Go's overflowing int64
/// aggregate is explicitly refused; no wrapped/negative length is emitted.
pub fn ranges(header: &[u8], size: u64, limits: Limits) -> Result<RangePlan, Error> {
    limits.check(&[header])?;
    if size > i64::MAX as u64 {
        return Err(Error::ArithmeticOverflow);
    }
    if header.is_empty() {
        return Ok(RangePlan::Full);
    }
    let Some(value) = header.strip_prefix(b"bytes=") else {
        return Ok(RangePlan::Rejected { no_overlap: false });
    };
    let count = value.split(|b| *b == b',').count();
    if count > limits.items {
        return Err(Error::ItemLimit);
    }
    let mut ranges = Vec::with_capacity(count);
    let mut no_overlap = false;
    for value in value.split(|b| *b == b',') {
        let value = trim(value);
        if value.is_empty() {
            continue;
        }
        let Some(dash) = value.iter().position(|b| *b == b'-') else {
            return Ok(RangePlan::Rejected { no_overlap: false });
        };
        let start = trim(&value[..dash]);
        let end = trim(&value[dash + 1..]);
        let range = if start.is_empty() {
            let Some(length) = (!end.starts_with(b"-")).then(|| integer(end)).flatten() else {
                return Ok(RangePlan::Rejected { no_overlap: false });
            };
            let length = length.min(size);
            ByteRange {
                start: size - length,
                length,
            }
        } else {
            let Some(start) = integer(start) else {
                return Ok(RangePlan::Rejected { no_overlap: false });
            };
            // Go does not parse an end value for a nonoverlapping start.
            if start >= size {
                no_overlap = true;
                continue;
            }
            let last = if end.is_empty() {
                size - 1
            } else {
                let Some(end) = integer(end).filter(|end| *end >= start) else {
                    return Ok(RangePlan::Rejected { no_overlap: false });
                };
                end.min(size - 1)
            };
            ByteRange {
                start,
                length: last - start + 1,
            }
        };
        ranges.push(range);
    }
    if ranges.is_empty() {
        return Ok(if no_overlap && size != 0 {
            RangePlan::Rejected { no_overlap: true }
        } else {
            RangePlan::Full
        });
    }
    let sum = ranges
        .iter()
        .try_fold(0_u64, |sum, range| sum.checked_add(range.length))
        .filter(|sum| i64::try_from(*sum).is_ok())
        .ok_or(Error::ArithmeticOverflow)?;
    if sum > size {
        Ok(RangePlan::Full)
    } else {
        Ok(RangePlan::Partial(ranges))
    }
}

fn integer(value: &[u8]) -> Option<u64> {
    // strconv.ParseInt(..., 10, 64): optional '+', no underscore or whitespace.
    let negative = value.starts_with(b"-");
    let value = value
        .strip_prefix(b"+")
        .or_else(|| value.strip_prefix(b"-"))
        .unwrap_or(value);
    if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
        return None;
    }
    value
        .iter()
        .try_fold(0_u64, |number, byte| {
            number.checked_mul(10)?.checked_add(u64::from(*byte - b'0'))
        })
        .filter(|number| i64::try_from(*number).is_ok() && (!negative || *number == 0))
}
