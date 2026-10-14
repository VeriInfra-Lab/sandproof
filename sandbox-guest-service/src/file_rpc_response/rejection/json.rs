use crate::{file_download::Error, go_json::Sink, go_quote};
use sandproof_guest_runtime::file_protocol::{JsonError, JsonField, JsonReason};

pub(super) fn message(error: &JsonError, put: &mut Sink<'_>) -> Result<(), Error> {
    put(b"unmarshal message: ")?;
    if error.reason() == JsonReason::Empty {
        return put(b"zero-length payload is not a valid JSON object");
    }
    put(b"unmarshal into *filesystem.")?;
    put(error.method().as_bytes())?;
    // The original protobuf library chooses ASCII space or NBSP from its Go
    // binary hash. Emit its ASCII spelling; oracle comparison normalizes only
    // that build-dependent prefix, never whitespace in user data or diagnostics.
    put(b"Request: proto: ")?;
    match error.reason() {
        JsonReason::UnexpectedEof => return put(b"unexpected EOF"),
        JsonReason::Depth => return put(b"exceeded max recursion depth"),
        JsonReason::Duplicate | JsonReason::InvalidField(_) => {}
        _ => put(b"syntax error ")?,
    }
    let (line, column) = error.position();
    put(b"(line ")?;
    put(line.to_string().as_bytes())?;
    put(b":")?;
    put(column.to_string().as_bytes())?;
    put(b"): ")?;
    match error.reason() {
        JsonReason::UnexpectedToken => {
            put(b"unexpected token ")?;
            put(error.raw())
        }
        JsonReason::InvalidValue => {
            put(b"invalid value ")?;
            put(error.raw())
        }
        JsonReason::MissingColon(byte) => {
            put(b"unexpected character ")?;
            put(char::from(byte).encode_utf8(&mut [0; 4]).as_bytes())?;
            put(b", missing \":\" after field name")
        }
        JsonReason::InvalidUtf8 => put(b"invalid UTF-8 in string"),
        JsonReason::InvalidCharacter(byte) => {
            put(b"invalid character '")?;
            control(byte, put)?;
            put(b"' in string")
        }
        JsonReason::InvalidEscape => {
            put(b"invalid escape code ")?;
            go_quote::quoted(error.raw(), put)?;
            put(b" in string")
        }
        JsonReason::Duplicate => {
            put(b"duplicate field ")?;
            put(error.raw())
        }
        JsonReason::InvalidField(field) => {
            put(b"invalid value for ")?;
            put(if field == JsonField::Depth {
                b"uint32"
            } else {
                b"string"
            })?;
            put(b" field ")?;
            put(field.name().as_bytes())?;
            put(b": ")?;
            put(error.raw())
        }
        JsonReason::Empty | JsonReason::UnexpectedEof | JsonReason::Depth => {
            unreachable!("unpositioned reason returned above")
        }
    }
}

fn control(byte: u8, put: &mut Sink<'_>) -> Result<(), Error> {
    match byte {
        7 => put(b"\\a"),
        8 => put(b"\\b"),
        12 => put(b"\\f"),
        b'\n' => put(b"\\n"),
        b'\r' => put(b"\\r"),
        b'\t' => put(b"\\t"),
        11 => put(b"\\v"),
        _ => {
            const HEX: &[u8] = b"0123456789abcdef";
            put(&[
                b'\\',
                b'x',
                HEX[usize::from(byte >> 4)],
                HEX[usize::from(byte & 15)],
            ])
        }
    }
}
