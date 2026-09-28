//! Canonical IPv4 policy values shared by the pure controller, API and host.
//! The changing host-local address set remains an external enforcement input.

use std::{collections::BTreeSet, fmt, net::Ipv4Addr};

use crate::{Ipv4Cidr, NetworkPolicy};

pub const MAX_IPV4_RULES_PER_LIST: usize = 128;
pub const PLATFORM_DENY_IPV4: &[Ipv4Cidr] = &[
    Ipv4Cidr {
        network: 0x0000_0000,
        prefix_length: 8,
    },
    Ipv4Cidr {
        network: 0x0a00_0000,
        prefix_length: 8,
    },
    Ipv4Cidr {
        network: 0x6440_0000,
        prefix_length: 10,
    },
    Ipv4Cidr {
        network: 0x7f00_0000,
        prefix_length: 8,
    },
    Ipv4Cidr {
        network: 0xa9fe_0000,
        prefix_length: 16,
    },
    Ipv4Cidr {
        network: 0xac10_0000,
        prefix_length: 12,
    },
    Ipv4Cidr {
        network: 0xc000_0000,
        prefix_length: 24,
    },
    Ipv4Cidr {
        network: 0xc000_0200,
        prefix_length: 24,
    },
    Ipv4Cidr {
        network: 0xc058_6300,
        prefix_length: 24,
    },
    Ipv4Cidr {
        network: 0xc0a8_0000,
        prefix_length: 16,
    },
    Ipv4Cidr {
        network: 0xc612_0000,
        prefix_length: 15,
    },
    Ipv4Cidr {
        network: 0xc633_6400,
        prefix_length: 24,
    },
    Ipv4Cidr {
        network: 0xcb00_7100,
        prefix_length: 24,
    },
    Ipv4Cidr {
        network: 0xe000_0000,
        prefix_length: 4,
    },
    Ipv4Cidr {
        network: 0xf000_0000,
        prefix_length: 4,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdkIpv4Error {
    TooManyRules,
    InvalidAddress,
    UnsupportedAddressKind,
}

impl fmt::Display for Ipv4Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", Ipv4Addr::from(self.network), self.prefix_length)
    }
}

impl Ipv4Cidr {
    /// Parse and canonicalize one fixed-SDK IPv4 address or CIDR entry.
    /// # Errors
    /// Rejects domains, IPv6, malformed CIDRs and unsupported zero-address forms.
    pub fn from_sdk(value: &str) -> Result<Self, SdkIpv4Error> {
        let (address, prefix_length) = if let Some((address, length)) = value.split_once('/') {
            let length = length
                .parse::<u8>()
                .map_err(|_| SdkIpv4Error::InvalidAddress)?;
            (address, length)
        } else {
            (value, 32)
        };
        if prefix_length > 32 || (address == "0.0.0.0" && value != "0.0.0.0/0") {
            return Err(SdkIpv4Error::InvalidAddress);
        }
        let address = address.parse::<Ipv4Addr>().map_err(|_| {
            if value.contains(':') || value.chars().any(char::is_alphabetic) {
                SdkIpv4Error::UnsupportedAddressKind
            } else {
                SdkIpv4Error::InvalidAddress
            }
        })?;
        let mask = if prefix_length == 0 {
            0
        } else {
            u32::MAX << (32 - prefix_length)
        };
        Ok(Self {
            network: u32::from(address) & mask,
            prefix_length,
        })
    }
}

/// Sort, deduplicate and canonicalize one SDK address list.
/// # Errors
/// Rejects an oversized list or any unsupported/malformed address.
pub fn normalize_sdk_ipv4_list(values: &[String]) -> Result<Vec<Ipv4Cidr>, SdkIpv4Error> {
    if values.len() > MAX_IPV4_RULES_PER_LIST {
        return Err(SdkIpv4Error::TooManyRules);
    }
    values
        .iter()
        .map(|value| Ipv4Cidr::from_sdk(value))
        .collect::<Result<BTreeSet<_>, _>>()
        .map(|entries| entries.into_iter().collect())
}

/// Static platform exclusions; actual host-local addresses are additional.
#[must_use]
pub fn platform_protected_ipv4(address: u32) -> bool {
    PLATFORM_DENY_IPV4
        .iter()
        .any(|range| range.contains(address))
}

impl NetworkPolicy {
    /// Convert the fixed SDK's address lists to one canonical durable policy.
    /// False with no allow exceptions is exactly the existing deny-all mode.
    /// # Errors
    /// Rejects unsupported or excessive rules before resource admission.
    pub fn from_sdk_ipv4(
        internet_access: Option<bool>,
        allow_out: &[String],
        deny_out: &[String],
    ) -> Result<Self, SdkIpv4Error> {
        let allow_out = normalize_sdk_ipv4_list(allow_out)?;
        let deny_out = normalize_sdk_ipv4_list(deny_out)?;
        if internet_access == Some(false) && allow_out.is_empty() {
            return Ok(Self::DenyAll);
        }
        Ok(Self::Ipv4 {
            internet_access: internet_access.unwrap_or(true),
            allow_out,
            deny_out,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalization_and_platform_floor() {
        let all = Ipv4Cidr::from_sdk("0.0.0.0/0").unwrap();
        assert!(all.contains(u32::from(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(platform_protected_ipv4(u32::from(Ipv4Addr::new(
            169, 254, 169, 254
        ))));
        assert!(!platform_protected_ipv4(u32::from(Ipv4Addr::new(
            1, 1, 1, 1
        ))));
        assert_eq!(
            Ipv4Cidr::from_sdk("1.2.3.4/24").unwrap().to_string(),
            "1.2.3.0/24"
        );
        assert!(
            !Ipv4Cidr {
                network: 1,
                prefix_length: 0
            }
            .contains(123)
        );
    }
}
