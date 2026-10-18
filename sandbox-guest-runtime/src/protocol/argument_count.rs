//! Validate the original string array while retaining only its count. Serde's
//! string scratch is bounded by the admitted input; no per-argument allocation
//! or vector capacity follows the number of entries. Continue after exceeding
//! the execution limit so malformed late elements retain JSON precedence.
use serde::{Deserialize, Deserializer, de};

/// Inspect only the allocation-sensitive path. This is not schema acceptance:
/// the caller must still run the original full schema with either Vec or Count.
/// In particular, do not allocate all environment values merely to decide
/// whether the argument vector can be built.
pub(super) fn exceeds(payload: &[u8], limit: usize) -> Result<bool, super::ProtocolError> {
    #[derive(Deserialize)]
    struct Probe {
        #[serde(default, deserialize_with = "super::optional_object")]
        process: Option<Arguments>,
    }
    #[derive(Deserialize)]
    struct Arguments {
        #[serde(default, deserialize_with = "super::null_default")]
        args: Count,
    }
    let probe: Probe = super::parse(payload)?;
    Ok(probe.process.is_some_and(|process| process.args.0 > limit))
}

#[derive(Default)]
pub(super) struct Count(pub usize);

impl<'de> Deserialize<'de> for Count {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = Count;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string array")
            }
            fn visit_seq<S: de::SeqAccess<'de>>(self, mut sequence: S) -> Result<Count, S::Error> {
                let mut count = 0_usize;
                while sequence.next_element::<StringOnly>()?.is_some() {
                    count = count
                        .checked_add(1)
                        .ok_or_else(|| de::Error::custom("argument count overflow"))?;
                }
                Ok(Count(count))
            }
        }
        deserializer.deserialize_seq(Visitor)
    }
}

struct StringOnly;
impl<'de> Deserialize<'de> for StringOnly {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl de::Visitor<'_> for Visitor {
            type Value = StringOnly;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string")
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<StringOnly, E> {
                Ok(StringOnly)
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}
