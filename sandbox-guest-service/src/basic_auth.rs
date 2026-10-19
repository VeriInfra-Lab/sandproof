//! Bounded username selection, before any account lookup. Password bytes are
//! temporary and zeroized; malformed Basic selects the initialized default.
use axum::http::HeaderMap;
use base64::{
    Engine, alphabet,
    engine::{GeneralPurpose, GeneralPurposeConfig},
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Unauthenticated,
    ResourceExhausted,
}

pub(crate) fn explicit_user(
    headers: &HeaderMap,
    username_bytes: usize,
    authorization_bytes: usize,
) -> Result<Option<String>, Error> {
    let mut values = headers.get_all("authorization").iter();
    let value = values.next();
    // Intentional protected component policy, unlike Go Header.Get's first.
    if values.next().is_some() {
        return Err(Error::Unauthenticated);
    }
    let Some(value) = value else { return Ok(None) };
    if value.as_bytes().len() > authorization_bytes {
        return Err(Error::ResourceExhausted);
    }
    let Some(user) = parse(value.as_bytes()) else {
        return Ok(None);
    };
    if user.len() > username_bytes {
        return Err(Error::ResourceExhausted);
    }
    // The account boundary accepts UTF-8 names, unlike Go's arbitrary string.
    Ok(Some(
        std::str::from_utf8(&user)
            .map_err(|_| Error::Unauthenticated)?
            .into(),
    ))
}

fn parse(value: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    if !value.get(..6)?.eq_ignore_ascii_case(b"Basic ") {
        return None;
    }
    // Go StdEncoding ignores CR/LF and does not require zero trailing bits.
    // HTTP HeaderValue rejects CR/LF; the raw parser still follows BasicAuth.
    let encoded = Zeroizing::new(
        value[6..]
            .iter()
            .copied()
            .filter(|b| !matches!(b, b'\r' | b'\n'))
            .collect::<Vec<_>>(),
    );
    let encoding = GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
    );
    let mut decoded = Zeroizing::new(Vec::new());
    encoding.decode_vec(&*encoded, &mut decoded).ok()?;
    let colon = decoded.iter().position(|b| *b == b':')?;
    // Clear the password before shortening, since Vec::truncate does not wipe.
    decoded[colon..].zeroize();
    decoded.truncate(colon);
    Some(decoded)
}

#[cfg(test)]
mod tests;
