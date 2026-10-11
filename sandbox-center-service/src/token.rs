//! One deployment-fixed public token key; no per-sandbox credential cache.
//! Changing an intent (renewal) does not invalidate an existing SDK connection.
use ring::hmac;
use sandproof_api::PortError;
use sandproof_protocol::internal::ControlIdentity;

pub(crate) struct Tokens(hmac::Key);
impl Tokens {
    pub(crate) fn new(key: &[u8]) -> Result<Self, PortError> {
        if key.len() < 32 || key.len() > 4096 {
            return Err(PortError::Invalid);
        }
        Ok(Self(hmac::Key::new(hmac::HMAC_SHA256, key)))
    }
    fn message(identity: &ControlIdentity) -> Result<Vec<u8>, PortError> {
        let mut message = b"sandproof/center/data-token/v1\0".to_vec();
        message.extend(
            serde_json::to_vec(&(
                identity.node.deployment_id.as_str(),
                identity.node.node_id.as_str(),
                identity.node.node_generation.get(),
                identity.tenant_id.as_str(),
                identity.sandbox_id.as_str(),
            ))
            .map_err(|_| PortError::Busy)?,
        );
        Ok(message)
    }
    pub(crate) fn issue(&self, identity: &ControlIdentity) -> Result<String, PortError> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let tag = hmac::sign(&self.0, &Self::message(identity)?);
        Ok(tag
            .as_ref()
            .iter()
            .flat_map(|byte| {
                [
                    char::from(HEX[usize::from(byte >> 4)]),
                    char::from(HEX[usize::from(byte & 15)]),
                ]
            })
            .collect())
    }
    pub(crate) fn verify(&self, identity: &ControlIdentity, token: &str) -> bool {
        if token.len() != 64 {
            return false;
        }
        let mut bytes = [0_u8; 32];
        for (target, pair) in bytes.iter_mut().zip(token.as_bytes().chunks_exact(2)) {
            let Some(high) = digit(pair[0]) else {
                return false;
            };
            let Some(low) = digit(pair[1]) else {
                return false;
            };
            *target = high * 16 + low;
        }
        Self::message(identity).is_ok_and(|message| hmac::verify(&self.0, &message, &bytes).is_ok())
    }
}
fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
