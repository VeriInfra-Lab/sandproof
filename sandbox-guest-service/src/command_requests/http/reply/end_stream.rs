use crate::{file_download::Error, go_json::Sink, go_quote};
use sandproof_guest_runtime::protocol::{EndStreamError, EndStreamReason};

pub(super) fn message(error: EndStreamError, put: &mut Sink<'_>) -> Result<(), Error> {
    put(b"unmarshal end stream message: ")?;
    match error.reason() {
        EndStreamReason::Eof => put(b"unexpected end of JSON input"),
        EndStreamReason::Syntax { byte, context } => {
            put(b"invalid character '")?;
            match byte {
                b'\'' => put(br"\'")?,
                b'"' => put(b"\"")?,
                _ => {
                    // At most one Unicode scalar (Latin-1 byte), quoted by Go.
                    let mut quoted = Vec::with_capacity(12);
                    go_quote::quoted(
                        char::from(byte).encode_utf8(&mut [0; 4]).as_bytes(),
                        &mut |part| {
                            quoted.extend_from_slice(part);
                            Ok(())
                        },
                    )?;
                    put(&quoted[1..quoted.len() - 1])?;
                }
            }
            put(b"' ")?;
            put(context.as_bytes())
        }
        EndStreamReason::Type {
            value,
            field,
            target,
        } => {
            put(b"json: cannot unmarshal ")?;
            put(value.as_bytes())?;
            if field.is_empty() {
                put(b" into Go value of type ")?;
            } else {
                put(b" into Go struct field connectEndStreamMessage.")?;
                put(field.as_bytes())?;
                put(b" of type ")?;
            }
            put(target.as_bytes())
        }
        EndStreamReason::Base64 { offset } => {
            put(b"decode base64: illegal base64 data at input byte ")?;
            put(offset.to_string().as_bytes())
        }
    }
}
