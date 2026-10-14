//! Shared fixed Go JSON escaping and count-before-allocation payload emission.
use crate::file_download::Error;

pub(crate) type Sink<'a> = dyn FnMut(&[u8]) -> Result<(), Error> + 'a;

pub(crate) fn string_content(bytes: &[u8], put: &mut Sink<'_>) -> Result<(), Error> {
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            match c {
                '"' => put(b"\\\"")?,
                '\\' => put(b"\\\\")?,
                '\n' => put(b"\\n")?,
                '\r' => put(b"\\r")?,
                '\t' => put(b"\\t")?,
                '\u{8}' => put(b"\\b")?,
                '\u{c}' => put(b"\\f")?,
                '<' => put(b"\\u003c")?,
                '>' => put(b"\\u003e")?,
                '&' => put(b"\\u0026")?,
                '\u{2028}' => put(b"\\u2028")?,
                '\u{2029}' => put(b"\\u2029")?,
                c if c < ' ' => {
                    const HEX: &[u8] = b"0123456789abcdef";
                    let value = c as usize;
                    put(&[b'\\', b'u', b'0', b'0', HEX[value >> 4], HEX[value & 15]])?;
                }
                c => put(c.encode_utf8(&mut [0; 4]).as_bytes())?,
            }
        }
        for _ in chunk.invalid() {
            put(b"\\ufffd")?;
        }
    }
    Ok(())
}

pub(crate) fn bounded(
    limit: u64,
    emit: impl Fn(&mut Sink<'_>) -> Result<(), Error>,
) -> Result<Vec<u8>, Error> {
    let mut length = 0usize;
    emit(&mut |bytes| {
        length = length
            .checked_add(bytes.len())
            .filter(|n| *n as u64 <= limit && isize::try_from(*n).is_ok())
            .ok_or(Error::ResponseLimit)?;
        Ok(())
    })?;
    let mut output = Vec::with_capacity(length);
    emit(&mut |bytes| {
        output.extend_from_slice(bytes);
        Ok(())
    })?;
    Ok(output)
}
