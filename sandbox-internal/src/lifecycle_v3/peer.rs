//! Protected registration for the original TLS-verified peer value.
use crate::{CertificatePin, TransportError};
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::lifecycle::{AuthenticatedCenterContext, CenterIdentity};

#[derive(Clone)]
pub struct CenterPeerRegistration {
    center: CenterIdentity,
    expected_node: NodeIdentity,
    pins: Vec<CertificatePin>,
}
impl CenterPeerRegistration {
    /// Both rotation pins identify the same protected logical center.
    /// # Errors
    /// Refuses a foreign deployment, absent/duplicate pins or more than two pins.
    pub fn new(
        center: CenterIdentity,
        expected_node: NodeIdentity,
        pins: Vec<CertificatePin>,
    ) -> Result<Self, TransportError> {
        if center.deployment != expected_node.deployment_id
            || !(1..=2).contains(&pins.len())
            || (pins.len() == 2 && pins[0] == pins[1])
        {
            return Err(TransportError::Configuration);
        }
        Ok(Self {
            center,
            expected_node,
            pins,
        })
    }

    /// `peer` must be the original value delivered by `ProtectedHttp::respond`
    /// after TLS checks, never a parsed body/header value. This is an explicit
    /// trusted binding seam, not a TLS implementation or unforgeability proof.
    /// # Errors
    /// An unregistered peer cannot obtain the configured logical binding.
    pub fn context_for_verified_peer(
        &self,
        peer: CertificatePin,
    ) -> Result<AuthenticatedCenterContext, TransportError> {
        if !self.pins.contains(&peer) {
            return Err(TransportError::Authentication);
        }
        Ok(AuthenticatedCenterContext::from_verified_peer_registration(
            self.center.clone(),
            self.expected_node.clone(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::CertificateDer;
    fn pin(byte: u8) -> CertificatePin {
        // Distinct digest fixtures for post-TLS binding only; no handshake is
        // performed or claimed by these tests of protected registration.
        CertificatePin::from_certificate(&CertificateDer::from(vec![byte]))
    }
    #[test]
    fn rotation_maps_two_verified_peer_values_to_the_same_logical_binding() {
        let node = NodeIdentity {
            deployment_id: "deployment".try_into().unwrap(),
            node_id: "host".try_into().unwrap(),
            node_generation: 1.try_into().unwrap(),
        };
        let center = CenterIdentity {
            deployment: node.deployment_id.clone(),
            center: "center".try_into().unwrap(),
        };
        let registration = CenterPeerRegistration::new(center, node, vec![pin(1), pin(2)]).unwrap();
        assert_eq!(
            registration.context_for_verified_peer(pin(1)).unwrap(),
            registration.context_for_verified_peer(pin(2)).unwrap()
        );
        assert_eq!(
            registration.context_for_verified_peer(pin(3)),
            Err(TransportError::Authentication)
        );
    }
}
