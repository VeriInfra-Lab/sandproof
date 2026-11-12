//! Buffered decoding of newly opened node record files. Callers retain path,
//! ownership, size, identity and durability checks; this is not a cache.
use serde::de::DeserializeOwned;
use std::io::{BufReader, Read};

pub(crate) fn from_reader<R: Read, T: DeserializeOwned>(reader: R) -> serde_json::Result<T> {
    // serde_json's IoRead consumes bytes individually. Buffer the file reads,
    // not whole records, while preserving its complete-document/EOF checks.
    serde_json::from_reader(BufReader::with_capacity(8 * 1024, reader))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        io::{self, Write},
    };

    struct Counted<'a, R> {
        inner: R,
        calls: &'a Cell<usize>,
        largest: &'a Cell<usize>,
    }
    impl<R: Read> Read for Counted<'_, R> {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.calls.set(self.calls.get() + 1);
            self.largest.set(self.largest.get().max(bytes.len()));
            self.inner.read(bytes)
        }
    }

    #[test]
    fn real_file_decodes_with_bounded_batch_reads_and_rejects_trailing_data() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let expected = serde_json::json!({"value": "x".repeat(64 * 1024)});
        file.write_all(&serde_json::to_vec(&expected).unwrap())
            .unwrap();
        let calls = Cell::new(0);
        let largest = Cell::new(0);
        let actual: serde_json::Value = from_reader(Counted {
            inner: file.reopen().unwrap(),
            calls: &calls,
            largest: &largest,
        })
        .unwrap();
        assert_eq!(actual, expected);
        assert!(
            calls.get() <= 12,
            "byte-at-a-time file reads: {}",
            calls.get()
        );
        assert!(largest.get() <= 8 * 1024);
        file.write_all(b" {}").unwrap();
        assert!(from_reader::<_, serde_json::Value>(file.reopen().unwrap()).is_err());
    }

    #[test]
    fn malformed_and_failed_reads_remain_errors() {
        struct Failed;
        impl Read for Failed {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::PermissionDenied.into())
            }
        }
        for input in [b"".as_slice(), b"{", b"{\"x\":}", b"{}false"] {
            assert!(from_reader::<_, serde_json::Value>(input).is_err());
        }
        let error = from_reader::<_, serde_json::Value>(Failed).unwrap_err();
        assert_eq!(error.io_error_kind(), Some(io::ErrorKind::PermissionDenied));
    }
}
