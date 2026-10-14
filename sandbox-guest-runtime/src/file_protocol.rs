//! Fixed filesystem unary JSON, independent of authorization and actual I/O.
//! GET/POST bodies, metadata lookup and mutation are separate ports. Unknown
//! methods (including currently rejected watchers) never become empty success.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::protocol::{ProtocolError, encode_json, null_default, parse, uint32};

mod content_type;
pub use content_type::{JsonContentType, json_content_type};
mod fault;
pub use fault::{FaultStage, FileFault};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Stat(String),
    MakeDir(String),
    ListDir { path: String, depth: u32 },
    Move { source: String, destination: String },
    Remove(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathRequest {
    #[serde(default, deserialize_with = "null_default")]
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListRequest {
    #[serde(default, deserialize_with = "null_default")]
    path: String,
    #[serde(default, deserialize_with = "uint32")]
    depth: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveRequest {
    #[serde(default, deserialize_with = "null_default")]
    source: String,
    #[serde(default, deserialize_with = "null_default")]
    destination: String,
}

/// Decode an already bounded unary HTTP body. No Connect stream envelope is
/// accepted here; HTTP admission must bound collection before this call.
/// # Errors
/// Rejects malformed/ambiguous messages, unknown methods and exceeded budgets.
pub fn decode_request(method: &str, body: &[u8], limit: usize) -> Result<Request, ProtocolError> {
    if limit == 0 || body.len() > limit {
        return Err(ProtocolError::ResourceExhausted);
    }
    match method {
        "Stat" => Ok(Request::Stat(parse::<PathRequest>(body)?.path)),
        "MakeDir" => Ok(Request::MakeDir(parse::<PathRequest>(body)?.path)),
        "Remove" => Ok(Request::Remove(parse::<PathRequest>(body)?.path)),
        "ListDir" => {
            let request: ListRequest = parse(body)?;
            // Preserve wire zero. The actual directory operation applies the
            // fixed depth=1 default; decoding does not perform a traversal.
            Ok(Request::ListDir {
                path: request.path,
                depth: request.depth,
            })
        }
        "Move" => {
            let request: MoveRequest = parse(body)?;
            Ok(Request::Move {
                source: request.source,
                destination: request.destination,
            })
        }
        _ => Err(ProtocolError::Unimplemented),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum FileType {
    #[serde(rename = "FILE_TYPE_UNSPECIFIED")]
    Unspecified,
    #[serde(rename = "FILE_TYPE_FILE")]
    File,
    #[serde(rename = "FILE_TYPE_DIRECTORY")]
    Directory,
    #[serde(rename = "FILE_TYPE_SYMLINK")]
    Symlink,
}

/// A valid protobuf timestamp, not a guess based on request or wall-clock time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ModifiedTime(String);

impl ModifiedTime {
    /// Canonical UTC protobuf JSON (0, 3, 6 or 9 fractional digits).
    /// # Errors
    /// Refuses invalid nanoseconds or dates outside protobuf years 1..=9999.
    pub fn new(seconds: i64, nanos: u32) -> Result<Self, ProtocolError> {
        if nanos >= 1_000_000_000 {
            return Err(ProtocolError::InvalidArgument);
        }
        let date = time::OffsetDateTime::from_unix_timestamp(seconds)
            .map_err(|_| ProtocolError::InvalidArgument)?;
        if !(1..=9999).contains(&date.year()) {
            return Err(ProtocolError::InvalidArgument);
        }
        let fraction = match nanos {
            0 => String::new(),
            n if n % 1_000_000 == 0 => format!(".{:03}", n / 1_000_000),
            n if n % 1_000 == 0 => format!(".{:06}", n / 1_000),
            n => format!(".{n:09}"),
        };
        Ok(Self(format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{fraction}Z",
            date.year(),
            u8::from(date.month()),
            date.day(),
            date.hour(),
            date.minute(),
            date.second(),
        )))
    }
}

/// Already observed attributes. Symlink lstat/target semantics, UID/name lookup
/// and best-effort xattr policy belong to the filesystem adapter. Encoding an
/// Entry is not evidence that a file exists or a mutation completed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: FileType,
    pub path: String,
    #[serde(serialize_with = "int64")]
    pub size: i64,
    pub mode: u32,
    pub permissions: String,
    pub owner: String,
    pub group: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_time: Option<ModifiedTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symlink_target: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "Serde serialize_with passes a field reference"
)]
fn int64<S: serde::Serializer>(value: &i64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_string())
}

#[derive(Clone, Copy)]
pub enum Response<'a> {
    Entry(&'a Entry),
    Entries(&'a [Entry]),
    Empty,
}

/// Bound JSON allocation while writing. No whole-result Value clone; the
/// caller must separately bound the retained attributes/list before this call.
/// # Errors
/// An undersized budget fails, never returns truncated JSON or partial success.
pub fn encode_response(response: Response<'_>, limit: usize) -> Result<Vec<u8>, ProtocolError> {
    #[derive(Serialize)]
    struct One<'a> {
        entry: &'a Entry,
    }
    #[derive(Serialize)]
    struct Many<'a> {
        entries: &'a [Entry],
    }
    #[derive(Serialize)]
    struct Empty {}
    match response {
        Response::Entry(entry) => encode_json(&One { entry }, limit),
        Response::Entries(entries) => encode_json(&Many { entries }, limit),
        Response::Empty => encode_json(&Empty {}, limit),
    }
}
