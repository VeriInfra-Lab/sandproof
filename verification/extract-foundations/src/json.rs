#![forbid(unsafe_code)]

//! Direct roots for the production JSON counter leaf, not a copied validator.
use sandproof_protocol::internal::PositiveCounter;
use serde::{Deserialize, Deserializer};

pub fn counter_decode<'de, D: Deserializer<'de>>(input: D) -> Result<PositiveCounter, D::Error> {
    PositiveCounter::deserialize(input)
}

pub fn counter_primitive<'de, D: Deserializer<'de>>(input: D) -> Result<u64, D::Error> {
    u64::deserialize(input)
}

// Keeps the real engine error type explicit for the reviewed library view.
pub fn json_error_passthrough(error: serde_json::Error) -> serde_json::Error {
    error
}

#[cfg(test)]
mod tests {
    use sandproof_protocol::internal::PositiveCounter;

    #[test]
    fn successful_decode_is_positive() {
        for input in ["0", "1", "18446744073709551615"] {
            if let Ok(value) = serde_json::from_str::<PositiveCounter>(input) {
                assert_ne!(value.get(), 0, "successful decode returned zero: {input}");
            }
        }
    }

    #[test]
    fn nonzero_and_original_error_order() {
        for (input, expected) in [("1", 1), ("18446744073709551615", u64::MAX)] {
            assert_eq!(
                serde_json::from_str::<PositiveCounter>(input)
                    .unwrap()
                    .get(),
                expected
            );
        }
        let zero = serde_json::from_slice::<PositiveCounter>(b"0 x").unwrap_err();
        assert_eq!(zero.to_string(), "invalid or exhausted internal counter");
        assert_eq!(zero.classify(), serde_json::error::Category::Data);
        assert_eq!((zero.line(), zero.column()), (0, 0));
        let nonzero = serde_json::from_slice::<PositiveCounter>(b"1 x").unwrap_err();
        assert_eq!(
            nonzero.to_string(),
            "trailing characters at line 1 column 3"
        );
        assert_eq!(nonzero.classify(), serde_json::error::Category::Syntax);
        assert_eq!((nonzero.line(), nonzero.column()), (1, 3));
    }
}
