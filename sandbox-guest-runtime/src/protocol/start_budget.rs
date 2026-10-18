//! Validate the canonical Start schema without retaining argument strings or
//! environment values. Exact decoded keys still enforce duplicate rejection;
//! unescaped keys borrow the admitted payload, escaped keys own their decoding.
use std::borrow::Cow;

use serde::{Deserialize, Deserializer, de};

use super::{ProtocolError, Request, StartLimits, StartRequest, argument_count, parse};

pub(super) fn refusal(
    payload: &[u8],
    limits: StartLimits,
) -> Result<Option<Request>, ProtocolError> {
    let checked: StartRequest<argument_count::Count, Key<'_>, argument_count::StringOnly> =
        parse(payload)?;
    let Some(process) = checked.process else {
        return Ok(None);
    };
    if process.args.0 > limits.arguments {
        return Ok(Some(Request::StartArgumentsExceeded {
            limit: limits.arguments,
        }));
    }
    if process.envs.len() > limits.env_entries {
        return Ok(Some(Request::StartEnvironmentExceeded {
            limits,
            cwd: process.cwd,
            invalid_name: process
                .envs
                .keys()
                .any(|key| key.0.is_empty() || key.0.contains('=')),
        }));
    }
    Ok(None)
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Key<'a>(Cow<'a, str>);

impl<'de> Deserialize<'de> for Key<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = Key<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string key")
            }
            fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
                Ok(Key(Cow::Borrowed(value)))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(Key(Cow::Owned(value.to_owned())))
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}
