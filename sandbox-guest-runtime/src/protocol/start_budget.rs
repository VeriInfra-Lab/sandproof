//! Check the canonical Start fields without retaining arguments or environment
//! strings. A candidate refusal must still pass the ordered JSON validator;
//! otherwise duplicate keys could be mistaken for execution-budget overflow.
use serde::{Deserialize, Deserializer, de};

use super::{ProtocolError, Request, StartLimits, StartRequest, argument_count, parse};

pub(super) fn refusal_candidate(
    payload: &[u8],
    limits: StartLimits,
) -> Result<Option<Request>, ProtocolError> {
    let checked: StartRequest<argument_count::Count, CheckedEnvironment> = parse(payload)?;
    let Some(process) = checked.process else {
        return Ok(None);
    };
    if process.args.0 > limits.arguments {
        return Ok(Some(Request::StartArgumentsExceeded {
            limit: limits.arguments,
        }));
    }
    if process.envs.count > limits.env_entries {
        return Ok(Some(Request::StartEnvironmentExceeded {
            limits,
            cwd: process.cwd,
            invalid_name: process.envs.invalid_name,
        }));
    }
    Ok(None)
}

#[derive(Default)]
struct CheckedEnvironment {
    count: usize,
    invalid_name: bool,
}

impl<'de> super::request_json::Environment<'de> for CheckedEnvironment {
    fn entries<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = CheckedEnvironment;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string map")
            }
            fn visit_map<M: de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut count = 0_usize;
                let mut invalid_name = false;
                while let Some(key) = map.next_key::<Name>()? {
                    map.next_value::<argument_count::StringOnly>()?;
                    invalid_name |= key.0;
                    count = count
                        .checked_add(1)
                        .ok_or_else(|| de::Error::custom("environment count overflow"))?;
                }
                Ok(CheckedEnvironment {
                    count,
                    invalid_name,
                })
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

struct Name(bool);
impl<'de> Deserialize<'de> for Name {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl de::Visitor<'_> for Visitor {
            type Value = Name;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string key")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Name, E> {
                Ok(Name(value.is_empty() || value.contains('=')))
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}
