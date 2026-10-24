use sandproof_protocol::lifecycle::ContentDigest;

#[test]
fn full_digest_bytes_survive_construction_and_projection() {
    for bytes in [
        [0; 32],
        [u8::MAX; 32],
        std::array::from_fn(|i| u8::try_from(i).unwrap()),
    ] {
        let digest = ContentDigest::from_sha256(bytes);
        assert_eq!(digest.as_bytes(), &bytes);
        assert_eq!(digest.into_bytes(), bytes);
    }
}

#[test]
fn differences_outside_a_numeric_prefix_remain_observable() {
    let left = [0; 32];
    let mut right = left;
    right[31] = 1;
    assert_ne!(
        ContentDigest::from_sha256(left),
        ContentDigest::from_sha256(right)
    );
}

#[test]
fn json_roundtrip_preserves_all_digest_bytes() {
    let digest =
        ContentDigest::from_sha256(std::array::from_fn(|i| 255 - u8::try_from(i).unwrap()));
    let encoded = serde_json::to_vec(&digest).unwrap();
    assert_eq!(
        serde_json::from_slice::<ContentDigest>(&encoded).unwrap(),
        digest
    );
    assert_eq!(
        serde_json::from_slice::<Vec<u8>>(&encoded).unwrap(),
        digest.as_bytes()
    );
}

#[test]
fn json_rejects_wrong_width_and_non_byte_representations() {
    for count in [0, 8, 31, 33] {
        let encoded = serde_json::to_vec(&vec![0u8; count]).unwrap();
        assert!(serde_json::from_slice::<ContentDigest>(&encoded).is_err());
    }
    for encoded in [
        b"16".as_slice(),
        b"null",
        b"\"0123456789abcdef\"",
        b"{\"bytes\":[]}",
    ] {
        assert!(serde_json::from_slice::<ContentDigest>(encoded).is_err());
    }
    let mut values = vec![0u16; 32];
    values[31] = 256;
    let encoded = serde_json::to_vec(&values).unwrap();
    assert!(serde_json::from_slice::<ContentDigest>(&encoded).is_err());
}
