#[cfg(test)]
mod tls_certificate_binding_tests {
    use super::*;

    // The runner inserts the frozen existing rcgen fixture here, with its
    // crate import rebound to crate::. No production statement is changed.
    // @CERTIFICATE_FIXTURE@

    fn wrappers(
        server_pins: Vec<CertificatePin>,
        client_pins: Vec<CertificatePin>,
    ) -> (ServerTls, ClientTls) {
        let material = fixtures::certificates();
        let server = ServerTls::new(
            material.host.material(),
            vec![material.ca.der().clone()],
            server_pins,
        )
        .unwrap();
        let client = ClientTls::new(
            material.center.material(),
            vec![material.ca.der().clone()],
            client_pins,
        )
        .unwrap();
        (server, client)
    }

    fn pin_from_hex(text: &str) -> CertificatePin {
        assert_eq!(text.len(), 64);
        CertificatePin(std::array::from_fn(|index| {
            u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).unwrap()
        }))
    }

    #[test]
    fn known_sha256_vectors_use_the_entire_original_input() {
        for (bytes, expected) in [
            (
                b"".as_slice(),
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                b"abc".as_slice(),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
        ] {
            let der = CertificateDer::from(bytes);
            assert_eq!(CertificatePin::from_certificate(&der), pin_from_hex(expected));
        }
    }

    #[test]
    fn owned_and_borrowed_der_keep_all_bytes() {
        for bytes in [Vec::new(), b"abc".to_vec(), vec![0, 128, 255], vec![7; 513]] {
            let borrowed = CertificateDer::from(bytes.as_slice());
            let owned = CertificateDer::from(bytes.clone());
            let expected = CertificatePin(Sha256::digest(bytes.as_slice()).into());
            assert_eq!(borrowed.as_ref(), bytes.as_slice());
            assert_eq!(owned.as_ref(), bytes.as_slice());
            assert_eq!(CertificatePin::from_certificate(&borrowed), expected);
            assert_eq!(CertificatePin::from_certificate(&owned), expected);
        }
    }

    #[test]
    fn none_and_some_empty_are_rejected_by_each_original_entry() {
        let pin = CertificatePin::from_certificate(&CertificateDer::from(b"peer".as_slice()));
        let (server, client) = wrappers(vec![pin], vec![pin]);
        let empty: [CertificateDer<'_>; 0] = [];
        for input in [None, Some(empty.as_slice())] {
            assert_eq!(verify_pin(input, &[pin]), Err(TransportError::Authentication));
            assert_eq!(server.verify_peer(input), Err(TransportError::Authentication));
            assert_eq!(client.verify_peer(input), Err(TransportError::Authentication));
            assert_eq!(server.peer_pin(input), Err(TransportError::Authentication));
        }
    }

    #[test]
    fn a_registered_tail_cannot_authorize_the_original_first_leaf() {
        let chain = [
            CertificateDer::from(b"first".as_slice()),
            CertificateDer::from(b"tail".as_slice()),
        ];
        let tail_pin = CertificatePin::from_certificate(&chain[1]);
        assert_eq!(
            verify_pin(Some(&chain), &[tail_pin]),
            Err(TransportError::Authentication),
            "a registered tail must not authorize the original first leaf"
        );
        let (server, client) = wrappers(vec![tail_pin], vec![tail_pin]);
        assert_eq!(server.verify_peer(Some(&chain)), Err(TransportError::Authentication));
        assert_eq!(client.verify_peer(Some(&chain)), Err(TransportError::Authentication));
        assert_eq!(server.peer_pin(Some(&chain)), Err(TransportError::Authentication));
    }

    #[test]
    fn peer_pin_returns_the_authorized_original_first_leaf() {
        let chain = [
            CertificateDer::from(b"first".as_slice()),
            CertificateDer::from(b"tail".as_slice()),
        ];
        let first = CertificatePin::from_certificate(&chain[0]);
        let (server, _) = wrappers(vec![first], vec![first]);
        assert_eq!(server.verify_peer(Some(&chain)), Ok(()));
        assert_eq!(
            server.peer_pin(Some(&chain)),
            Ok(first),
            "peer_pin must return the authorized original first leaf"
        );
    }

    #[test]
    fn each_tls_object_uses_its_own_pin_field() {
        let a = [CertificateDer::from(b"a".as_slice())];
        let b = [CertificateDer::from(b"b".as_slice())];
        let pa = CertificatePin::from_certificate(&a[0]);
        let pb = CertificatePin::from_certificate(&b[0]);
        let (server, client) = wrappers(vec![pa], vec![pb]);
        assert_eq!(server.verify_peer(Some(&a)), Ok(()));
        assert_eq!(client.verify_peer(Some(&a)), Err(TransportError::Authentication));
        assert_eq!(server.verify_peer(Some(&b)), Err(TransportError::Authentication));
        assert_eq!(client.verify_peer(Some(&b)), Ok(()));
    }

    #[test]
    fn every_pin_byte_and_registered_position_is_preserved() {
        let chain = [CertificateDer::from(vec![0, 128, 255])];
        let expected = CertificatePin::from_certificate(&chain[0]);
        let mut wrong = Vec::new();
        for index in 0..32 {
            let mut changed = expected;
            changed.0[index] ^= 1;
            assert_eq!(verify_pin(Some(&chain), &[changed]), Err(TransportError::Authentication));
            wrong.push(changed);
        }
        assert_eq!(verify_pin(Some(&chain), &wrong), Err(TransportError::Authentication));
        for index in [0, 16, 32] {
            let mut pins = wrong.clone();
            pins.insert(index, expected);
            assert_eq!(verify_pin(Some(&chain), &pins), Ok(()));
        }
        assert_eq!(verify_pin(Some(&chain), &[]), Err(TransportError::Authentication));
    }
}
