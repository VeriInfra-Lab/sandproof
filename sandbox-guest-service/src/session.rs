use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha512};
use subtle::ConstantTimeEq;
use tokio::sync::{Mutex, OwnedRwLockReadGuard, RwLock, Semaphore, SemaphorePermit};
use zeroize::Zeroizing;

/// Payload-free errors: never include tokens, environment values or metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceError {
    InvalidArgument,
    Unauthenticated,
    ResourceExhausted,
    PayloadTooLarge,
    Unavailable,
    DeadlineExceeded,
}

/// The metadata transport must obtain these fields from this VM's trusted
/// host-provided MMDS. A caller-controlled JSON body is not metadata evidence.
pub struct Metadata {
    pub instance_id: String,
    pub template_id: String,
    pub access_token_sha512: [u8; 64],
}

pub trait MetadataSource: Send + Sync + 'static {
    fn fetch(&self) -> impl Future<Output = Result<Metadata, ServiceError>> + Send;
}

#[derive(Clone, Copy, Debug)]
pub struct InitLimits {
    pub body_bytes: usize,
    pub token_bytes: usize,
    /// Retained user/default-directory/env key/value UTF-8 bytes, not allocator
    /// or total service RAM. Request/connection and metadata budgets are extra.
    pub state_bytes: usize,
    pub env_entries: usize,
    pub in_flight: usize,
    pub timeout: Duration,
}

impl InitLimits {
    pub(crate) fn validate(self) -> Result<Self, ServiceError> {
        if self.body_bytes == 0
            || self.token_bytes == 0
            || self.state_bytes == 0
            || self.env_entries == 0
            || self.in_flight == 0
            || self.timeout.is_zero()
            || self.in_flight > tokio::sync::Semaphore::MAX_PERMITS
            || self.token_bytes > self.body_bytes
            || tokio::time::Instant::now()
                .checked_add(self.timeout)
                .is_none()
        {
            return Err(ServiceError::InvalidArgument);
        }
        Ok(self)
    }
}

/// Template+creation environment is already merged by the original host. The
/// two E2B identity entries are taken from MMDS and cannot be replaced by init;
/// the fixed command API still permits per-command environment overrides.
/// Per-command account resolution, cwd expansion and final env overrides must
/// use one authorized snapshot; these defaults do not constitute a uid/gid.
#[derive(Clone, PartialEq, Eq)]
pub struct Defaults {
    pub user: String,
    pub workdir: Option<String>,
    pub env: BTreeMap<String, String>,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            user: "root".into(),
            workdir: None,
            env: BTreeMap::new(),
        }
    }
}

struct Initialized {
    hash: [u8; 64],
    // Fixed file signing needs the original token, not its MMDS digest.
    // Move the bounded init secret here; never expose it through AuthPermit.
    token: Secret,
    instance: String,
    template: String,
    defaults: Defaults,
}

struct Shared {
    current: Arc<RwLock<Option<Initialized>>>,
    init: Mutex<()>,
    init_requests: Semaphore,
}

/// One service lifetime. Restart begins uninitialized and must authenticate
/// against MMDS again; this type never adopts old command processes or PIDs.
pub struct Session<M> {
    metadata: Arc<M>,
    shared: Arc<Shared>,
    limits: InitLimits,
}

impl<M> Clone for Session<M> {
    fn clone(&self) -> Self {
        Self {
            metadata: Arc::clone(&self.metadata),
            shared: Arc::clone(&self.shared),
            limits: self.limits,
        }
    }
}

/// Hold through the operation's admission/side-effect dispatch, not a long
/// output subscription. Initialization waits for already admitted dispatches;
/// a new request always rechecks the current token. No token is exposed here.
pub struct AuthPermit {
    guard: OwnedRwLockReadGuard<Option<Initialized>, Initialized>,
}

impl AuthPermit {
    #[must_use]
    pub fn defaults(&self) -> &Defaults {
        &self.guard.defaults
    }
}

impl<M: MetadataSource> Session<M> {
    #[cfg(target_os = "linux")]
    pub(crate) fn shares_state(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }
    /// # Errors
    /// Refuses zero, inconsistent or unrepresentable initialization limits.
    pub fn new(metadata: M, limits: InitLimits) -> Result<Self, ServiceError> {
        let limits = limits.validate()?;
        Ok(Self {
            metadata: Arc::new(metadata),
            shared: Arc::new(Shared {
                current: Arc::new(RwLock::new(None)),
                init: Mutex::new(()),
                init_requests: Semaphore::new(limits.in_flight),
            }),
            limits,
        })
    }

    #[must_use]
    pub const fn limits(&self) -> InitLimits {
        self.limits
    }

    pub(crate) fn try_init_slot(&self) -> Result<SemaphorePermit<'_>, ServiceError> {
        self.shared
            .init_requests
            .try_acquire()
            .map_err(|_| ServiceError::ResourceExhausted)
    }

    /// # Errors
    /// Missing, malformed, oversized, stale and pre-init tokens all fail
    /// closed. The caller separately bounds request slots and lock wait time.
    pub async fn authorize(&self, token: Option<&str>) -> Result<AuthPermit, ServiceError> {
        let token = token.ok_or(ServiceError::Unauthenticated)?;
        let digest = token_digest(token, self.limits.token_bytes)?;
        let guard = Arc::clone(&self.shared.current).read_owned().await;
        let guard = OwnedRwLockReadGuard::try_map(guard, Option::as_ref)
            .map_err(|_| ServiceError::Unauthenticated)?;
        if !bool::from(guard.hash.ct_eq(&digest)) {
            return Err(ServiceError::Unauthenticated);
        }
        Ok(AuthPermit { guard })
    }

    /// Apply the original host's restricted init body atomically. Concurrent
    /// init is rejected without a queue; same-token replay is convergent. A
    /// cancelled call either leaves the old state or has already committed the
    /// entire replacement. It never installs a token/defaults partial state.
    /// # Errors
    /// Malformed/over-budget input, invalid token/metadata, unavailable MMDS,
    /// overlapping init or deadline expiration leaves no new authorization.
    /// A lost response after commit remains unknown to the host, not undone.
    pub async fn initialize(&self, body: &[u8]) -> Result<(), ServiceError> {
        let request = self.parse(body)?;
        tokio::time::timeout(self.limits.timeout, self.apply(request))
            .await
            .map_err(|_| ServiceError::DeadlineExceeded)?
    }

    fn parse(&self, body: &[u8]) -> Result<InitRequest, ServiceError> {
        if body.len() > self.limits.body_bytes {
            return Err(ServiceError::PayloadTooLarge);
        }
        if body.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
            return Err(ServiceError::InvalidArgument);
        }
        serde_json::from_slice(body).map_err(|_| ServiceError::InvalidArgument)
    }

    async fn apply(&self, mut request: InitRequest) -> Result<(), ServiceError> {
        let token = request
            .access_token
            .take()
            .ok_or(ServiceError::Unauthenticated)?;
        let digest = token_digest(&token.0, self.limits.token_bytes)?;
        let _init = self
            .shared
            .init
            .try_lock()
            .map_err(|_| ServiceError::ResourceExhausted)?;
        let (current_hash, identity) = {
            let state = self.shared.current.read().await;
            state.as_ref().map_or((None, None), |state| {
                (
                    Some(state.hash),
                    Some((state.instance.clone(), state.template.clone())),
                )
            })
        };
        let (instance, template) =
            if current_hash.is_some_and(|hash| bool::from(hash.ct_eq(&digest))) {
                identity.ok_or(ServiceError::Unavailable)?
            } else {
                let metadata = self.metadata.fetch().await?;
                if metadata.instance_id.is_empty()
                    || metadata.template_id.is_empty()
                    || metadata.instance_id.contains('\0')
                    || metadata.template_id.contains('\0')
                    || metadata.instance_id.len() > self.limits.state_bytes
                    || metadata.template_id.len() > self.limits.state_bytes
                    || identity
                        .as_ref()
                        .is_some_and(|(original_instance, original_template)| {
                            original_instance != &metadata.instance_id
                                || original_template != &metadata.template_id
                        })
                    || !bool::from(metadata.access_token_sha512.ct_eq(&digest))
                {
                    return Err(ServiceError::Unauthenticated);
                }
                (metadata.instance_id, metadata.template_id)
            };
        let mut current = self.shared.current.write().await;
        let mut defaults = current
            .as_ref()
            .map_or_else(Defaults::default, |state| state.defaults.clone());
        apply_defaults(&mut defaults, request, &instance, &template, self.limits)?;
        *current = Some(Initialized {
            hash: digest,
            token,
            instance,
            template,
            defaults,
        });
        Ok(())
    }
}

mod file_authorization;

struct Secret(Zeroizing<String>);

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|value| Self(Zeroizing::new(value)))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InitRequest {
    access_token: Option<Secret>,
    default_user: Option<String>,
    default_workdir: Option<String>,
    env_vars: Option<BTreeMap<String, String>>,
}

fn token_digest(token: &str, limit: usize) -> Result<[u8; 64], ServiceError> {
    if token.is_empty()
        || token.len() > limit
        || !token.bytes().all(|byte| (33..=126).contains(&byte))
    {
        return Err(ServiceError::Unauthenticated);
    }
    Ok(Sha512::digest(token.as_bytes()).into())
}

fn apply_defaults(
    defaults: &mut Defaults,
    request: InitRequest,
    instance: &str,
    template: &str,
    limits: InitLimits,
) -> Result<(), ServiceError> {
    if let Some(user) = request.default_user.filter(|value| !value.is_empty()) {
        defaults.user = user;
    }
    if let Some(workdir) = request.default_workdir.filter(|value| !value.is_empty()) {
        defaults.workdir = Some(workdir);
    }
    if let Some(env) = request.env_vars {
        defaults.env = env;
    }
    defaults
        .env
        .insert("E2B_SANDBOX_ID".into(), instance.into());
    defaults
        .env
        .insert("E2B_TEMPLATE_ID".into(), template.into());
    if defaults.user.contains('\0')
        || defaults
            .workdir
            .as_ref()
            .is_some_and(|value| value.contains('\0'))
        || defaults
            .env
            .iter()
            .any(|(key, value)| key.is_empty() || key.contains(['=', '\0']) || value.contains('\0'))
    {
        return Err(ServiceError::InvalidArgument);
    }
    let size = defaults
        .user
        .len()
        .checked_add(defaults.workdir.as_ref().map_or(0, String::len))
        .and_then(|size| {
            defaults.env.iter().try_fold(size, |size, (key, value)| {
                size.checked_add(key.len())?.checked_add(value.len())
            })
        });
    if defaults.env.len() > limits.env_entries || size.is_none_or(|size| size > limits.state_bytes)
    {
        return Err(ServiceError::ResourceExhausted);
    }
    Ok(())
}
