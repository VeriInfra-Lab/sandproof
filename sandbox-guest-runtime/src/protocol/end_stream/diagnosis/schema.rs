//! Go saves regular field type errors but custom error/detail decoders return
//! immediately. Later custom failures can therefore outrank an earlier field.
// Base64 observation derived from Go 1.26.8 encoding/base64/base64.go.
// Copyright 2009 The Go Authors. All rights reserved. BSD license: GO-LICENSE.
use super::EndStreamReason as Error;
use serde::{
    Deserializer,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_json::value::RawValue;
use std::fmt;

#[derive(Clone, Copy)]
enum Node {
    End,
    Error,
    Detail,
    Metadata,
    Details,
    Strings,
    String,
}
impl Node {
    fn target(self) -> &'static str {
        match self {
            Self::End => "connect.connectEndStreamMessage",
            Self::Error => {
                r#"struct { Code string "json:\"code\""; Message string "json:\"message\""; Details []*connect.connectWireDetail "json:\"details\"" }"#
            }
            Self::Detail => {
                r#"struct { Type string "json:\"type\""; Value string "json:\"value\"" }"#
            }
            Self::Metadata => "http.Header",
            Self::Details => "[]*connect.connectWireDetail",
            Self::Strings => "[]string",
            Self::String => "string",
        }
    }
    fn marker(self) -> u8 {
        match self {
            Self::String => b'"',
            Self::Details | Self::Strings => b'[',
            _ => b'{',
        }
    }
    fn custom(self) -> bool {
        matches!(self, Self::Error | Self::Detail)
    }
}
#[derive(Clone, Copy, Default)]
struct Outcome {
    error: Option<Error>,
    fatal: bool,
}
impl Outcome {
    fn include(&mut self, other: Self, custom: bool) {
        if other.error.is_some() && (custom || other.fatal) {
            self.error = other.error;
            self.fatal = true;
        } else if self.error.is_none() {
            self.error = other.error;
        }
    }
}

pub(super) fn observe(input: &[u8]) -> Option<Error> {
    let raw = serde_json::from_slice::<&RawValue>(input).ok()?;
    node(raw, Node::End, "").ok()?.error
}
fn node(raw: &RawValue, schema: Node, field: &'static str) -> Result<Outcome, serde_json::Error> {
    let first = raw.get().as_bytes()[0];
    if first == b'n' {
        return Ok(Outcome::default());
    }
    if first != schema.marker() {
        let value = match first {
            b'{' => "object",
            b'[' => "array",
            b'"' => "string",
            b't' | b'f' => "bool",
            _ => "number",
        };
        return Ok(Outcome {
            error: Some(Error::Type {
                value,
                field,
                target: schema.target(),
            }),
            fatal: false,
        });
    }
    if matches!(schema, Node::String) {
        return Ok(Outcome::default());
    }
    let mut decoder = serde_json::Deserializer::from_str(raw.get());
    let visitor = Fields { schema, field };
    if first == b'[' {
        decoder.deserialize_seq(visitor)
    } else {
        decoder.deserialize_map(visitor)
    }
}
struct Fields {
    schema: Node,
    field: &'static str,
}
impl<'de> Visitor<'de> for Fields {
    type Value = Outcome;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("validated EOS JSON")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut values: A) -> Result<Outcome, A::Error> {
        let child = if matches!(self.schema, Node::Details) {
            Node::Detail
        } else {
            Node::String
        };
        let mut result = Outcome::default();
        while let Some(raw) = values.next_element::<&RawValue>()? {
            if !result.fatal {
                let observed = node(raw, child, self.field).map_err(serde::de::Error::custom)?;
                result.include(observed, child.custom());
            }
        }
        Ok(result)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut fields: A) -> Result<Outcome, A::Error> {
        let mut result = Outcome::default();
        let mut binary = String::new();
        while let Some(key) = fields.next_key::<String>()? {
            let raw = fields.next_value::<&RawValue>()?;
            if result.fatal {
                continue;
            }
            let Some((child, field)) = child(self.schema, &key) else {
                continue;
            };
            let observed = node(raw, child, field).map_err(serde::de::Error::custom)?;
            if matches!(self.schema, Node::Detail)
                && field == "error.details.value"
                && raw.get().starts_with('"')
            {
                binary = serde_json::from_str(raw.get()).map_err(serde::de::Error::custom)?;
            }
            result.include(observed, child.custom());
        }
        if matches!(self.schema, Node::Detail) && result.error.is_none() {
            result.error = binary_error(binary.as_bytes()).map(|offset| Error::Base64 { offset });
        }
        Ok(result)
    }
}
fn child(schema: Node, key: &str) -> Option<(Node, &'static str)> {
    use super::super::field;
    match schema {
        Node::Metadata => Some((Node::Strings, "metadata")),
        Node::End if field(key, "error") => Some((Node::Error, "error")),
        Node::End if field(key, "metadata") => Some((Node::Metadata, "metadata")),
        Node::Error if field(key, "code") => Some((Node::String, "error.code")),
        Node::Error if field(key, "message") => Some((Node::String, "error.message")),
        Node::Error if field(key, "details") => Some((Node::Details, "error.details")),
        Node::Detail if field(key, "type") => Some((Node::String, "error.details.type")),
        Node::Detail if field(key, "value") => Some((Node::String, "error.details.value")),
        _ => None,
    }
}

// Go 1.26.8 encoding/base64 decodeQuantum, without output allocation. CR/LF
// consume source offsets but not quantum digits. Nonzero unused bits are legal.
fn binary_error(input: &[u8]) -> Option<usize> {
    let padding = input.len().is_multiple_of(4);
    let mut offset = 0;
    loop {
        let mut digits = 0;
        while digits < 4 {
            let Some(&byte) = input.get(offset) else {
                return if digits == 0 || (digits > 1 && !padding) {
                    None
                } else {
                    Some(offset - digits)
                };
            };
            offset += 1;
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => {
                    digits += 1;
                    continue;
                }
                b'\r' | b'\n' => continue,
                b'=' if padding && digits >= 2 => (),
                _ => return Some(offset - 1),
            }
            if digits == 2 {
                while input
                    .get(offset)
                    .is_some_and(|b| matches!(b, b'\r' | b'\n'))
                {
                    offset += 1;
                }
                match input.get(offset) {
                    None => return Some(input.len()),
                    Some(b'=') => offset += 1,
                    _ => return Some(offset - 1),
                }
            }
            while input
                .get(offset)
                .is_some_and(|b| matches!(b, b'\r' | b'\n'))
            {
                offset += 1;
            }
            return (offset < input.len()).then_some(offset);
        }
    }
}
