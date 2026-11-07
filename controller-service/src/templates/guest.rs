//! Protected identity of the runtime installed in an immutable rootfs.
//! Shape validation is not an installation, compatibility or provenance proof.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuestKind {
    #[serde(rename = "rust-guest-v1")]
    RustGuestV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestFingerprint {
    pub bytes: u64,
    pub sha256: String,
}

impl GuestFingerprint {
    fn valid_with_limit(&self, limit: u64) -> bool {
        self.bytes > 0 && self.bytes <= limit && hex(&self.sha256, 64)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestVersions {
    pub service_package_version: String,
    pub worker_package_version: String,
    pub config_version: u32,
    pub worker_protocol_version: u32,
    pub pid_format_version: u8,
}

impl GuestVersions {
    fn is_valid(&self) -> bool {
        package(&self.service_package_version)
            && package(&self.worker_package_version)
            && self.config_version > 0
            && self.worker_protocol_version > 0
            && self.pid_format_version > 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestBinaries {
    pub service: GuestFingerprint,
    pub worker: GuestFingerprint,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateGuest {
    pub kind: GuestKind,
    /// Independently approved release manifest, not a hash supplied by the guest.
    pub manifest_sha256: String,
    pub source_commit: String,
    pub versions: GuestVersions,
    pub artifacts: GuestBinaries,
    /// Exact installed daemon configuration, including all protected budgets.
    pub configuration: GuestFingerprint,
}

impl TemplateGuest {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        hex(&self.manifest_sha256, 64)
            && hex(&self.source_commit, 40)
            && self.versions.is_valid()
            && self.artifacts.service.valid_with_limit(256 * 1024 * 1024)
            && self.artifacts.worker.valid_with_limit(256 * 1024 * 1024)
            && self.configuration.valid_with_limit(1024 * 1024)
    }
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn package(value: &str) -> bool {
    value.len() <= 17 && has_three_decimal_parts(value)
}

fn has_three_decimal_parts(value: &str) -> bool {
    let mut parts = 1_u8;
    let mut digits = 0_u8;
    for byte in value.as_bytes() {
        if *byte == b'.' {
            if digits == 0 || parts >= 3 {
                return false;
            }
            parts += 1;
            digits = 0;
        } else {
            if !byte.is_ascii_digit() || digits >= 5 {
                return false;
            }
            digits += 1;
        }
    }
    parts == 3 && digits > 0
}

#[cfg(test)]
mod proof_pattern_tests {
    use super::package;

    fn original_package(value: &str) -> bool {
        value.len() <= 17
            && value.split('.').count() == 3
            && value.split('.').all(|part| {
                !part.is_empty()
                    && part.len() <= 5
                    && part.bytes().all(|byte| byte.is_ascii_digit())
            })
    }

    #[test]
    fn package_matches_original_split() {
        for value in [
            "",
            ".",
            "..",
            "...",
            "1",
            "1.2",
            "1.2.3",
            "0.0.0",
            "01.002.00003",
            ".1.2",
            "1..2",
            "1.2.",
            "1.2.3.4",
            "1.2.3.",
            "12345.12345.12345",
            "123456.12345.12345",
            "é.1.2",
            "１２.1.2",
            "١.1.2",
            "1.2.3\0",
            "-1.2.3",
        ] {
            assert_eq!(package(value), original_package(value), "{value:?}");
        }
        let parts = [
            "", "0", "1", "01", "12345", "123456", "a", "é", "１２", "1\0", "-1", ".", "1.2",
        ];
        for first in parts {
            for second in parts {
                for last in parts {
                    let value = format!("{first}.{second}.{last}");
                    assert_eq!(package(&value), original_package(&value), "{value:?}");
                }
            }
        }
        for first in 0..=7 {
            for second in 0..=7 {
                for last in 0..=7 {
                    let value = format!(
                        "{}.{}.{}",
                        "1".repeat(first),
                        "2".repeat(second),
                        "3".repeat(last)
                    );
                    assert_eq!(package(&value), original_package(&value), "{value:?}");
                }
            }
        }
    }
}
