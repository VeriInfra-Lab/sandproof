//! File account/path resolution from the original authenticated snapshot.
//! This does no file I/O and grants no per-user DAC isolation. The future
//! dispatcher must retain the original authorization until side effects settle.
use axum::http::HeaderMap;
use sandproof_guest_runtime::accounts::Account;
use sandproof_guest_runtime::file_protocol::Request;

use crate::execution::{expand_path, selected_user_with_limits};
use crate::{AccountPool, AccountSource, AuthPermit, ContextError};

#[cfg(test)]
mod tests;

/// Per-path bounds include the resolved path. A Move retains at most two.
/// These are explicit component budgets, not a frozen production policy.
#[derive(Clone, Copy, Debug)]
pub struct FileContextLimits {
    pub path_bytes: usize,
    pub username_bytes: usize,
    pub authorization_bytes: usize,
}

#[derive(Clone)]
pub struct FileSettings {
    pub daemon_workdir: String,
    pub limits: FileContextLimits,
}

impl FileSettings {
    fn validate(&self) -> Result<(), ContextError> {
        if self.limits.path_bytes == 0
            || self.limits.path_bytes > isize::MAX as usize / 2
            || self.limits.username_bytes == 0
            || self.limits.authorization_bytes == 0
            || !self.daemon_workdir.starts_with('/')
            || self.daemon_workdir.contains('\0')
            || self.daemon_workdir.len() > self.limits.path_bytes
        {
            return Err(ContextError::InvalidArgument);
        }
        Ok(())
    }
}

/// Intended ownership for created objects, not credentials to install on the
/// filesystem worker. Fixed envd file I/O runs with daemon privileges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileOwner {
    pub username: String,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileContext {
    pub owner: FileOwner,
    pub request: Request,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpFileContext {
    pub owner: FileOwner,
    pub path: String,
}

/// One account/default snapshot for a sequential HTTP upload. The dispatcher
/// must keep the exact original authorization alive; this is not a credential.
pub struct HttpFileSelection {
    account: Account,
    workdir: Option<String>,
    settings: FileSettings,
}

impl HttpFileSelection {
    pub(crate) fn check_input(&self, path: &str) -> Result<(), ContextError> {
        check_path_size(path, &self.settings)
    }

    /// Resolve an already decoded full filename (not its basename).
    /// # Errors
    /// Invalid or oversized paths, before any file effect.
    pub fn resolve(&self, path: &str) -> Result<HttpFileContext, ContextError> {
        Ok(HttpFileContext {
            owner: FileOwner {
                username: self.account.name.clone(),
                uid: self.account.credentials.uid,
                gid: self.account.credentials.gid,
            },
            path: resolve_file_path(path, self.workdir.as_deref(), &self.account, &self.settings)?,
        })
    }
}

impl AuthPermit {
    /// RPC files use Basic username, with the same fallback as commands. Both
    /// Move paths share one lookup/snapshot; an invalid second path never
    /// dispatches the first. No rename/parent creation happens in this method.
    /// # Errors
    /// Invalid/budgeted context, account lookup failure and path expansion error.
    pub async fn resolve_file_rpc<S: AccountSource>(
        &self,
        request: &Request,
        headers: &HeaderMap,
        accounts: &AccountPool<S>,
        settings: &FileSettings,
    ) -> Result<FileContext, ContextError> {
        settings.validate()?;
        match request {
            Request::Stat(path)
            | Request::MakeDir(path)
            | Request::Remove(path)
            | Request::ListDir { path, .. } => check_path_size(path, settings)?,
            Request::Move {
                source,
                destination,
            } => {
                check_path_size(source, settings)?;
                check_path_size(destination, settings)?;
            }
        }
        let name = selected_user_with_limits(
            headers,
            &self.defaults().user,
            settings.limits.username_bytes,
            settings.limits.authorization_bytes,
        )?;
        let account = file_account(&name, accounts, settings).await?;
        let path = |value: &str| self.file_path(value, &account, settings);
        let request = match request {
            Request::Stat(value) => Request::Stat(path(value)?),
            Request::MakeDir(value) => Request::MakeDir(path(value)?),
            Request::Remove(value) => Request::Remove(path(value)?),
            Request::ListDir { path: value, depth } => Request::ListDir {
                path: path(value)?,
                depth: *depth,
            },
            Request::Move {
                source,
                destination,
            } => Request::Move {
                source: path(source)?,
                destination: path(destination)?,
            },
        };
        Ok(FileContext {
            owner: owner(account),
            request,
        })
    }

    /// GET/POST /files use the decoded `username` query, never Basic user or
    /// a `user` query alias. Absent username selects init defaults; explicit
    /// empty stays invalid. Raw-upload path requirements belong to its route.
    /// # Errors
    /// Same bounded account/path failures as RPC resolution. No file is opened.
    pub async fn resolve_http_file<S: AccountSource>(
        &self,
        username: Option<&str>,
        path: Option<&str>,
        accounts: &AccountPool<S>,
        settings: &FileSettings,
    ) -> Result<HttpFileContext, ContextError> {
        settings.validate()?;
        let path = path.unwrap_or("");
        check_path_size(path, settings)?;
        self.resolve_http_files(username, accounts, settings)
            .await?
            .resolve(path)
    }

    /// Resolve the account once for all files in one HTTP request. Initialization
    /// must remain blocked by the original permit until actual writes settle.
    /// # Errors
    /// Invalid settings, unknown/invalid account or query budget/timeout.
    pub async fn resolve_http_files<S: AccountSource>(
        &self,
        username: Option<&str>,
        accounts: &AccountPool<S>,
        settings: &FileSettings,
    ) -> Result<HttpFileSelection, ContextError> {
        settings.validate()?;
        let name = username.unwrap_or(&self.defaults().user);
        let account = file_account(name, accounts, settings).await?;
        Ok(HttpFileSelection {
            account,
            workdir: self.defaults().workdir.clone(),
            settings: settings.clone(),
        })
    }

    fn file_path(
        &self,
        path: &str,
        account: &Account,
        settings: &FileSettings,
    ) -> Result<String, ContextError> {
        resolve_file_path(path, self.defaults().workdir.as_deref(), account, settings)
    }
}

fn resolve_file_path(
    path: &str,
    workdir: Option<&str>,
    account: &Account,
    settings: &FileSettings,
) -> Result<String, ContextError> {
    let path = if path.is_empty() {
        workdir.unwrap_or("")
    } else {
        path
    };
    check_path_size(path, settings)?;
    // Keep absolute spelling, relative lexical cleaning and ~ expansion.
    // Do not resolve symlinks or pre-check filesystem existence. NUL also
    // survives here: fixed envd rejects it at the operation's I/O boundary,
    // not as an ExpandAndResolve error. No CString/syscall occurs here.
    let resolved = expand_path(path, &account.home, &settings.daemon_workdir)?;
    check_path_size(&resolved, settings)?;
    Ok(resolved)
}

fn check_path_size(path: &str, settings: &FileSettings) -> Result<(), ContextError> {
    if path.len() > settings.limits.path_bytes {
        return Err(ContextError::ResourceExhausted);
    }
    Ok(())
}

async fn file_account<S: AccountSource>(
    name: &str,
    accounts: &AccountPool<S>,
    settings: &FileSettings,
) -> Result<Account, ContextError> {
    if name.is_empty() || name.contains(['\0', '\n', '\r', ':']) {
        return Err(ContextError::Unauthenticated);
    }
    if name.len() > settings.limits.username_bytes {
        return Err(ContextError::ResourceExhausted);
    }
    let account = accounts.lookup(name.to_owned()).await?;
    if account.name != name
        || account.home.contains('\0')
        || account.credentials.uid == u32::MAX
        || account.credentials.gid == u32::MAX
    {
        return Err(ContextError::Unavailable);
    }
    // Source allocation is bounded by AccountPool's source contract. Check
    // before joining so a long home cannot allocate an unbudgeted result.
    check_path_size(&account.home, settings)?;
    Ok(account)
}

fn owner(account: Account) -> FileOwner {
    FileOwner {
        username: account.name,
        uid: account.credentials.uid,
        gid: account.credentials.gid,
    }
}
