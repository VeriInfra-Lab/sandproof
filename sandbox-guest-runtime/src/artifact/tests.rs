use super::*;
use std::io::Write;

fn expected(data: &[u8]) -> Expected {
    Fingerprint {
        bytes: data.len() as u64,
        sha256: format!("{:x}", Sha256::digest(data)),
    }
    .validate()
    .unwrap()
}
fn file(data: &[u8]) -> File {
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(data).unwrap();
    // Deliberately leave the position at EOF; observation must start at zero.
    file
}

#[test]
fn exact_known_digest_large_input_and_same_open_inode() {
    let known = Fingerprint {
        bytes: 3,
        sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
    }
    .validate()
    .unwrap();
    assert_eq!(known.verify(file(b"abc"), || true), Ok(()));
    let bytes: Vec<_> = (0..20_000)
        .map(|n| u8::try_from(n % 251).unwrap())
        .collect();
    let mut checkpoints = 0;
    assert_eq!(
        expected(&bytes).verify(file(&bytes), || {
            checkpoints += 1;
            true
        }),
        Ok(())
    );
    assert!(checkpoints >= 6); // three bounded reads, EOF and outer checks
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("worker");
    std::fs::write(&path, b"abc").unwrap();
    let original = File::open(&path).unwrap();
    std::fs::rename(&path, directory.path().join("original")).unwrap();
    std::fs::write(&path, b"bad").unwrap();
    assert_eq!(known.verify(original, || true), Ok(()));
    assert_eq!(
        known.verify(File::open(path).unwrap(), || true),
        Err(Error::Mismatch)
    );
}

#[test]
fn schema_size_digest_and_type_refuse_without_execution() {
    for bytes in [0, u64::MAX] {
        assert!(matches!(
            Fingerprint {
                bytes,
                sha256: "0".repeat(64)
            }
            .validate(),
            Err(Error::Configuration)
        ));
    }
    for sha256 in [
        String::new(),
        "0".repeat(63),
        "0".repeat(65),
        "A".repeat(64),
        "g".repeat(64),
        "é".repeat(32),
    ] {
        assert!(matches!(
            Fingerprint { bytes: 1, sha256 }.validate(),
            Err(Error::Configuration)
        ));
    }
    for data in [&b""[..], b"ab", b"abcd", b"bad"] {
        assert_eq!(
            expected(b"abc").verify(file(data), || true),
            Err(Error::Mismatch)
        );
    }
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        expected(b"abc").verify(File::open(directory.path()).unwrap(), || true),
        Err(Error::Input)
    );
}

#[test]
fn cancellation_and_growth_or_truncation_between_observations_refuse() {
    for stop_at in 1..=6 {
        let data = vec![b'x'; 20_000];
        let mut checkpoints = 0;
        assert_eq!(
            expected(&data).verify(file(&data), || {
                checkpoints += 1;
                checkpoints < stop_at
            }),
            Err(Error::Cancelled)
        );
        assert_eq!(checkpoints, stop_at);
    }
    for replacement in [&b"ab"[..], b"abcd", b"bad"] {
        let original = file(b"abc");
        let mut writer = original.try_clone().unwrap();
        let mut calls = 0;
        assert_eq!(
            expected(b"abc").verify(original, || {
                calls += 1;
                if calls == 2 {
                    writer.seek(SeekFrom::Start(0)).unwrap();
                    writer.write_all(replacement).unwrap();
                    writer.set_len(replacement.len() as u64).unwrap();
                    writer.seek(SeekFrom::Start(0)).unwrap();
                }
                true
            }),
            Err(Error::Mismatch)
        );
    }
}
