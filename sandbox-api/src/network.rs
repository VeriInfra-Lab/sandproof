//! Interpret the fixed SDK's address-only network request before admission.
//! No host effect is allowed while this conversion can still reject input.

use sandbox_controller_core::NetworkPolicy;

use crate::{CreateRequest, PortError};

pub(crate) fn selected_policy(request: &CreateRequest) -> Result<NetworkPolicy, PortError> {
    let allow_out = request
        .network
        .as_ref()
        .map_or(&[][..], |network| network.allow_out.as_slice());
    let deny_out = request
        .network
        .as_ref()
        .map_or(&[][..], |network| network.deny_out.as_slice());
    if request
        .network
        .as_ref()
        .is_some_and(|network| network.allow_public_traffic == Some(false))
    {
        return Err(PortError::Invalid);
    }
    NetworkPolicy::from_sdk_ipv4(request.allow_internet_access, allow_out, deny_out)
        .map_err(|_| PortError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CreateNetwork;
    use sandbox_controller_core::{Ipv4Cidr, MAX_IPV4_RULES_PER_LIST};
    use std::net::Ipv4Addr;

    fn request(internet: Option<bool>, allow: &[&str], deny: &[&str]) -> CreateRequest {
        CreateRequest {
            template_id: "base".into(),
            timeout: None,
            allow_internet_access: internet,
            network: Some(CreateNetwork {
                allow_public_traffic: Some(true),
                allow_out: allow.iter().map(|item| (*item).into()).collect(),
                deny_out: deny.iter().map(|item| (*item).into()).collect(),
            }),
            metadata: None,
            env_vars: None,
        }
    }

    #[test]
    fn sdk_defaults_and_false_exceptions_are_distinct() {
        assert!(matches!(
            selected_policy(&request(None, &[], &[])).unwrap(),
            NetworkPolicy::Ipv4 {
                internet_access: true,
                ..
            }
        ));
        assert_eq!(
            selected_policy(&request(Some(false), &[], &["0.0.0.0/0"])).unwrap(),
            NetworkPolicy::DenyAll
        );
        assert!(matches!(
            selected_policy(&request(Some(false), &["1.1.1.1"], &[])).unwrap(),
            NetworkPolicy::Ipv4 {
                internet_access: false,
                ..
            }
        ));
    }

    #[test]
    fn address_lists_are_canonical_and_bounded() {
        let policy = selected_policy(&request(
            None,
            &["1.2.3.4/24", "1.2.3.0/24", "0.0.0.0/0"],
            &["8.8.8.8"],
        ))
        .unwrap();
        let NetworkPolicy::Ipv4 { allow_out, .. } = policy else {
            panic!("expected IPv4 policy")
        };
        assert_eq!(allow_out.len(), 2);
        assert!(allow_out.contains(&Ipv4Cidr {
            network: u32::from(Ipv4Addr::new(1, 2, 3, 0)),
            prefix_length: 24,
        }));
        for invalid in ["example.com", "::1", "0.0.0.0", "1.1.1.1/33", "1.1.1.1/abc"] {
            assert_eq!(
                selected_policy(&request(None, &[invalid], &[])),
                Err(PortError::Invalid)
            );
        }
        let too_many = vec!["1.1.1.1"; MAX_IPV4_RULES_PER_LIST + 1];
        assert_eq!(
            selected_policy(&request(None, &too_many, &[])),
            Err(PortError::Invalid)
        );
    }
}
