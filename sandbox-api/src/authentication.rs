//! Fixed service-role authentication. Selecting a router fixes its trust
//! boundary; request headers can never switch public requests into build work.

use std::sync::Arc;

use axum::http::HeaderMap;

use crate::{ApiCredential, ConfigurationError};

pub const PRIVATE_BUILD_CREDENTIAL_HEADER: &str = "x-sandproof-build-key";
pub const PRIVATE_BUILD_TENANT_HEADER: &str = "x-sandproof-build-owner";

/// One operator-provisioned backend credential, not a copy of user API keys.
/// This authenticates the private helper only. The port must separately check
/// the current registered tenant and exact build anchor under admission.
#[derive(Clone)]
pub struct PrivateBuildCredential {
    secret: Arc<str>,
}

impl PrivateBuildCredential {
    /// # Errors
    /// Rejects malformed credentials. Operators must supply real entropy.
    pub fn new(secret: String) -> Result<Self, ConfigurationError> {
        if !(32..=256).contains(&secret.len())
            || !secret.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(ConfigurationError::WeakOrDuplicateSecret);
        }
        Ok(Self {
            secret: secret.into(),
        })
    }

    /// Accepts one backend key and one canonical positive numeric owner.
    /// No I/O occurs here: ownership authorization belongs to the port.
    #[must_use]
    pub fn authenticate(&self, headers: &HeaderMap) -> Option<String> {
        let supplied = exactly_one(headers, PRIVATE_BUILD_CREDENTIAL_HEADER)?;
        if !secret_equal(supplied, self.secret.as_bytes()) {
            return None;
        }
        let owner = std::str::from_utf8(exactly_one(headers, PRIVATE_BUILD_TENANT_HEADER)?).ok()?;
        let numeric: u64 = owner.parse().ok()?;
        (numeric > 0 && numeric.to_string() == owner).then(|| owner.to_owned())
    }
}

#[derive(Clone)]
pub(crate) enum Authentication {
    Public(Arc<[ApiCredential]>),
    PrivateBuild(PrivateBuildCredential),
}

impl Authentication {
    pub(crate) fn authenticate(&self, headers: &HeaderMap) -> Option<String> {
        match self {
            Self::Public(credentials) => authenticate_public(credentials, headers),
            Self::PrivateBuild(credential) => credential.authenticate(headers),
        }
    }
}

/// Shared pure API-key lookup for the SDK parser and protected build proxy.
/// Only operator-validated credentials may be supplied. Authorization against
/// current resource ownership remains the downstream port's responsibility.
#[must_use]
pub fn authenticate_public(credentials: &[ApiCredential], headers: &HeaderMap) -> Option<String> {
    let supplied = exactly_one(headers, "x-api-key")?;
    let mut tenant = None;
    for credential in credentials {
        if secret_equal(supplied, credential.secret.as_bytes()) {
            tenant = Some(credential.tenant.clone());
        }
    }
    tenant
}

fn exactly_one<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a [u8]> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?;
    values.next().is_none().then(|| value.as_bytes())
}

/// Same-length comparison does not exit early on credential contents.
pub(crate) fn secret_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (&a, &b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    const KEY: &str = "private-build-test-credential-000001";

    fn headers(owner: &str) -> HeaderMap {
        let mut result = HeaderMap::new();
        result.insert(
            PRIVATE_BUILD_CREDENTIAL_HEADER,
            HeaderValue::from_static(KEY),
        );
        result.insert(
            PRIVATE_BUILD_TENANT_HEADER,
            HeaderValue::from_str(owner).unwrap(),
        );
        result
    }

    #[test]
    fn private_authentication_requires_one_exact_key_and_canonical_owner() {
        let auth = PrivateBuildCredential::new(KEY.into()).unwrap();
        for owner in ["1", "18446744073709551615"] {
            assert_eq!(auth.authenticate(&headers(owner)).as_deref(), Some(owner));
        }
        for owner in [
            "",
            "0",
            "01",
            "+1",
            "-1",
            " 1",
            "1 ",
            "18446744073709551616",
            "alice",
        ] {
            assert!(auth.authenticate(&headers(owner)).is_none(), "{owner}");
        }
        for name in [PRIVATE_BUILD_CREDENTIAL_HEADER, PRIVATE_BUILD_TENANT_HEADER] {
            let mut duplicate = headers("1");
            duplicate.append(name, duplicate[name].clone());
            assert!(auth.authenticate(&duplicate).is_none());
            duplicate.remove(name);
            assert!(auth.authenticate(&duplicate).is_none());
        }
        let mut wrong = headers("1");
        wrong.insert(
            PRIVATE_BUILD_CREDENTIAL_HEADER,
            HeaderValue::from_static("private-build-test-credential-000002"),
        );
        assert!(auth.authenticate(&wrong).is_none());
        wrong.remove(PRIVATE_BUILD_CREDENTIAL_HEADER);
        wrong.insert("x-api-key", HeaderValue::from_static(KEY));
        assert!(auth.authenticate(&wrong).is_none());
    }

    #[test]
    fn private_secret_validation_and_public_role_are_independent() {
        for secret in [
            "x".repeat(31),
            "x".repeat(257),
            format!("{}\n", "x".repeat(32)),
        ] {
            assert!(PrivateBuildCredential::new(secret).is_err());
        }
        let public = Authentication::Public(
            vec![ApiCredential {
                tenant: "alice".into(),
                secret: KEY.into(),
            }]
            .into(),
        );
        assert!(public.authenticate(&headers("1")).is_none());
        let mut user = HeaderMap::new();
        user.insert("x-api-key", HeaderValue::from_static(KEY));
        user.insert(PRIVATE_BUILD_TENANT_HEADER, HeaderValue::from_static("2"));
        assert_eq!(public.authenticate(&user).as_deref(), Some("alice"));
    }
}
