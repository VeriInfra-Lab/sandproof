//! Go multipart.NewWriter uses 30 random bytes formatted as lowercase hex.
//! Run only at the multipart decision inside the admitted blocking worker.
use super::Error;
const HEX: &[u8; 16] = b"0123456789abcdef";

pub(super) fn generate() -> Result<String, Error> {
    generate_with(|bytes| getrandom::fill(bytes).map_err(|_| Error::EntropyUnavailable))
}

fn generate_with(fill: impl FnOnce(&mut [u8; 30]) -> Result<(), Error>) -> Result<String, Error> {
    let mut bytes = [0; 30];
    fill(&mut bytes)?;
    let mut boundary = String::with_capacity(60);
    for byte in bytes {
        boundary.push(char::from(HEX[usize::from(byte >> 4)]));
        boundary.push(char::from(HEX[usize::from(byte & 15)]));
    }
    Ok(boundary)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_random_bytes_keep_fixed_hex_format_and_entropy_failure_has_no_fallback() {
        let boundary = generate_with(|bytes| {
            for (index, byte) in bytes.iter_mut().enumerate() {
                *byte = u8::try_from(index).unwrap();
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(
            boundary,
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d"
        );
        assert_eq!(
            generate_with(|_| Err(Error::EntropyUnavailable)),
            Err(Error::EntropyUnavailable)
        );
        let actual = generate().unwrap();
        assert_eq!(actual.len(), 60);
        assert!(
            actual
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
    }
}
