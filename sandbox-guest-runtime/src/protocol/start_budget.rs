//! Validate the canonical Start schema without retaining argument strings or
//! environment values. Exact decoded keys still enforce duplicate rejection;
//! unescaped keys borrow the admitted payload; short escaped keys stay inline.
use crate::file_protocol::request_json::StringKey;

use super::{ProtocolError, Request, StartLimits, StartRequest, argument_count, parse};

pub(super) fn refusal(
    payload: &[u8],
    limits: StartLimits,
) -> Result<Option<Request>, ProtocolError> {
    let checked: StartRequest<argument_count::Count, StringKey<'_>, argument_count::StringOnly> =
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
                .any(|key| key.as_str().is_empty() || key.as_str().contains('=')),
        }));
    }
    Ok(None)
}
