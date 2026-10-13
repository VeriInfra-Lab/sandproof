//! Go HTTP dates, including obsolete layouts, fractions and local zone lookup.
// Adapted from Go 1.26.8 time/format.go and net/http/header.go.
// Copyright 2009 The Go Authors. BSD license: ../multipart/GO-LICENSE.
use super::Error;
use time::{Date, Month, OffsetDateTime};

/// A checked UTC instant. Nanoseconds distinguish Unix epoch from unspecified
/// time. Calendar conversion supports time crate's explicit -9999..9999 range;
/// values beyond it are component errors, never guessed dates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    seconds: i64,
    nanos: u32,
}
impl Timestamp {
    /// # Errors
    /// Unrepresentable calendar value or invalid nanoseconds.
    pub fn new(seconds: i64, nanos: u32) -> Result<Self, Error> {
        if nanos >= 1_000_000_000 || OffsetDateTime::from_unix_timestamp(seconds).is_err() {
            return Err(Error::InvalidTimestamp);
        }
        Ok(Self { seconds, nanos })
    }
    #[must_use]
    pub const fn seconds(self) -> i64 {
        self.seconds
    }
    #[must_use]
    pub const fn nanos(self) -> u32 {
        self.nanos
    }
    pub(super) const fn go_zero(self) -> bool {
        self.seconds == -62_135_596_800 && self.nanos == 0
    }
    pub(super) const fn unspecified(self) -> bool {
        self.go_zero() || self.seconds == 0 && self.nanos == 0
    }
    pub(super) const fn truncate(self) -> Self {
        Self {
            seconds: self.seconds,
            nanos: 0,
        }
    }

    /// Canonical Last-Modified UTC; Go suppresses exact year-1 zero and epoch.
    /// # Panics
    /// Only if the timestamp constructor invariant is violated.
    #[must_use]
    pub fn last_modified(self) -> Option<String> {
        if self.unspecified() {
            return None;
        }
        let time = OffsetDateTime::from_unix_timestamp(self.seconds).expect("checked timestamp");
        let year = if time.year() < 0 {
            format!("-{:04}", -time.year())
        } else {
            format!("{:04}", time.year())
        };
        Some(format!(
            "{}, {:02} {} {year} {:02}:{:02}:{:02} GMT",
            DAYS[usize::from(time.weekday().number_days_from_sunday())],
            time.day(),
            MONTHS[usize::from(u8::from(time.month()) - 1)],
            time.hour(),
            time.minute(),
            time.second()
        ))
    }
}

/// Trusted daemon local-zone snapshot, corresponding to Go Local.lookupName.
/// `wall_as_utc` is the parsed civil time treated as UTC, BEFORE offset lookup.
/// Unknown names return None and keep that UTC instant, even GMT+N (Go's
/// fabricated location changes the displayed zone, not the Unix instant).
pub trait LocalZone {
    fn offset(&self, name: &[u8], wall_as_utc: i64) -> Option<i32>;
}
pub struct Utc;
impl LocalZone for Utc {
    fn offset(&self, _: &[u8], _: i64) -> Option<i32> {
        None
    }
}

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const LONG_DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Parse exactly the three fixed net/http layouts. Request bytes must already
/// be bounded; malformed dates return None and are ignored by date conditions.
/// Weekday spelling is checked case-insensitively but not against the date.
#[must_use]
pub fn parse(value: &[u8], local: &impl LocalZone) -> Option<Timestamp> {
    (0..3).find_map(|layout| parse_layout(value, layout, local))
}
fn parse_layout(value: &[u8], layout: u8, local: &impl LocalZone) -> Option<Timestamp> {
    let mut input = Input(value);
    input.word(if layout == 1 { &LONG_DAYS } else { &DAYS })?;
    let (year, month, day, hour, minute, second, nanos, zone) = if layout < 2 {
        input.literal(b", ")?;
        let day = input.number(2, true)?;
        input.literal(if layout == 0 { b" " } else { b"-" })?;
        let month = input.word(&MONTHS)? + 1;
        input.literal(if layout == 0 { b" " } else { b"-" })?;
        let mut year = if layout == 0 {
            i32::from(input.number(4, true)?)
        } else {
            let bytes = input.0.get(..2)?;
            let year = std::str::from_utf8(bytes).ok()?.parse::<i32>().ok()?;
            input.0 = &input.0[2..];
            year
        };
        if layout == 1 {
            year += if year >= 69 { 1900 } else { 2000 };
        }
        input.literal(b" ")?;
        let (hour, minute, second, nanos) = input.clock()?;
        input.literal(b" ")?;
        let zone = if layout == 0 {
            input.literal(b"GMT")?;
            b"".as_slice()
        } else {
            input.zone()?
        };
        (year, month, day, hour, minute, second, nanos, zone)
    } else {
        input.literal(b" ")?;
        let month = input.word(&MONTHS)? + 1;
        input.literal(b" ")?;
        let day = input.number(2, false)?;
        input.literal(b" ")?;
        let (hour, minute, second, nanos) = input.clock()?;
        input.literal(b" ")?;
        let year = i32::from(input.number(4, true)?);
        (
            year,
            month,
            day,
            hour,
            minute,
            second,
            nanos,
            b"".as_slice(),
        )
    };
    if !input.0.is_empty() {
        return None;
    }
    let date = Date::from_calendar_date(
        year,
        Month::try_from(u8::try_from(month).ok()?).ok()?,
        u8::try_from(day).ok()?,
    )
    .ok()?;
    let time = date
        .with_hms_nano(
            u8::try_from(hour).ok()?,
            u8::try_from(minute).ok()?,
            u8::try_from(second).ok()?,
            nanos,
        )
        .ok()?
        .assume_utc();
    let wall = time.unix_timestamp();
    let offset = if zone.is_empty() || zone == b"UTC" {
        0
    } else {
        local.offset(zone, wall).unwrap_or(0)
    };
    Timestamp::new(wall.checked_sub(i64::from(offset))?, nanos).ok()
}
struct Input<'a>(&'a [u8]);
impl<'a> Input<'a> {
    fn literal(&mut self, prefix: &[u8]) -> Option<()> {
        for byte in prefix {
            if *byte == b' ' {
                if self.0.first().is_some_and(|b| *b != b' ') {
                    return None;
                }
                while self.0.first() == Some(&b' ') {
                    self.0 = &self.0[1..];
                }
            } else {
                self.0 = self.0.strip_prefix(&[*byte])?;
            }
        }
        Some(())
    }
    fn word(&mut self, words: &[&str]) -> Option<usize> {
        let index = words.iter().position(|word| {
            self.0
                .get(..word.len())
                .is_some_and(|value| value.eq_ignore_ascii_case(word.as_bytes()))
        })?;
        self.0 = &self.0[words[index].len()..];
        Some(index)
    }
    fn number(&mut self, max: usize, fixed: bool) -> Option<u16> {
        let count = self
            .0
            .iter()
            .take(max)
            .take_while(|b| b.is_ascii_digit())
            .count();
        if count == 0 || fixed && count != max {
            return None;
        }
        let value = self.0[..count]
            .iter()
            .fold(0_u16, |value, byte| value * 10 + u16::from(*byte - b'0'));
        self.0 = &self.0[count..];
        Some(value)
    }
    fn clock(&mut self) -> Option<(u16, u16, u16, u32)> {
        let hour = self.number(2, false)?;
        self.literal(b":")?;
        let minute = self.number(2, true)?;
        self.literal(b":")?;
        let second = self.number(2, true)?;
        let mut nanos = 0;
        if matches!(self.0.first(), Some(b'.' | b','))
            && self.0.get(1).is_some_and(u8::is_ascii_digit)
        {
            self.0 = &self.0[1..];
            let mut count = 0;
            while let Some(byte) = self.0.first().filter(|b| b.is_ascii_digit()) {
                if count < 9 {
                    nanos = nanos * 10 + u32::from(*byte - b'0');
                }
                count += 1;
                self.0 = &self.0[1..];
            }
            for _ in count..9 {
                nanos *= 10;
            }
        }
        Some((hour, minute, second, nanos))
    }
    fn zone(&mut self) -> Option<&'a [u8]> {
        let value = self.0;
        let length = if value.starts_with(b"UTC") {
            3
        } else if value.starts_with(b"ChST") || value.starts_with(b"MeST") {
            4
        } else if value.starts_with(b"GMT") {
            3 + signed_offset(&value[3..]).unwrap_or(0)
        } else if value.len() >= 3 && matches!(value.first(), Some(b'+' | b'-')) {
            signed_offset(value)?
        } else {
            let count = value
                .iter()
                .take(6)
                .take_while(|b| b.is_ascii_uppercase())
                .count();
            match count {
                3 => 3,
                4 if value[3] == b'T' || value.starts_with(b"WITA") => 4,
                5 if value[4] == b'T' => 5,
                _ => return None,
            }
        };
        self.0 = &value[length..];
        Some(&value[..length])
    }
}
fn signed_offset(value: &[u8]) -> Option<usize> {
    if !matches!(value.first(), Some(b'+' | b'-')) {
        return None;
    }
    let count = value[1..].iter().take_while(|b| b.is_ascii_digit()).count();
    if count == 0 {
        return None;
    }
    let number = value[1..=count].iter().try_fold(0_i64, |n, b| {
        n.checked_mul(10)?.checked_add(i64::from(*b - b'0'))
    })?;
    (number <= 23).then_some(count + 1)
}
