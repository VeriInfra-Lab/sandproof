//! Command schemas over the shared fixed protobuf JSON tokenizer. Known message
//! nesting is acyclic and at most four calls deep; unknown values are iterative.
use super::{
    Decoder, Failure, JsonError, JsonField as F, JsonOneof, JsonReason as R, Kind, Token, finish,
};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
enum Schema {
    Start,
    Config,
    SelectRequest,
    Selector,
    InputRequest,
    Input,
    Signal,
    Update,
    Pty,
    Size,
    List,
}
#[derive(Clone, Copy)]
enum Value {
    Scalar(F),
    Message(Schema),
    List(F),
    Map,
}
struct Field {
    bit: u8,
    value: Value,
    oneof: Option<JsonOneof>,
}

pub(crate) fn observe(method: &str, body: &[u8]) -> Option<JsonError> {
    let (method, schema) = match method {
        "Start" => ("Start", Schema::Start),
        "Connect" => ("Connect", Schema::SelectRequest),
        "List" => ("List", Schema::List),
        "SendInput" => ("SendInput", Schema::InputRequest),
        "CloseStdin" => ("CloseStdin", Schema::SelectRequest),
        "SendSignal" => ("SendSignal", Schema::Signal),
        "Update" => ("Update", Schema::Update),
        _ => return None,
    };
    let error = if body.is_empty() {
        Failure::at(R::Empty, 0)
    } else {
        let mut decoder = Decoder::new(body);
        let result = message(&mut decoder, schema, body).and_then(|()| {
            let last = decoder.read()?;
            if last.kind == Kind::Eof {
                Ok(())
            } else {
                Err(Failure::token(R::UnexpectedToken, &last))
            }
        });
        result.err()?
    };
    Some(finish(method, body, error))
}

fn field(schema: Schema, name: &str) -> Option<Field> {
    use Schema as S;
    use Value::{List, Map, Message, Scalar};
    let (bit, value) = match (schema, name) {
        (S::Start, "process") => (1, Message(S::Config)),
        (S::Start | S::Selector, "tag") => (2, Scalar(F::Tag)),
        (S::Start, "stdin") => (4, Scalar(F::Stdin)),
        (S::Start | S::Update, "pty") => (8, Message(S::Pty)),
        (S::Config, "cmd") => (1, Scalar(F::Cmd)),
        (S::Config, "args") => (2, List(F::Args)),
        (S::Config, "envs") => (4, Map),
        (S::Config, "cwd") => (8, Scalar(F::Cwd)),
        (S::SelectRequest | S::InputRequest | S::Signal | S::Update, "process") => {
            (1, Message(S::Selector))
        }
        (S::Selector, "pid") => (1, Scalar(F::Pid)),
        (S::InputRequest, "input") => (2, Message(S::Input)),
        (S::Input, "stdin") => (1, Scalar(F::StdinBytes)),
        (S::Input, "pty") => (2, Scalar(F::PtyBytes)),
        (S::Signal, "signal") => (2, Scalar(F::Signal)),
        (S::Pty, "size") => (1, Message(S::Size)),
        (S::Size, "cols") => (1, Scalar(F::Cols)),
        (S::Size, "rows") => (2, Scalar(F::Rows)),
        _ => return None,
    };
    let oneof = match schema {
        S::Selector => Some(JsonOneof::Selector),
        S::Input => Some(JsonOneof::Input),
        _ => None,
    };
    Some(Field { bit, value, oneof })
}

fn message(decoder: &mut Decoder<'_>, schema: Schema, body: &[u8]) -> Result<(), Failure> {
    let first = decoder.read()?;
    if first.kind != Kind::ObjectOpen {
        return Err(Failure::token(R::UnexpectedToken, &first));
    }
    let mut seen = 0;
    let mut selected = false;
    loop {
        let key = decoder.read()?;
        if key.kind == Kind::ObjectClose {
            return Ok(());
        }
        if key.kind != Kind::Name {
            return Err(Failure::token(R::UnexpectedToken, &key));
        }
        let name = string(&key, body)?;
        let Some(field) = field(schema, &name) else {
            skip(decoder)?;
            continue;
        };
        if seen & field.bit != 0 {
            return Err(Failure::token(R::Duplicate, &key));
        }
        seen |= field.bit;
        // Go ignores the peek error while testing null. A previously selected
        // oneof can therefore precede a lexical error in the next value.
        if decoder.peek().is_ok_and(|token| token.kind == Kind::Null) {
            decoder.read()?;
            continue;
        }
        if let Some(oneof) = field.oneof {
            if selected {
                return Err(Failure::token(R::Oneof(oneof), &key));
            }
            selected = true;
        }
        match field.value {
            Value::Scalar(field) => scalar(decoder, field, body)?,
            Value::Message(schema) => message(decoder, schema, body)?,
            Value::List(field) => list(decoder, field, body)?,
            Value::Map => map(decoder, body)?,
        }
    }
}

fn string(token: &Token, body: &[u8]) -> Result<String, Failure> {
    serde_json::from_slice(&body[token.range.clone()])
        .map_err(|_| Failure::token(R::InvalidValue, token))
}
fn scalar(decoder: &mut Decoder<'_>, field: F, body: &[u8]) -> Result<(), Failure> {
    let token = decoder.read()?;
    let valid = match field {
        F::Stdin => token.kind == Kind::Bool,
        F::Pid | F::Cols | F::Rows => super::valid_scalar(F::Depth, &token, body),
        F::Signal => {
            token.kind == Kind::String
                || (token.kind == Kind::Number && signed(&body[token.range.clone()]))
        }
        F::StdinBytes | F::PtyBytes => {
            token.kind == Kind::String
                && crate::protocol::request_json::decode_bytes(&string(&token, body)?).is_ok()
        }
        _ => token.kind == Kind::String,
    };
    if valid {
        Ok(())
    } else {
        Err(Failure::token(R::InvalidField(field), &token))
    }
}
fn signed(raw: &[u8]) -> bool {
    let Ok(raw) = std::str::from_utf8(raw) else {
        return false;
    };
    let raw = raw.strip_suffix(['e', 'E']).unwrap_or(raw);
    let negative = raw.starts_with('-');
    let Some(magnitude) =
        crate::file_protocol::request_json::exact_uint32(raw.strip_prefix('-').unwrap_or(raw))
    else {
        return false;
    };
    let value = if negative {
        -i64::from(magnitude)
    } else {
        i64::from(magnitude)
    };
    i32::try_from(value).is_ok()
}
fn list(decoder: &mut Decoder<'_>, field: F, body: &[u8]) -> Result<(), Failure> {
    let first = decoder.read()?;
    if first.kind != Kind::ArrayOpen {
        return Err(Failure::token(R::UnexpectedToken, &first));
    }
    loop {
        if decoder.peek()?.kind == Kind::ArrayClose {
            decoder.read()?;
            return Ok(());
        }
        scalar(decoder, field, body)?;
    }
}
fn map(decoder: &mut Decoder<'_>, body: &[u8]) -> Result<(), Failure> {
    let first = decoder.read()?;
    if first.kind != Kind::ObjectOpen {
        return Err(Failure::token(R::UnexpectedToken, &first));
    }
    let mut keys = BTreeSet::new();
    loop {
        let key = decoder.read()?;
        if key.kind == Kind::ObjectClose {
            return Ok(());
        }
        if key.kind != Kind::Name {
            return Err(Failure::token(R::UnexpectedToken, &key));
        }
        if !keys.insert(string(&key, body)?) {
            return Err(Failure::token(R::DuplicateMap, &key));
        }
        scalar(decoder, F::MapValue, body)?;
    }
}
fn skip(decoder: &mut Decoder<'_>) -> Result<(), Failure> {
    let mut open = 0_usize;
    loop {
        let value = decoder.read()?;
        match value.kind {
            Kind::ObjectOpen | Kind::ArrayOpen => open += 1,
            Kind::ObjectClose | Kind::ArrayClose => open = open.saturating_sub(1),
            _ => (),
        }
        if open == 0 {
            return Ok(());
        }
    }
}
