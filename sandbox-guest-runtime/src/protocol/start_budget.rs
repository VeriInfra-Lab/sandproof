//! Validate the canonical Start schema without retaining argument strings or
//! environment values. Exact decoded keys still enforce duplicate rejection;
//! unescaped keys borrow the admitted payload; short escaped keys stay inline.
use crate::file_protocol::request_json::StringKey;
use serde::{Deserializer, de};

use super::{ProtocolError, Request, StartLimits, StartRequest, argument_count, parse};

mod keys;

pub(super) fn refusal(
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
                let mut keys = keys::Chunks::default();
                let mut invalid_name = false;
                while let Some(key) = map.next_key::<StringKey<'de>>()? {
                    map.next_value::<argument_count::StringOnly>()?;
                    invalid_name |= key.as_str().is_empty() || key.as_str().contains('=');
                    keys.push(key);
                }
                if !keys.unique() {
                    return Err(de::Error::custom("duplicate environment key"));
                }
                // No executable prefix escapes. Any malformed map fails before
                // a marker; the original ordered diagnostic parser determines
                // the public first failure, including an earlier duplicate.
                Ok(CheckedEnvironment {
                    count: keys.len(),
                    invalid_name,
                })
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}
