//! Authenticated default/context composition. This does not register a public
//! command or release a process. The caller retains `AuthPermit` through actual
//! admission/dispatch and separately owns process/public-selector capacity.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::http::HeaderMap;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use sandproof_guest_runtime::accounts::{Account, AccountError, AccountLimits, lookup_local_user};
use sandproof_guest_runtime::protocol::StartRequest;
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
        let permit = Arc::clone(&self.slots)
            .try_acquire_owned()
            .map_err(|_| ContextError::ResourceExhausted)?;
        let source = Arc::clone(&self.source);
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            source.lookup(&username)
        });
        tokio::time::timeout(self.timeout, task)
            .await
            .map_err(|_| ContextError::DeadlineExceeded)?
            .map_err(|_| ContextError::Unavailable)?
            .map_err(account_error)
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

#[derive(Clone, Copy, Debug)]
pub struct ContextLimits {
    pub bytes: usize,
    pub env_entries: usize,
    pub arguments: usize,
    pub username_bytes: usize,
    pub authorization_bytes: usize,
}

/// Captured by the future daemon from its trusted launch configuration. Only
/// PATH is inherited; HOME/USER/LOGNAME originate from the selected account.
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
        let process = request
            .process
            .as_ref()
            .ok_or(ContextError::InvalidArgument)?;
        if process.cmd.is_empty()
            || process.cmd.contains('\0')
            || process.args.iter().any(|arg| arg.contains('\0'))
        {
            return Err(ContextError::InvalidArgument);
        }
        if process.args.len() > limit.arguments {
            return Err(ContextError::ResourceExhausted);
        }
        let name = selected_user(headers, &self.defaults().user, limit)?;
        let account = accounts.lookup(name.clone()).await?;
        if account.name != name {
            return Err(ContextError::Unavailable);
        }
        let cwd = resolve_path(
            process
                .cwd
                .as_deref()
                .filter(|path| !path.is_empty())
                .or(self.defaults().workdir.as_deref())
                .unwrap_or(""),
            &account.home,
            &settings.daemon_workdir,
        )?;
        let mut env = BTreeMap::from([
            ("PATH".into(), settings.daemon_path.clone()),
            ("HOME".into(), account.home.clone()),
            ("USER".into(), account.name.clone()),
            ("LOGNAME".into(), account.name),
        ]);
        env.extend(self.defaults().env.clone());
        env.extend(process.envs.clone());
        if env
            .iter()
            .any(|(key, value)| key.is_empty() || key.contains(['=', '\0']) || value.contains('\0'))
        {
            return Err(ContextError::InvalidArgument);
        }
        let mut sizes = std::iter::once(process.cmd.len())
            .chain(std::iter::once(cwd.len()))
            .chain(process.args.iter().map(String::len))
            .chain(env.iter().flat_map(|(key, value)| [key.len(), value.len()]));
        if env.len() > limit.env_entries
            || sizes
                .try_fold(0usize, usize::checked_add)
                .is_none_or(|sum| sum > limit.bytes)
        {
            return Err(ContextError::ResourceExhausted);
        }
        Ok(LaunchSpec {
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
    let mut values = headers.get_all("authorization").iter();
    let value = values.next();
    if values.next().is_some() {
        return Err(ContextError::Unauthenticated);
    }
    let mut user = None;
    if let Some(value) = value {
        if value.as_bytes().len() > authorization_bytes {
            return Err(ContextError::ResourceExhausted);
        }
        if let Ok(value) = value.to_str() {
            // Fixed Go BasicAuth returns ok=false for other schemes/malformed
            // input, which then selects the initialized default username.
            if value
                .get(..6)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Basic "))
                && let Ok(decoded) = STANDARD.decode(&value[6..])
                && let Some(colon) = decoded.iter().position(|byte| *byte == b':')
            {
                user = Some(
                    std::str::from_utf8(&decoded[..colon])
                        .map_err(|_| ContextError::Unauthenticated)?
                        .to_owned(),
                );
            }
        }
    }
    let user = user.unwrap_or_else(|| fallback.into());
    if user.is_empty() || user.contains(['\0', '\n', '\r', ':']) {
        return Err(ContextError::Unauthenticated);
    }
    if user.len() > username_bytes {
        return Err(ContextError::ResourceExhausted);
    }
    Ok(user)
}

fn resolve_path(path: &str, home: &str, daemon_cwd: &str) -> Result<String, ContextError> {
    if path.contains('\0') || home.contains('\0') {
        return Err(ContextError::InvalidArgument);
    }
    expand_path(path, home, daemon_cwd)
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
