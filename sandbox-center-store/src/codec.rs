use crate::{ControlState, StoreError};
use std::io::{self, Write};

struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for BoundedWriter {
    fn write(&mut self, value: &[u8]) -> io::Result<usize> {
        if value.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("center block capacity exhausted"));
        }
        self.bytes.extend_from_slice(value);
        Ok(value.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn encode<T: ControlState>(state: &T, limit: usize) -> Result<Vec<u8>, StoreError> {
    state.check_supported_profiles()?;
    if !state.is_valid() {
        return Err(StoreError::InvalidState);
    }
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut writer, state).map_err(|error| {
        if error.is_io() {
            StoreError::Capacity
        } else {
            StoreError::InvalidState
        }
    })?;
    // Validate what the codec actually produced, not merely the input object.
    decode::<T>(&writer.bytes, limit)?;
    Ok(writer.bytes)
}

pub(crate) fn decode<T: ControlState>(value: &[u8], limit: usize) -> Result<T, StoreError> {
    if value.is_empty() || value.len() > limit {
        return Err(StoreError::InvalidRecord);
    }
    let state: T = serde_json::from_slice(value).map_err(|_| StoreError::InvalidRecord)?;
    state.check_supported_profiles()?;
    if !state.is_valid() {
        return Err(StoreError::InvalidState);
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Sample {
        counter: u64,
        text: String,
    }
    impl ControlState for Sample {
        const FORMAT_VERSION: u32 = 1;
        fn is_valid(&self) -> bool {
            self.counter > 0
        }
    }

    #[test]
    fn exact_byte_boundary_and_invalid_state_never_encode_partial_records() {
        let value = Sample {
            counter: 1,
            text: "fixture".into(),
        };
        let encoded = serde_json::to_vec(&value).unwrap();
        assert_eq!(encode(&value, encoded.len()).unwrap(), encoded);
        assert_eq!(encode(&value, encoded.len() - 1), Err(StoreError::Capacity));
        assert_eq!(
            encode(
                &Sample {
                    counter: 0,
                    text: String::new()
                },
                1024
            ),
            Err(StoreError::InvalidState)
        );
    }

    #[test]
    fn corrupt_duplicate_unknown_fields_and_semantic_corruption_fail_closed() {
        for value in [
            b"".as_slice(),
            b"{",
            b"null",
            b"[]",
            br#"{"counter":1,"counter":2,"text":"fixture"}"#,
            br#"{"counter":1,"text":"fixture","unknown":true}"#,
            br#"{"counter":true,"text":"fixture"}"#,
        ] {
            assert_eq!(
                decode::<Sample>(value, 1024),
                Err(StoreError::InvalidRecord)
            );
        }
        assert_eq!(
            decode::<Sample>(br#"{"counter":0,"text":"fixture"}"#, 1024),
            Err(StoreError::InvalidState)
        );
        assert_eq!(
            decode::<Sample>(br#"{"counter":1,"text":"fixture"}"#, 2),
            Err(StoreError::InvalidRecord)
        );
    }
}
