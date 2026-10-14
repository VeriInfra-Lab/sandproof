//! Fixed envd GET/POST query binding: Go URL.Query and oapi runtime v1.6.0.
//! Parsing grants no authorization and performs no path/account/file operation.

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub raw_bytes: usize,
    /// All nonempty fields, including unknown, malformed and repeated fields.
    pub fields: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parameter {
    Path,
    Username,
    Signature,
    SignatureExpiration,
}
impl Parameter {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Username => "username",
            Self::Signature => "signature",
            Self::SignatureExpiration => "signature_expiration",
        }
    }
}
const PARAMETERS: [Parameter; 4] = [
    Parameter::Path,
    Parameter::Username,
    Parameter::Signature,
    Parameter::SignatureExpiration,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegerError {
    Syntax,
    Range,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidConfiguration,
    ResourceExhausted,
    Duplicate(Parameter),
    Integer {
        value: Vec<u8>,
        reason: IntegerError,
    },
    UnsupportedText(Parameter),
}

/// Go strings retain bytes, including invalid UTF-8. Optional empty values stay
/// distinct from absence. Consumers with text-only paths must call `into_text`.
#[derive(Debug, PartialEq, Eq)]
pub struct Query {
    pub path: Option<Vec<u8>>,
    pub username: Option<Vec<u8>>,
    pub signature: Option<Vec<u8>>,
    pub signature_expiration: Option<i64>,
}
pub struct TextQuery {
    pub path: Option<String>,
    pub username: Option<String>,
    pub signature: Option<String>,
    pub signature_expiration: Option<i64>,
}
impl Query {
    /// # Errors
    /// The existing UTF-8 path/account adapters cannot accept arbitrary Go bytes.
    pub fn into_text(self) -> Result<TextQuery, Error> {
        let text = |value: Option<Vec<u8>>, parameter| {
            value
                .map(String::from_utf8)
                .transpose()
                .map_err(|_| Error::UnsupportedText(parameter))
        };
        Ok(TextQuery {
            path: text(self.path, Parameter::Path)?,
            username: text(self.username, Parameter::Username)?,
            signature: text(self.signature, Parameter::Signature)?,
            signature_expiration: self.signature_expiration,
        })
    }
}

/// Bind in generated path/username/signature/expiration order, independent of
/// wire order. As URL.Query does, skip a whole field with bad percent escapes
/// or a literal semicolon; preserve other fields. '+' is space, '%2B' is '+'.
/// # Errors
/// Invalid budgets, total query/field limits, duplicate scalar or invalid int64.
pub fn parse(raw: &[u8], limits: Limits) -> Result<Query, Error> {
    if limits.raw_bytes == 0
        || limits.fields == 0
        || limits.raw_bytes > isize::MAX as usize / 16
        || limits.fields > isize::MAX as usize / 16
    {
        return Err(Error::InvalidConfiguration);
    }
    if raw.len() > limits.raw_bytes
        || raw.split(|b| *b == b'&').filter(|v| !v.is_empty()).count() > limits.fields
    {
        return Err(Error::ResourceExhausted);
    }
    let mut values: [Option<Vec<u8>>; 4] = Default::default();
    let mut duplicate = [false; 4];
    for field in raw.split(|b| *b == b'&').filter(|v| !v.is_empty()) {
        if field.contains(&b';') {
            continue;
        }
        let (key, value) = field
            .iter()
            .position(|b| *b == b'=')
            .map_or((field, &[][..]), |i| (&field[..i], &field[i + 1..]));
        let Some(key) = unescape(key) else { continue };
        let Some(index) = PARAMETERS.iter().position(|p| p.name().as_bytes() == key) else {
            continue;
        };
        let Some(value) = unescape(value) else {
            continue;
        };
        if values[index].is_some() {
            duplicate[index] = true;
        } else {
            values[index] = Some(value);
        }
    }
    for (index, parameter) in PARAMETERS.iter().enumerate() {
        if duplicate[index] {
            return Err(Error::Duplicate(*parameter));
        }
    }
    let expiration = values[3]
        .take()
        .map(|value| integer(&value).map_err(|reason| Error::Integer { value, reason }))
        .transpose()?;
    Ok(Query {
        path: values[0].take(),
        username: values[1].take(),
        signature: values[2].take(),
        signature_expiration: expiration,
    })
}
fn unescape(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut result = Vec::with_capacity(bytes.len());
    let mut remaining = bytes;
    while let Some((&byte, tail)) = remaining.split_first() {
        remaining = tail;
        result.push(match byte {
            b'+' => b' ',
            b'%' => {
                let high = hex(*remaining.first()?)?;
                let low = hex(*remaining.get(1)?)?;
                remaining = &remaining[2..];
                high * 16 + low
            }
            b => b,
        });
    }
    Some(result)
}
fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
fn integer(value: &[u8]) -> Result<i64, IntegerError> {
    let negative = value.first() == Some(&b'-');
    let digits = if matches!(value.first(), Some(b'-' | b'+')) {
        &value[1..]
    } else {
        value
    };
    if digits.is_empty() {
        return Err(IntegerError::Syntax);
    }
    // Go ParseInt first uses ParseUint with 64 bits; an earlier unsigned
    // overflow wins over a later bad byte, signed range is checked afterward.
    let mut number = 0u64;
    for byte in digits {
        if !byte.is_ascii_digit() {
            return Err(IntegerError::Syntax);
        }
        number = number
            .checked_mul(10)
            .and_then(|n| n.checked_add(u64::from(byte - b'0')))
            .ok_or(IntegerError::Range)?;
    }
    if negative && number == 1 << 63 {
        return Ok(i64::MIN);
    }
    let number = i64::try_from(number).map_err(|_| IntegerError::Range)?;
    Ok(if negative { -number } else { number })
}

#[cfg(test)]
mod tests;
