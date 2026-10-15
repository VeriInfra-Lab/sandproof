// Fixed protobuf v1.36.12 JSON token order and lexical failure rules.
// Copyright 2018 The Go Authors. All rights reserved.
// BSD license: PROTOBUF-LICENSE.
use super::{Failure, JsonReason};
use std::ops::Range;
mod string;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Start,
    Eof,
    Null,
    Bool,
    Number,
    String,
    Name,
    ObjectOpen,
    ObjectClose,
    ArrayOpen,
    ArrayClose,
    Comma,
}
#[derive(Clone)]
pub(super) struct Token {
    pub(super) kind: Kind,
    pub(super) range: Range<usize>,
}
pub(super) struct Decoder<'a> {
    body: &'a [u8],
    offset: usize,
    last: Kind,
    stack: Vec<Kind>,
    peeked: Option<Result<Token, Failure>>,
}
impl<'a> Decoder<'a> {
    pub(super) fn new(body: &'a [u8]) -> Self {
        Self {
            body,
            offset: 0,
            last: Kind::Start,
            stack: Vec::new(),
            peeked: None,
        }
    }
    pub(super) fn read(&mut self) -> Result<Token, Failure> {
        if let Some(result) = self.peeked.take() {
            return result;
        }
        self.read_next()
    }
    pub(super) fn peek(&mut self) -> Result<Token, Failure> {
        if self.peeked.is_none() {
            self.peeked = Some(self.read_next());
        }
        self.peeked
            .as_ref()
            .expect("cached token or lexical failure")
            .clone()
    }
    fn read_next(&mut self) -> Result<Token, Failure> {
        loop {
            let mut token = self.next()?;
            let invalid = || Failure::token(JsonReason::UnexpectedToken, &token);
            match token.kind {
                Kind::Eof if !self.stack.is_empty() => {
                    return Err(Failure::at(JsonReason::UnexpectedEof, self.offset));
                }
                Kind::Null | Kind::Bool | Kind::Number if !self.value_next() => {
                    return Err(invalid());
                }
                Kind::String if !self.value_next() => {
                    if !matches!(self.last, Kind::ObjectOpen | Kind::Comma) {
                        return Err(invalid());
                    }
                    match self.body.get(self.offset) {
                        None => return Err(Failure::at(JsonReason::UnexpectedEof, self.offset)),
                        Some(b':') => {
                            token.kind = Kind::Name;
                            self.offset += 1;
                            self.whitespace();
                        }
                        Some(byte) => {
                            return Err(Failure::at(JsonReason::MissingColon(*byte), self.offset));
                        }
                    }
                }
                Kind::ObjectOpen | Kind::ArrayOpen => {
                    if !self.value_next() {
                        return Err(invalid());
                    }
                    if self.stack.len() == 10_000 {
                        return Err(Failure::at(JsonReason::Depth, token.range.start));
                    }
                    self.stack.push(token.kind);
                }
                Kind::ObjectClose => {
                    if self.stack.last() != Some(&Kind::ObjectOpen)
                        || matches!(self.last, Kind::Name | Kind::Comma)
                    {
                        return Err(invalid());
                    }
                    self.stack.pop();
                }
                Kind::ArrayClose => {
                    if self.stack.last() != Some(&Kind::ArrayOpen) || self.last == Kind::Comma {
                        return Err(invalid());
                    }
                    self.stack.pop();
                }
                Kind::Comma
                    if self.stack.is_empty()
                        || !matches!(
                            self.last,
                            Kind::Null
                                | Kind::Bool
                                | Kind::Number
                                | Kind::String
                                | Kind::ObjectClose
                                | Kind::ArrayClose
                        ) =>
                {
                    return Err(invalid());
                }
                _ => {}
            }
            self.last = token.kind;
            if token.kind != Kind::Comma {
                return Ok(token);
            }
        }
    }
    fn value_next(&self) -> bool {
        match self.stack.last() {
            None => self.last == Kind::Start,
            Some(Kind::ObjectOpen) => self.last == Kind::Name,
            Some(Kind::ArrayOpen) => matches!(self.last, Kind::ArrayOpen | Kind::Comma),
            _ => false,
        }
    }
    fn whitespace(&mut self) {
        while matches!(
            self.body.get(self.offset),
            Some(b' ' | b'\n' | b'\r' | b'\t')
        ) {
            self.offset += 1;
        }
    }
    fn next(&mut self) -> Result<Token, Failure> {
        self.whitespace();
        let start = self.offset;
        let input = &self.body[start..];
        let (kind, len) = match input.first() {
            None => (Kind::Eof, 0),
            Some(b'n') if literal(input, b"null") => (Kind::Null, 4),
            Some(b't') if literal(input, b"true") => (Kind::Bool, 4),
            Some(b'f') if literal(input, b"false") => (Kind::Bool, 5),
            Some(b'"') => (Kind::String, string::length(input, start)?),
            Some(b'{') => (Kind::ObjectOpen, 1),
            Some(b'}') => (Kind::ObjectClose, 1),
            Some(b'[') => (Kind::ArrayOpen, 1),
            Some(b']') => (Kind::ArrayClose, 1),
            Some(b',') => (Kind::Comma, 1),
            Some(b'-' | b'0'..=b'9') if number(input).is_some() => {
                (Kind::Number, number(input).unwrap())
            }
            _ => {
                let len = input
                    .iter()
                    .take(32)
                    .take_while(|b| not_delimiter(**b))
                    .count();
                let len = if len == 0 {
                    rune_length(input).unwrap_or(1)
                } else {
                    len
                };
                return Err(Failure {
                    reason: JsonReason::InvalidValue,
                    position: start,
                    raw: start..start + len,
                });
            }
        };
        self.offset += len;
        self.whitespace();
        Ok(Token {
            kind,
            range: start..start + len,
        })
    }
}

fn literal(input: &[u8], literal: &[u8]) -> bool {
    input.starts_with(literal) && input.get(literal.len()).is_none_or(|b| !not_delimiter(*b))
}
fn not_delimiter(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'+' | b'.' | b'_')
}
fn number(input: &[u8]) -> Option<usize> {
    let mut pos = usize::from(input.first() == Some(&b'-'));
    match input.get(pos)? {
        b'0' => pos += 1,
        b'1'..=b'9' => {
            pos += 1;
            while input.get(pos).is_some_and(u8::is_ascii_digit) {
                pos += 1;
            }
        }
        _ => return None,
    }
    if input.get(pos) == Some(&b'.') && input.get(pos + 1).is_some_and(u8::is_ascii_digit) {
        pos += 2;
        while input.get(pos).is_some_and(u8::is_ascii_digit) {
            pos += 1;
        }
    }
    if matches!(input.get(pos), Some(b'e' | b'E')) && input.get(pos + 1).is_some() {
        pos += 1;
        if matches!(input.get(pos), Some(b'+' | b'-')) {
            pos += 1;
            if pos == input.len() {
                return None;
            }
        }
        while input.get(pos).is_some_and(u8::is_ascii_digit) {
            pos += 1;
        }
    }
    input
        .get(pos)
        .is_none_or(|b| !not_delimiter(*b))
        .then_some(pos)
}

pub(super) fn rune_length(bytes: &[u8]) -> Option<usize> {
    let len = match bytes.first()? {
        0..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    std::str::from_utf8(bytes.get(..len)?).ok().map(|_| len)
}
