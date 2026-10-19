// Behavior derived from Go 1.26.8 encoding/json/scanner.go.
// Copyright 2010 The Go Authors. All rights reserved.
// BSD license: GO-LICENSE.
use super::EndStreamReason as Error;

#[derive(Clone, Copy)]
enum State {
    Begin,
    ArrayStart,
    KeyStart,
    Key,
    String,
    Escape,
    Hex(u8),
    Minus,
    Zero,
    Integer,
    Dot,
    Decimal,
    Exponent,
    Sign,
    Digits,
    Literal(&'static [u8], usize),
    End,
    Top,
}
#[derive(Clone, Copy)]
enum Container {
    Key,
    Value,
    Array,
}
struct Scanner {
    state: State,
    stack: Vec<Container>,
    top: bool,
}

pub(super) fn observe(input: &[u8]) -> Option<Error> {
    let mut scanner = Scanner {
        state: State::Begin,
        stack: Vec::new(),
        top: false,
    };
    for &byte in input {
        if let Err(error) = scanner.step(byte) {
            return Some(error);
        }
    }
    if scanner.top {
        return None;
    }
    let result = scanner.step(b' ');
    // Go marks scalar top-level completion before checking the extra byte.
    if scanner.top {
        None
    } else {
        Some(result.err().unwrap_or(Error::Eof))
    }
}
fn error(byte: u8, context: &'static str) -> Error {
    Error::Syntax { byte, context }
}
fn space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}
impl Scanner {
    fn step(&mut self, byte: u8) -> Result<(), Error> {
        use State as S;
        match self.state {
            S::ArrayStart if byte == b']' => self.end(byte),
            S::Begin | S::ArrayStart => self.begin(byte),
            S::KeyStart if byte == b'}' => {
                *self.stack.last_mut().expect("object") = Container::Value;
                self.end(byte)
            }
            S::KeyStart | S::Key => {
                if space(byte) {
                    return Ok(());
                }
                if byte != b'"' {
                    return Err(error(byte, "looking for beginning of object key string"));
                }
                self.state = S::String;
                Ok(())
            }
            S::String => {
                match byte {
                    b'"' => self.state = S::End,
                    b'\\' => self.state = S::Escape,
                    0..=31 => return Err(error(byte, "in string literal")),
                    _ => (),
                }
                Ok(())
            }
            S::Escape => {
                self.state = match byte {
                    b'b' | b'f' | b'n' | b'r' | b't' | b'\\' | b'/' | b'"' => S::String,
                    b'u' => S::Hex(4),
                    _ => return Err(error(byte, "in string escape code")),
                };
                Ok(())
            }
            S::Hex(left) => {
                if !byte.is_ascii_hexdigit() {
                    return Err(error(byte, "in \\u hexadecimal character escape"));
                }
                self.state = if left == 1 {
                    S::String
                } else {
                    S::Hex(left - 1)
                };
                Ok(())
            }
            S::Minus => {
                self.state = match byte {
                    b'0' => S::Zero,
                    b'1'..=b'9' => S::Integer,
                    _ => return Err(error(byte, "in numeric literal")),
                };
                Ok(())
            }
            S::Integer | S::Decimal | S::Digits if byte.is_ascii_digit() => Ok(()),
            S::Zero | S::Integer => self.number_end(byte, true),
            S::Dot => {
                self.required_digit(byte, S::Decimal, "after decimal point in numeric literal")
            }
            S::Decimal => self.number_end(byte, false),
            S::Exponent if matches!(byte, b'+' | b'-') => {
                self.state = S::Sign;
                Ok(())
            }
            S::Exponent | S::Sign => {
                self.required_digit(byte, S::Digits, "in exponent of numeric literal")
            }
            S::Digits | S::End => self.end(byte),
            S::Top => {
                if space(byte) {
                    Ok(())
                } else {
                    Err(error(byte, "after top-level value"))
                }
            }
            S::Literal(word, at) => {
                if byte != word[at] {
                    return Err(error(byte, literal_context(word, at)));
                }
                self.state = if at + 1 == word.len() {
                    S::End
                } else {
                    S::Literal(word, at + 1)
                };
                Ok(())
            }
        }
    }
    fn begin(&mut self, byte: u8) -> Result<(), Error> {
        use State as S;
        if space(byte) {
            return Ok(());
        }
        self.state = match byte {
            b'{' => {
                self.push(byte, Container::Key)?;
                S::KeyStart
            }
            b'[' => {
                self.push(byte, Container::Array)?;
                S::ArrayStart
            }
            b'"' => S::String,
            b'-' => S::Minus,
            b'0' => S::Zero,
            b'1'..=b'9' => S::Integer,
            b't' => S::Literal(b"true", 1),
            b'f' => S::Literal(b"false", 1),
            b'n' => S::Literal(b"null", 1),
            _ => return Err(error(byte, "looking for beginning of value")),
        };
        Ok(())
    }
    fn push(&mut self, byte: u8, kind: Container) -> Result<(), Error> {
        if self.stack.len() == 10_000 {
            return Err(error(byte, "exceeded max depth"));
        }
        self.stack.push(kind);
        Ok(())
    }
    fn end(&mut self, byte: u8) -> Result<(), Error> {
        let Some(container) = self.stack.last_mut() else {
            self.top = true;
            self.state = State::Top;
            return self.step(byte);
        };
        if space(byte) {
            self.state = State::End;
            return Ok(());
        }
        self.state = match (*container, byte) {
            (Container::Key, b':') => {
                *container = Container::Value;
                State::Begin
            }
            (Container::Key, _) => return Err(error(byte, "after object key")),
            (Container::Value, b',') => {
                *container = Container::Key;
                State::Key
            }
            (Container::Array, b',') => State::Begin,
            (Container::Value, b'}') | (Container::Array, b']') => {
                self.stack.pop();
                if self.stack.is_empty() {
                    self.top = true;
                    State::Top
                } else {
                    State::End
                }
            }
            (Container::Value, _) => return Err(error(byte, "after object key:value pair")),
            (Container::Array, _) => return Err(error(byte, "after array element")),
        };
        Ok(())
    }
    fn number_end(&mut self, byte: u8, decimal: bool) -> Result<(), Error> {
        self.state = match byte {
            b'.' if decimal => State::Dot,
            b'e' | b'E' => State::Exponent,
            _ => return self.end(byte),
        };
        Ok(())
    }
    fn required_digit(
        &mut self,
        byte: u8,
        next: State,
        context: &'static str,
    ) -> Result<(), Error> {
        if !byte.is_ascii_digit() {
            return Err(error(byte, context));
        }
        self.state = next;
        Ok(())
    }
}
fn literal_context(word: &[u8], at: usize) -> &'static str {
    match (word, at) {
        (b"true", 1) => "in literal true (expecting 'r')",
        (b"true", 2) => "in literal true (expecting 'u')",
        (b"true", 3) => "in literal true (expecting 'e')",
        (b"false", 1) => "in literal false (expecting 'a')",
        (b"false", 2) => "in literal false (expecting 'l')",
        (b"false", 3) => "in literal false (expecting 's')",
        (b"false", 4) => "in literal false (expecting 'e')",
        (b"null", 1) => "in literal null (expecting 'u')",
        (b"null", 2 | 3) => "in literal null (expecting 'l')",
        _ => unreachable!("fixed literal scanner position"),
    }
}
