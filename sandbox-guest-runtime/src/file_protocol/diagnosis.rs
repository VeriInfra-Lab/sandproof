//! Ordered failure observations from the fixed protobuf JSON decoder. This
//! runs only after the bounded request decoder rejects a payload. A successful
//! diagnostic parse is a disagreement, never permission to fabricate a failure.
// Behavior derived from google.golang.org/protobuf v1.36.12 encoding/protojson
// and internal/encoding/json. Copyright 2018 The Go Authors. All rights reserved.
// BSD license: diagnosis/PROTOBUF-LICENSE.
use std::{fmt, ops::Range};

mod command;
mod tokens;
pub(crate) use command::observe as observe_command;
use tokens::{Decoder, Kind, Token};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonField {
    Path,
    Source,
    Destination,
    Depth,
    Cmd,
    Args,
    Cwd,
    Tag,
    Stdin,
    StdinBytes,
    PtyBytes,
    Signal,
    Pid,
    Cols,
    Rows,
    MapValue,
}
impl JsonField {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Source => "source",
            Self::Destination => "destination",
            Self::Depth => "depth",
            Self::Cmd => "cmd",
            Self::Args => "args",
            Self::Cwd => "cwd",
            Self::Tag => "tag",
            Self::Stdin | Self::StdinBytes => "stdin",
            Self::PtyBytes => "pty",
            Self::Signal => "signal",
            Self::Pid => "pid",
            Self::Cols => "cols",
            Self::Rows => "rows",
            Self::MapValue => "value",
        }
    }
    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Depth | Self::Pid | Self::Cols | Self::Rows => "uint32",
            Self::Stdin => "bool",
            Self::StdinBytes | Self::PtyBytes => "bytes",
            Self::Signal => "enum",
            _ => "string",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonOneof {
    Selector,
    Input,
}
impl JsonOneof {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Selector => "process.ProcessSelector.selector",
            Self::Input => "process.ProcessInput.input",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonReason {
    Empty,
    UnexpectedEof,
    Depth,
    UnexpectedToken,
    InvalidValue,
    MissingColon(u8),
    InvalidUtf8,
    InvalidCharacter(u8),
    InvalidEscape,
    Duplicate,
    DuplicateMap,
    Oneof(JsonOneof),
    InvalidField(JsonField),
}

/// Only this decoder can construct an observation. Raw fragments contain at
/// most the already admitted body; Debug never discloses its field contents.
#[derive(Clone, PartialEq, Eq)]
pub struct JsonError {
    method: &'static str,
    reason: JsonReason,
    position: (usize, usize),
    raw: Vec<u8>,
}
impl fmt::Debug for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsonError").finish_non_exhaustive()
    }
}
impl JsonError {
    #[must_use]
    pub const fn method(&self) -> &'static str {
        self.method
    }
    #[must_use]
    pub const fn reason(&self) -> JsonReason {
        self.reason
    }
    #[must_use]
    pub const fn position(&self) -> (usize, usize) {
        self.position
    }
    #[must_use]
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }
}

#[derive(Clone)]
struct Failure {
    reason: JsonReason,
    position: usize,
    raw: Range<usize>,
}
impl Failure {
    fn at(reason: JsonReason, position: usize) -> Self {
        Self {
            reason,
            position,
            raw: position..position,
        }
    }
    fn token(reason: JsonReason, token: &Token) -> Self {
        Self {
            reason,
            position: token.range.start,
            raw: token.range.clone(),
        }
    }
}

pub(super) fn observe(method: &str, body: &[u8]) -> Option<JsonError> {
    let method = match method {
        "Stat" => "Stat",
        "MakeDir" => "MakeDir",
        "ListDir" => "ListDir",
        "Move" => "Move",
        "Remove" => "Remove",
        _ => return None,
    };
    let error = if body.is_empty() {
        Failure::at(JsonReason::Empty, 0)
    } else {
        message(method, body).err()?
    };
    Some(finish(method, body, error))
}

fn finish(method: &'static str, body: &[u8], error: Failure) -> JsonError {
    let prefix = &body[..error.position];
    let line = prefix.split(|b| *b == b'\n').count();
    let column_bytes = prefix.rsplit(|b| *b == b'\n').next().unwrap_or_default();
    let mut column = 1;
    let mut at = 0;
    while at < column_bytes.len() {
        at += tokens::rune_length(&column_bytes[at..]).unwrap_or(1);
        column += 1;
    }
    JsonError {
        method,
        reason: error.reason,
        position: (line, column),
        raw: body[error.raw].to_vec(),
    }
}

fn message(method: &str, body: &[u8]) -> Result<(), Failure> {
    let mut decoder = Decoder::new(body);
    let first = decoder.read()?;
    if first.kind != Kind::ObjectOpen {
        return Err(Failure::token(JsonReason::UnexpectedToken, &first));
    }
    let mut seen = 0_u8;
    loop {
        let key = decoder.read()?;
        if key.kind == Kind::ObjectClose {
            break;
        }
        if key.kind != Kind::Name {
            return Err(Failure::token(JsonReason::UnexpectedToken, &key));
        }
        // Tokenization has already checked all string escapes and Unicode.
        let name: String = serde_json::from_slice(&body[key.range.clone()])
            .map_err(|_| Failure::token(JsonReason::InvalidValue, &key))?;
        let field = match (method, name.as_str()) {
            ("Move", "source") => Some((JsonField::Source, 1)),
            ("Move", "destination") => Some((JsonField::Destination, 2)),
            ("ListDir", "depth") => Some((JsonField::Depth, 2)),
            (m, "path") if m != "Move" => Some((JsonField::Path, 1)),
            _ => None,
        };
        if let Some((field, bit)) = field {
            if seen & bit != 0 {
                return Err(Failure::token(JsonReason::Duplicate, &key));
            }
            seen |= bit;
            let value = decoder.read()?;
            if value.kind != Kind::Null && !valid_scalar(field, &value, body) {
                return Err(Failure::token(JsonReason::InvalidField(field), &value));
            }
        } else {
            let mut open = 0_usize;
            loop {
                let value = decoder.read()?;
                match value.kind {
                    Kind::ObjectOpen | Kind::ArrayOpen => open += 1,
                    Kind::ObjectClose | Kind::ArrayClose => open = open.saturating_sub(1),
                    _ => {}
                }
                if open == 0 {
                    break;
                }
            }
        }
    }
    let last = decoder.read()?;
    if last.kind != Kind::Eof {
        return Err(Failure::token(JsonReason::UnexpectedToken, &last));
    }
    Ok(())
}

fn valid_scalar(field: JsonField, token: &Token, body: &[u8]) -> bool {
    if field != JsonField::Depth {
        return token.kind == Kind::String;
    }
    let raw = &body[token.range.clone()];
    match token.kind {
        Kind::String => serde_json::from_slice::<String>(raw)
            .ok()
            .and_then(|s| super::request_json::exact_uint32(&s))
            .is_some(),
        Kind::Number => {
            let Ok(raw) = std::str::from_utf8(raw) else {
                return false;
            };
            let raw = raw.strip_suffix(['e', 'E']).unwrap_or(raw);
            super::request_json::exact_uint32(raw).is_some()
        }
        _ => false,
    }
}
