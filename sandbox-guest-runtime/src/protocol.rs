//! Fixed SDK command JSON/Connect framing. This module does not authenticate,
//! resolve public selectors, choose execution defaults, or authorize a launch.
//! HTTP admission must bound the body before collecting it and retain its
//! permit until actual transport completion. These are per-message budgets,
//! never a cumulative limit on a Start/Connect response stream.

use std::collections::BTreeMap;
use std::io::{self, Write};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use sandproof_guest_core::Channel;
use serde::{Deserialize, Serialize};

mod request_json;
use crate::file_protocol::request_json::{prepare_command, uint32};
use request_json::{bytes, envs, signal};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    InvalidArgument,
    ResourceExhausted,
    Unimplemented,
}

impl ProtocolError {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidArgument => "invalid_argument",
            Self::ResourceExhausted => "resource_exhausted",
            Self::Unimplemented => "unimplemented",
        }
    }
}

/// Public selectors remain untrusted wire values. In particular, this type
/// does not bind a PID to a service session or protect against PID reuse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selector {
    Unset,
    Pid(u32),
    Tag(String),
}

impl<'de> Deserialize<'de> for Selector {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Fields {
            #[serde(default, deserialize_with = "null_default")]
            pid: Option<Uint32>,
            #[serde(default, deserialize_with = "null_default")]
            tag: Option<String>,
        }
        let fields = Object::<Fields>::deserialize(deserializer)?.0;
        match (fields.pid, fields.tag) {
            (Some(pid), None) => Ok(Self::Pid(pid.0)),
            (None, Some(tag)) => Ok(Self::Tag(tag)),
            (None, None) => Ok(Self::Unset),
            _ => Err(serde::de::Error::custom("multiple selectors")),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProcessConfig {
    #[serde(default, deserialize_with = "null_default")]
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cmd: String,
    #[serde(default, deserialize_with = "null_default")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, deserialize_with = "envs")]
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub envs: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "null_default")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

/// Borrowed list metadata, already selected by the active command registry.
/// This encoder does not infer liveness or reconstruct execution defaults.
#[derive(Serialize)]
pub struct ProcessInfo<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<&'a ProcessConfig>,
    #[serde(skip_serializing_if = "is_zero")]
    pub pid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<&'a str>,
}

#[derive(Clone, Copy)]
pub enum Response<'a> {
    List(&'a [ProcessInfo<'a>]),
    /// Successful `SendInput`, `CloseStdin`, `SendSignal` or Update acknowledgement.
    /// Empty is not a process-exit or complete-resource-cleanup receipt.
    Empty,
}

/// Encode one complete unary reply without cloning process metadata. The
/// caller separately owns the registry snapshot and the transport capacity.
/// List order is supplied by that snapshot, not a new ordering guarantee.
/// # Errors
/// Returns no partial success when the full JSON cannot fit the explicit budget.
pub fn encode_response(response: Response<'_>, limit: usize) -> Result<Vec<u8>, ProtocolError> {
    #[derive(Serialize)]
    struct List<'a> {
        #[serde(skip_serializing_if = "<[ProcessInfo<'_>]>::is_empty")]
        processes: &'a [ProcessInfo<'a>],
    }
    #[derive(Serialize)]
    struct Empty {}
    match response {
        Response::List(processes) => encode_json(&List { processes }, limit),
        Response::Empty => encode_json(&Empty {}, limit),
    }
}

fn is_zero<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Pty {
    #[serde(default, deserialize_with = "optional_object")]
    pub size: Option<PtySize>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct PtySize {
    #[serde(default, deserialize_with = "uint32")]
    pub cols: u32,
    #[serde(default, deserialize_with = "uint32")]
    pub rows: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct StartRequest {
    #[serde(default, deserialize_with = "optional_object")]
    pub process: Option<ProcessConfig>,
    #[serde(default, deserialize_with = "optional_object")]
    pub pty: Option<Pty>,
    #[serde(default, deserialize_with = "null_default")]
    pub tag: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub stdin: Option<bool>,
}

impl StartRequest {
    /// The fixed envd defaults omitted stdin to true; the current SDK sends
    /// false explicitly. Do not collapse absence into false during decoding.
    #[must_use]
    pub fn stdin_enabled(&self) -> bool {
        self.stdin.unwrap_or(true)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Unset,
    Stdin(Vec<u8>),
    Pty(Vec<u8>),
}

impl<'de> Deserialize<'de> for Input {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Fields {
            #[serde(default, deserialize_with = "null_default")]
            stdin: Option<Bytes>,
            #[serde(default, deserialize_with = "null_default")]
            pty: Option<Bytes>,
        }
        let fields = Object::<Fields>::deserialize(deserializer)?.0;
        match (fields.stdin, fields.pty) {
            (Some(bytes), None) => Ok(Self::Stdin(bytes.0)),
            (None, Some(bytes)) => Ok(Self::Pty(bytes.0)),
            (None, None) => Ok(Self::Unset),
            _ => Err(serde::de::Error::custom("multiple inputs")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Unspecified,
    Terminate,
    Kill,
    /// Proto3 preserves unknown numeric enums; the executor must reject an
    /// unsupported signal explicitly, never substitute SIGTERM/SIGKILL.
    Unknown(i32),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    Start(StartRequest),
    Connect(Option<Selector>),
    List,
    SendInput {
        process: Option<Selector>,
        input: Option<Input>,
    },
    CloseStdin(Option<Selector>),
    SendSignal {
        process: Option<Selector>,
        signal: Signal,
    },
    Update {
        process: Option<Selector>,
        pty: Option<Pty>,
    },
}

#[derive(Deserialize)]
struct SelectRequest {
    #[serde(default, deserialize_with = "optional_object")]
    process: Option<Selector>,
}

#[derive(Deserialize)]
struct InputRequest {
    #[serde(default, deserialize_with = "optional_object")]
    process: Option<Selector>,
    #[serde(default, deserialize_with = "optional_object")]
    input: Option<Input>,
}

#[derive(Deserialize)]
struct SignalRequest {
    #[serde(default, deserialize_with = "optional_object")]
    process: Option<Selector>,
    #[serde(default, deserialize_with = "signal")]
    signal: Option<Signal>,
}

#[derive(Deserialize)]
struct UpdateRequest {
    #[serde(default, deserialize_with = "optional_object")]
    process: Option<Selector>,
    #[serde(default, deserialize_with = "optional_object")]
    pty: Option<Pty>,
}

#[derive(Deserialize)]
struct Empty {}

/// Decode one complete HTTP body after bounded HTTP collection. Start/Connect
/// require exactly one uncompressed Connect envelope, including its full
/// declared length; unary methods use unframed JSON. Extra envelopes, flags,
/// compression and trailing bytes are never silently discarded.
///
/// # Errors
/// Returns a payload-free protocol error on invalid framing/JSON, unknown
/// methods or a message above the caller's explicit nonzero budget.
pub fn decode_request(
    method: &str,
    body: &[u8],
    max_message_bytes: usize,
) -> Result<Request, ProtocolError> {
    if max_message_bytes == 0 {
        return Err(ProtocolError::ResourceExhausted);
    }
    let payload = if matches!(method, "Start" | "Connect") {
        let header: &[u8; 5] = body
            .get(..5)
            .and_then(|slice| slice.try_into().ok())
            .ok_or(ProtocolError::InvalidArgument)?;
        let size = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if size > max_message_bytes {
            return Err(ProtocolError::ResourceExhausted);
        }
        if header[0] != 0 || body.len().checked_sub(5) != Some(size) {
            return Err(ProtocolError::InvalidArgument);
        }
        &body[5..]
    } else {
        if body.len() > max_message_bytes {
            return Err(ProtocolError::ResourceExhausted);
        }
        body
    };
    if !matches!(
        method,
        "Start" | "Connect" | "List" | "SendInput" | "CloseStdin" | "SendSignal" | "Update"
    ) {
        return Err(ProtocolError::Unimplemented);
    }
    let prepared = prepare_command(payload)?;
    let payload = prepared.as_ref();
    match method {
        "Start" => Ok(Request::Start(parse(payload)?)),
        "Connect" => Ok(Request::Connect(parse::<SelectRequest>(payload)?.process)),
        "CloseStdin" => Ok(Request::CloseStdin(
            parse::<SelectRequest>(payload)?.process,
        )),
        "List" => {
            parse::<Empty>(payload)?;
            Ok(Request::List)
        }
        "SendInput" => {
            let request: InputRequest = parse(payload)?;
            Ok(Request::SendInput {
                process: request.process,
                input: request.input,
            })
        }
        "SendSignal" => {
            let request: SignalRequest = parse(payload)?;
            Ok(Request::SendSignal {
                process: request.process,
                signal: request.signal.unwrap_or(Signal::Unspecified),
            })
        }
        "Update" => {
            let request: UpdateRequest = parse(payload)?;
            Ok(Request::Update {
                process: request.process,
                pty: request.pty,
            })
        }
        _ => Err(ProtocolError::Unimplemented),
    }
}

pub(crate) fn parse<'de, T: Deserialize<'de>>(payload: &'de [u8]) -> Result<T, ProtocolError> {
    serde_json::from_slice::<Object<T>>(payload)
        .map(|object| object.0)
        .map_err(|_| ProtocolError::InvalidArgument)
}

// Serde structs also accept positional arrays by default. Protobuf messages
// must be JSON objects; enforce that without reparsing into an untyped Value
// (which would erase duplicate keys before the typed decoder sees them).
struct Object<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
            type Value = Object<T>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a message object")
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                map: M,
            ) -> Result<Self::Value, M::Error> {
                T::deserialize(serde::de::value::MapAccessDeserializer::new(map)).map(Object)
            }
        }
        deserializer.deserialize_map(Visitor(std::marker::PhantomData))
    }
}

fn optional_object<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Ok(Option::<Object<T>>::deserialize(deserializer)?.map(|object| object.0))
}

pub(crate) fn null_default<'de, D: serde::Deserializer<'de>, T: Deserialize<'de> + Default>(
    deserializer: D,
) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Deserialize)]
struct Uint32(#[serde(deserialize_with = "uint32")] u32);

#[derive(Deserialize)]
struct Bytes(#[serde(deserialize_with = "bytes")] Vec<u8>);

/// Already observed, not inferred from EOF or transport success. On Linux a
/// signal exit has code -1 and exited=false. Nonzero exits retain their error.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct End<'a> {
    #[serde(skip_serializing_if = "is_zero")]
    pub exit_code: i32,
    #[serde(skip_serializing_if = "is_zero")]
    pub exited: bool,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<&'a str>,
}

pub enum Event<'a> {
    Start(u32),
    Output(Channel, &'a [u8]),
    End(End<'a>),
    KeepAlive,
}

/// Encode one event, bounded before base64 allocation and during JSON writing.
/// The owning stream must separately enforce start/end order and consume a
/// runtime output chunk only after the downstream owner accepts it.
/// # Errors
/// Refuses an event that cannot fit in the message budget or Connect u32 size.
pub fn encode_event(event: Event<'_>, limit: usize) -> Result<Vec<u8>, ProtocolError> {
    #[derive(Serialize)]
    struct Response<'a> {
        event: EncodedEvent<'a>,
    }
    #[derive(Serialize)]
    #[serde(rename_all = "lowercase")]
    enum EncodedEvent<'a> {
        Start {
            #[serde(skip_serializing_if = "is_zero")]
            pid: u32,
        },
        Data(EncodedOutput),
        End(End<'a>),
        Keepalive {},
    }
    #[derive(Serialize)]
    #[serde(rename_all = "lowercase")]
    enum EncodedOutput {
        Stdout(String),
        Stderr(String),
        Pty(String),
    }
    let event = match event {
        Event::Start(pid) => EncodedEvent::Start { pid },
        Event::Output(channel, bytes) => {
            if base64::encoded_len(bytes.len(), true)
                .is_none_or(|len| len > limit.min(u32::MAX as usize))
            {
                return Err(ProtocolError::ResourceExhausted);
            }
            let encoded = STANDARD.encode(bytes);
            EncodedEvent::Data(match channel {
                Channel::Stdout => EncodedOutput::Stdout(encoded),
                Channel::Stderr => EncodedOutput::Stderr(encoded),
                Channel::Pty => EncodedOutput::Pty(encoded),
            })
        }
        Event::End(end) => EncodedEvent::End(end),
        Event::KeepAlive => EncodedEvent::Keepalive {},
    };
    encode_frame(0, &Response { event }, limit)
}

/// Connect end-of-stream is separate from the process End event. Call only
/// after the operation's full completion; an abrupt EOF is never a success.
/// # Errors
/// Refuses an insufficient message budget. Error codes contain no request data.
pub fn encode_stream_end(
    error: Option<ProtocolError>,
    limit: usize,
) -> Result<Vec<u8>, ProtocolError> {
    let value = error.map_or_else(
        || serde_json::json!({}),
        |error| serde_json::json!({"error": {"code": error.code()}}),
    );
    encode_frame(2, &value, limit)
}

fn encode_frame<T: Serialize>(
    flags: u8,
    value: &T,
    limit: usize,
) -> Result<Vec<u8>, ProtocolError> {
    let mut writer = BoundedWriter {
        bytes: vec![flags, 0, 0, 0, 0],
        remaining: limit.min(u32::MAX as usize),
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| ProtocolError::ResourceExhausted)?;
    let size =
        u32::try_from(writer.bytes.len() - 5).map_err(|_| ProtocolError::ResourceExhausted)?;
    writer.bytes[1..5].copy_from_slice(&size.to_be_bytes());
    Ok(writer.bytes)
}

struct BoundedWriter {
    bytes: Vec<u8>,
    remaining: usize,
}

pub(crate) fn encode_json<T: Serialize>(value: &T, limit: usize) -> Result<Vec<u8>, ProtocolError> {
    if limit == 0 {
        return Err(ProtocolError::ResourceExhausted);
    }
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        remaining: limit,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| ProtocolError::ResourceExhausted)?;
    Ok(writer.bytes)
}

impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(io::Error::other("message budget exceeded"));
        }
        let required = self.bytes.len() + bytes.len();
        if required > self.bytes.capacity() {
            // Vec's implicit growth can exceed the configured wire budget.
            // Grow geometrically only within it, before appending any bytes.
            let limit = self.bytes.len() + self.remaining;
            let capacity = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(required)
                .min(limit);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(|_| io::Error::other("message allocation failed"))?;
        }
        self.bytes.extend_from_slice(bytes);
        self.remaining -= bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
