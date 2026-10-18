//! Authenticated default/context composition. This does not register a public
//! command or release a process. The caller retains `AuthPermit` through actual
//! admission/dispatch and separately owns process/public-selector capacity.
use crate::task_accounting::{self, BlockingTask};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::http::HeaderMap;
use sandproof_guest_runtime::accounts::{Account, AccountError, AccountLimits, lookup_local_user};
use sandproof_guest_runtime::protocol::{ProcessConfig, StartRequest};
use sandproof_guest_runtime::{LaunchSpec, PtySize};
use tokio::sync::Semaphore;

use crate::AuthPermit;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextError {
    InvalidArgument,
    Unauthenticated,
    ResourceExhausted,
    Unavailable,
    DeadlineExceeded,
    Unimplemented,
}

#[derive(Debug)]
pub(crate) enum CommandContextError {
    Context(ContextError),
    Expansion(crate::upload_response::PathExpansion),
}
impl CommandContextError {
    pub(crate) const fn context(&self) -> ContextError {
        match self {
            Self::Context(error) => *error,
            Self::Expansion(_) => ContextError::InvalidArgument,
        }
    }
}
impl From<ContextError> for CommandContextError {
    fn from(error: ContextError) -> Self {
        Self::Context(error)
    }
}

/// Blocking, read-only account lookup. Implementations never execute commands.
pub trait AccountSource: Send + Sync + 'static {
    /// # Errors
    /// Reports unknown accounts, invalid source data, I/O and budget failures.
    fn lookup(&self, username: &str) -> Result<Account, AccountError>;
}

pub struct SystemAccounts {
    limits: AccountLimits,
}
impl AccountSource for SystemAccounts {
    fn lookup(&self, username: &str) -> Result<Account, AccountError> {
        lookup_local_user(username, self.limits)
    }
}

/// Bounds submitted AND actually running filesystem lookups. Cancellation or
/// response timeout cannot return capacity before the blocking closure exits.
pub struct AccountPool<S> {
    source: Arc<S>,
    slots: Arc<Semaphore>,
    timeout: Duration,
}

impl<S> Clone for AccountPool<S> {
    fn clone(&self) -> Self {
        Self {
            source: Arc::clone(&self.source),
            slots: Arc::clone(&self.slots),
            timeout: self.timeout,
        }
    }
}

impl AccountPool<SystemAccounts> {
    /// # Errors
    /// Rejects invalid source, concurrency and deadline budgets before I/O.
    pub fn local(
        limits: AccountLimits,
        in_flight: usize,
        timeout: Duration,
    ) -> Result<Self, ContextError> {
        let limits = limits.validate().map_err(account_error)?;
        Self::new(SystemAccounts { limits }, in_flight, timeout)
    }
}

impl<S: AccountSource> AccountPool<S> {
    /// # Errors
    /// Rejects zero/unrepresentable concurrency or timeout configuration.
    pub fn new(source: S, in_flight: usize, timeout: Duration) -> Result<Self, ContextError> {
        if in_flight == 0
            || in_flight > Semaphore::MAX_PERMITS
            || timeout.is_zero()
            || tokio::time::Instant::now().checked_add(timeout).is_none()
        {
            return Err(ContextError::InvalidArgument);
        }
        Ok(Self {
            source: Arc::new(source),
            slots: Arc::new(Semaphore::new(in_flight)),
            timeout,
        })
    }

    pub(crate) async fn lookup(&self, username: String) -> Result<Account, ContextError> {
        let deadline = tokio::time::Instant::now()
            .checked_add(self.timeout)
            .ok_or(ContextError::InvalidArgument)?;
        self.lookup_admitted(username, deadline, Arc::new(())).await
    }

    pub(crate) async fn lookup_admitted(
        &self,
        username: String,
        deadline: tokio::time::Instant,
        owner: crate::owned_body::Owner,
    ) -> Result<Account, ContextError> {
        let deadline = deadline.min(
            tokio::time::Instant::now()
                .checked_add(self.timeout)
                .ok_or(ContextError::InvalidArgument)?,
        );
        if tokio::time::Instant::now() >= deadline {
            return Err(ContextError::DeadlineExceeded);
        }
        let permit = Arc::clone(&self.slots)
            .try_acquire_owned()
            .map_err(|_| ContextError::ResourceExhausted)?;
        let source = Arc::clone(&self.source);
        let task = task_accounting::spawn_blocking(BlockingTask::AccountLookup, move || {
            let _owner = owner;
            let _permit = permit;
            if tokio::time::Instant::now() >= deadline {
                return Err(ContextError::DeadlineExceeded);
            }
            source.lookup(&username).map_err(account_error)
        });
        let result = tokio::time::timeout_at(deadline, task)
            .await
            .map_err(|_| ContextError::DeadlineExceeded)?
            .map_err(|_| ContextError::Unavailable)?;
        if tokio::time::Instant::now() >= deadline {
            return Err(ContextError::DeadlineExceeded);
        }
        result
    }
}

fn account_error(error: AccountError) -> ContextError {
    match error {
        AccountError::InvalidArgument => ContextError::InvalidArgument,
        AccountError::NotFound => ContextError::Unauthenticated,
        AccountError::ResourceExhausted => ContextError::ResourceExhausted,
        AccountError::Unavailable => ContextError::Unavailable,
    }
}

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextLimits {
    pub bytes: usize,
    pub env_entries: usize,
    pub arguments: usize,
    pub username_bytes: usize,
    pub authorization_bytes: usize,
}

/// Captured by the future daemon from its trusted launch configuration. Only
/// PATH is inherited; HOME/USER/LOGNAME originate from the selected account.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionSettings {
    pub daemon_path: String,
    pub daemon_workdir: String,
    pub limits: ContextLimits,
}

impl AuthPermit {
    /// Build a resolved launch from this exact authenticated defaults snapshot.
    /// Environment USER/HOME never select a kernel identity. No user code runs.
    /// # Errors
    /// Invalid/oversized context, unknown account and source failure/deadline
    /// are explicit errors; there is no fallback to daemon uid.
    pub async fn resolve_command<S: AccountSource>(
        &self,
        request: &StartRequest,
        headers: &HeaderMap,
        accounts: &AccountPool<S>,
        settings: &ExecutionSettings,
    ) -> Result<LaunchSpec, ContextError> {
        command_process(request, settings)?;
        let limit = settings.limits;
        let name = selected_user(headers, &self.defaults().user, limit)?;
        let account = accounts.lookup(name.clone()).await?;
        if account.name != name {
            return Err(ContextError::Unavailable);
        }
        self.compose_command(request, account, settings)
    }

    // The account has already been selected under this exact authorization.
    // This pure phase cannot refresh account lookup or execute a command.
    pub(crate) fn compose_command(
        &self,
        request: &StartRequest,
        account: Account,
        settings: &ExecutionSettings,
    ) -> Result<LaunchSpec, ContextError> {
        self.compose_command_observed(request, account, settings)
            .map_err(|error| error.context())
    }

    pub(crate) fn compose_command_observed(
        &self,
        request: &StartRequest,
        account: Account,
        settings: &ExecutionSettings,
    ) -> Result<LaunchSpec, CommandContextError> {
        let limit = settings.limits;
        let process = command_process(request, settings)?;
        let cwd = self.command_cwd(process.cwd.as_deref(), &account, settings)?;
        let environment = merged_environment(
            [
                ("PATH", settings.daemon_path.as_str()),
                ("HOME", account.home.as_str()),
                ("USER", account.name.as_str()),
                ("LOGNAME", account.name.as_str()),
            ],
            &self.defaults().env,
            &process.envs,
        );
        let mut bytes = std::iter::once(process.cmd.len())
            .chain(std::iter::once(cwd.len()))
            .chain(process.args.iter().map(String::len))
            .try_fold(0usize, usize::checked_add);
        let mut entries = Some(0usize);
        for (key, value) in environment.clone() {
            // Invalid keys still precede count/byte refusal, including when a
            // previous entry has already exceeded either budget. No full copy
            // of the request/default map exists during these checks.
            if invalid_environment_name(key) {
                return Err(ContextError::InvalidArgument.into());
            }
            entries = entries.and_then(|count| count.checked_add(1));
            bytes = bytes
                .and_then(|count| count.checked_add(key.len()))
                .and_then(|count| count.checked_add(value.len()));
        }
        if entries.is_none_or(|count| count > limit.env_entries)
            || bytes.is_none_or(|count| count > limit.bytes)
        {
            return Err(ContextError::ResourceExhausted.into());
        }
        let env = environment
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect();
        Ok(LaunchSpec {
            envd_wrapper_path: Some(settings.daemon_path.clone()),
            program: process.cmd.clone(),
            args: process.args.clone(),
            cwd: cwd.into(),
            env,
            stdin: request.stdin_enabled(),
            pty: request.pty.as_ref().map(|pty| {
                pty.size.as_ref().map_or_else(PtySize::default, |size| {
                    PtySize::from_wire(size.cols, size.rows)
                })
            }),
            credentials: Some(account.credentials),
        })
    }

    fn command_cwd(
        &self,
        cwd: Option<&str>,
        account: &Account,
        settings: &ExecutionSettings,
    ) -> Result<String, CommandContextError> {
        resolve_path(
            cwd.filter(|path| !path.is_empty())
                .or(self.defaults().workdir.as_deref())
                .unwrap_or(""),
            account,
            &settings.daemon_workdir,
        )
    }

    // The caller already checked matching original limits, account and clock.
    // All unique request keys remain in the effective environment after merge,
    // so a request map alone exceeding env_entries cannot become a legal launch.
    // Keep cwd and invalid-name precedence without owning that oversized map.
    pub(crate) fn environment_refusal_observed(
        &self,
        cwd: Option<&str>,
        invalid_request_name: bool,
        account: &Account,
        settings: &ExecutionSettings,
    ) -> CommandContextError {
        if let Err(error) = self.command_cwd(cwd, account, settings) {
            return error;
        }
        if invalid_request_name
            || self
                .defaults()
                .env
                .keys()
                .any(|key| invalid_environment_name(key))
        {
            ContextError::InvalidArgument.into()
        } else {
            ContextError::ResourceExhausted.into()
        }
    }
}

fn invalid_environment_name(key: &str) -> bool {
    key.is_empty() || key.contains('=')
}

// Borrow only the effective entries in the original override order. The four
// builtins have unique keys; both other inputs are already decoded unique maps.
// Iteration order is private: the final LaunchSpec still owns a sorted map.
fn merged_environment<'a>(
    base: [(&'a str, &'a str); 4],
    defaults: &'a BTreeMap<String, String>,
    request: &'a BTreeMap<String, String>,
) -> impl Iterator<Item = (&'a str, &'a str)> + Clone {
    base.into_iter()
        .filter(move |(key, _)| !defaults.contains_key(*key) && !request.contains_key(*key))
        .chain(
            defaults
                .iter()
                .filter(move |(key, _)| !request.contains_key(*key))
                .map(|(key, value)| (key.as_str(), value.as_str())),
        )
        .chain(
            request
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        )
}

pub(crate) fn validate_command_settings(settings: &ExecutionSettings) -> Result<(), ContextError> {
    let limit = settings.limits;
    if limit.bytes == 0
        || limit.env_entries == 0
        || limit.arguments == 0
        || limit.username_bytes == 0
        || limit.authorization_bytes == 0
        || !settings.daemon_workdir.starts_with('/')
        || settings.daemon_workdir.contains('\0')
        || settings.daemon_path.contains('\0')
    {
        return Err(ContextError::InvalidArgument);
    }
    Ok(())
}

fn command_process<'a>(
    request: &'a StartRequest,
    settings: &ExecutionSettings,
) -> Result<Cow<'a, ProcessConfig>, ContextError> {
    validate_command_settings(settings)?;
    let limit = settings.limits;
    let process = request
        .process
        .as_ref()
        .map_or_else(|| Cow::Owned(ProcessConfig::default()), Cow::Borrowed);
    if process.args.len() > limit.arguments {
        return Err(ContextError::ResourceExhausted);
    }
    Ok(process)
}

fn selected_user(
    headers: &HeaderMap,
    fallback: &str,
    limits: ContextLimits,
) -> Result<String, ContextError> {
    selected_user_with_limits(
        headers,
        fallback,
        limits.username_bytes,
        limits.authorization_bytes,
    )
}

pub(crate) fn selected_user_with_limits(
    headers: &HeaderMap,
    fallback: &str,
    username_bytes: usize,
    authorization_bytes: usize,
) -> Result<String, ContextError> {
    let user = crate::basic_auth::explicit_user(headers, username_bytes, authorization_bytes)
        .map_err(|error| match error {
            crate::basic_auth::Error::Unauthenticated => ContextError::Unauthenticated,
            crate::basic_auth::Error::ResourceExhausted => ContextError::ResourceExhausted,
        })?
        .unwrap_or_else(|| fallback.into());
    if user.is_empty() || user.contains(['\0', '\n', '\r', ':']) {
        return Err(ContextError::Unauthenticated);
    }
    if user.len() > username_bytes {
        return Err(ContextError::ResourceExhausted);
    }
    Ok(user)
}

fn resolve_path(
    path: &str,
    account: &Account,
    daemon_cwd: &str,
) -> Result<String, CommandContextError> {
    if account.home.contains('\0') {
        return Err(ContextError::InvalidArgument.into());
    }
    expand_path(path, &account.home, daemon_cwd).map_err(|error| match error {
        ContextError::InvalidArgument => CommandContextError::Expansion(
            crate::upload_response::PathExpansion::new(&account.name),
        ),
        other => CommandContextError::Context(other),
    })
}

pub(crate) fn expand_path(
    path: &str,
    home: &str,
    daemon_cwd: &str,
) -> Result<String, ContextError> {
    let expanded = if let Some(rest) = path.strip_prefix('~') {
        if !rest.is_empty() && !rest.starts_with(['/', '\\']) {
            return Err(ContextError::InvalidArgument);
        }
        join_clean(&[home, rest])
    } else {
        path.into()
    };
    // The fixed endpoint returns an already absolute request unchanged; it
    // lexically cleans joined home-relative paths without resolving symlinks.
    if expanded.starts_with('/') {
        return Ok(expanded);
    }
    let from_home = join_clean(&[home, &expanded]);
    Ok(if from_home.starts_with('/') {
        from_home
    } else {
        join_clean(&[daemon_cwd, &from_home])
    })
}

fn join_clean(parts: &[&str]) -> String {
    let joined = parts
        .iter()
        .copied()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if joined.is_empty() {
        return joined;
    }
    let absolute = joined.starts_with('/');
    let mut segments = Vec::new();
    for segment in joined.split('/') {
        match segment {
            ".." if segments.last().is_some_and(|last| *last != "..") => {
                segments.pop();
            }
            ".." if !absolute => segments.push(segment),
            "" | "." | ".." => {}
            _ => segments.push(segment),
        }
    }
    let path = segments.join("/");
    if absolute {
        format!("/{path}")
    } else if path.is_empty() {
        ".".into()
    } else {
        path
    }
}
