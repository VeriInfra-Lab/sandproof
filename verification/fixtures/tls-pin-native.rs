#[cfg(test)]
mod tls_pin_candidate_tests {
    use super::*;

    // Finite regression oracle, copied from the frozen original expression.
    // This is not a general equivalence proof.
    fn original_guard(
        certificates: Option<&[CertificateDer<'_>]>,
        pins: &[CertificatePin],
    ) -> Result<(), TransportError> {
        if certificates
            .and_then(|chain| chain.first())
            .is_some_and(|leaf| pins.contains(&CertificatePin::from_certificate(leaf)))
        {
            Ok(())
        } else {
            Err(TransportError::Authentication)
        }
    }

    #[test]
    fn complete_snapshot_keeps_none_empty_order_and_all_bytes() {
        assert_eq!(certificate_chain_bytes(None), None);
        let empty: [CertificateDer<'_>; 0] = [];
        assert_eq!(certificate_chain_bytes(Some(&empty)), Some(Vec::new()));
        let values: Vec<Vec<u8>> = vec![vec![], vec![0, 128, 255], vec![7; 513]];
        let certificates: Vec<CertificateDer<'_>> = values
            .iter()
            .map(|value| CertificateDer::from(value.as_slice()))
            .collect();
        assert_eq!(
            certificate_chain_bytes(Some(&certificates)),
            Some(values.clone())
        );
        for (certificate, original) in certificates.iter().zip(&values) {
            assert_eq!(certificate.as_ref(), original.as_slice());
        }
    }

    #[test]
    fn only_the_original_first_leaf_can_authorize() {
        let first = CertificateDer::from(vec![0, 128, 255]);
        let second = CertificateDer::from(vec![1, 128, 255]);
        let first_pin = CertificatePin::from_certificate(&first);
        let second_pin = CertificatePin::from_certificate(&second);
        assert_ne!(first_pin, second_pin);
        let certificates = [first, second];
        assert_eq!(
            verify_pin(Some(&certificates), &[second_pin]),
            Err(TransportError::Authentication),
            "a registered tail certificate cannot authorize an unregistered first leaf"
        );
        assert_eq!(verify_pin(Some(&certificates), &[first_pin]), Ok(()));
        assert_eq!(
            verify_pin(Some(&certificates), &[second_pin, first_pin]),
            Ok(())
        );
    }

    #[test]
    fn every_pin_byte_and_every_registered_position_matters() {
        let certificate = CertificateDer::from(vec![3, 0, 255]);
        let expected = CertificatePin::from_certificate(&certificate);
        let certificates = [certificate];
        let mut mismatches = Vec::new();
        for index in 0..32 {
            let mut changed = expected;
            changed.0[index] ^= 1;
            assert_eq!(
                verify_pin(Some(&certificates), &[changed]),
                Err(TransportError::Authentication),
                "changing pin byte {index} must reject"
            );
            mismatches.push(changed);
        }
        assert_eq!(
            verify_pin(Some(&certificates), &mismatches),
            Err(TransportError::Authentication)
        );
        for index in [0, 16, 32] {
            let mut pins = mismatches.clone();
            pins.insert(index, expected);
            assert_eq!(verify_pin(Some(&certificates), &pins), Ok(()));
        }
    }

    #[test]
    fn candidate_matches_frozen_original_over_complete_finite_inputs() {
        let a = CertificateDer::from(vec![0, 128, 255]);
        let z = CertificateDer::from(vec![1, 128, 255]);
        let pa = CertificatePin::from_certificate(&a);
        let pz = CertificatePin::from_certificate(&z);
        let choices = [
            vec![],
            vec![a.clone()],
            vec![z.clone()],
            vec![a.clone(), z.clone()],
            vec![z.clone(), a.clone()],
            vec![a.clone(), z.clone(), a.clone()],
        ];
        let registrations = [
            vec![],
            vec![pa],
            vec![pz],
            vec![pa, pz],
            vec![pz, pa],
            vec![pa, pa],
        ];
        for pins in &registrations {
            assert_eq!(verify_pin(None, pins), original_guard(None, pins));
            for chain in &choices {
                assert_eq!(
                    verify_pin(Some(chain), pins),
                    original_guard(Some(chain), pins)
                );
            }
        }
    }

    #[test]
    fn leaf_digest_return_uses_the_same_full_first_input() {
        let chain = [vec![0, 128, 255], vec![1, 128, 255]];
        let original = CertificateDer::from(chain[0].as_slice());
        let expected = CertificatePin::from_certificate(&original);
        let empty: [Vec<u8>; 0] = [];
        assert_eq!(
            leaf_pin_bytes(None, &Sha256CertificateDigest),
            Err(TransportError::Authentication)
        );
        assert_eq!(
            leaf_pin_bytes(Some(&empty), &Sha256CertificateDigest),
            Err(TransportError::Authentication)
        );
        assert_eq!(
            leaf_pin_bytes(Some(&chain), &Sha256CertificateDigest),
            Ok(expected)
        );
        assert_eq!(
            verify_pin_bytes(Some(&chain), &[expected], &Sha256CertificateDigest),
            Ok(())
        );
    }
}
