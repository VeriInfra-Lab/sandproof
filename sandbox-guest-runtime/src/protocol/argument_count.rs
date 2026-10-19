//! Validate the original string array while retaining only its count. Serde's
//! string scratch is bounded by the admitted input; no per-argument allocation
//! or vector capacity follows the number of entries. Continue after exceeding
//! the execution limit so malformed late elements retain JSON precedence.
use serde::{Deserialize, Deserializer, de};

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

pub(super) struct StringOnly;
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
