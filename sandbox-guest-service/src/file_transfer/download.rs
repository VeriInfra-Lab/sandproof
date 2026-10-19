use super::{FileStreams, IdentityDownloadError, TransferIoError};
use crate::{
    AccountPool, AccountSource, AuthPermit, ContextError,
    file_download::{
        ContentType, Download, DownloadError, IdentityLimits, IdentityRequest, Limits,
        NegotiationLimits,
    },
    files::{FileSettings, HttpFileContext},
    owned_body::Owner,
};
use sandproof_guest_runtime::{
    file_download::date::{LocalZone, Timestamp},
    filesystem::transfer::{DownloadPath, DownloadPathError},
};
use std::{os::unix::fs::MetadataExt, sync::Arc};

#[cfg(test)]
mod tests;

/// Query values are already decoded; absent/empty path uses the authenticated
/// default workdir. MIME detection uses the original query, not the resolved path.
pub struct DownloadRequest {
    pub username: Option<String>,
    pub path: Option<String>,
    pub http: IdentityRequest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadRequestError {
    Authorization(crate::file_authorization::Error),
    Admission(super::FileStreamError),
    Context(ContextError),
    Download(DownloadError),
}
impl From<IdentityDownloadError> for DownloadRequestError {
    fn from(error: IdentityDownloadError) -> Self {
        match error {
            IdentityDownloadError::Admission(error) => Self::Admission(error),
            IdentityDownloadError::Download(error) => Self::Download(error),
        }
    }
}

impl FileStreams {
    /// One admission and deadline from account lookup through final response
    /// byte release. A cancelled/timed-out account lookup retains the original
    /// authorization and transfer slot until its blocking call actually exits.
    /// Caller authenticates and decodes the query; this does not install routes.
    /// # Errors
    /// Admission, account/path resolution, unmapped I/O, budgets or original
    /// deadline. Observed unknown accounts become the fixed bounded 401;
    /// other context failures remain explicit, without invented diagnostics.
    pub async fn download_request<S: AccountSource, Z: LocalZone + Send + 'static>(
        &self,
        auth: AuthPermit,
        request: DownloadRequest,
        accounts: &AccountPool<S>,
        settings: &FileSettings,
        limits: DownloadLimits,
        local: Z,
    ) -> Result<Download, DownloadRequestError> {
        self.download_request_before((auth, request), accounts, settings, limits, local, None)
            .await
    }

    /// Consume only a read capability; path and username come exclusively from
    /// its signed query. Authentication time remains in the original deadline.
    /// # Errors
    /// Wrong operation/expiry or the same admitted download errors.
    pub async fn download_authorized<S: AccountSource, Z: LocalZone + Send + 'static>(
        &self,
        authorization: crate::file_authorization::Authorization,
        http: IdentityRequest,
        accounts: &AccountPool<S>,
        settings: &FileSettings,
        limits: DownloadLimits,
        local: Z,
    ) -> Result<Download, DownloadRequestError> {
        use crate::file_authorization::{Error, Operation};
        if http.method != axum::http::Method::GET && http.method != axum::http::Method::HEAD {
            return Err(DownloadRequestError::Authorization(Error::WrongOperation));
        }
        let (auth, query, origin) = authorization
            .take(Operation::Read)
            .map_err(DownloadRequestError::Authorization)?;
        self.download_request_before(
            (
                auth,
                DownloadRequest {
                    path: query.path,
                    username: query.username,
                    http,
                },
            ),
            accounts,
            settings,
            limits,
            local,
            Some(origin),
        )
        .await
    }

    async fn download_request_before<S: AccountSource, Z: LocalZone + Send + 'static>(
        &self,
        start: (AuthPermit, DownloadRequest),
        accounts: &AccountPool<S>,
        settings: &FileSettings,
        limits: DownloadLimits,
        local: Z,
        original: Option<crate::file_authorization::Origin>,
    ) -> Result<Download, DownloadRequestError> {
        let (auth, mut request) = start;
        let (lease, deadline) = self
            .admit_before(original)
            .map_err(DownloadRequestError::Admission)?;
        let auth = Arc::new(auth);
        let owner: Owner = Arc::new((Arc::clone(&auth), lease));
        let identity = IdentityLimits {
            plan: limits.response,
            chunk_bytes: self.limits.chunk_bytes,
            file_bytes: self.limits.total_bytes,
            path_bytes: self.limits.file.path_bytes,
        };
        let selection = match auth
            .resolve_http_files_admitted(
                request.username.as_deref(),
                accounts,
                settings,
                deadline,
                Arc::clone(&owner),
            )
            .await
        {
            Ok(selection) => selection,
            Err(crate::files::SelectionError::Context(error)) => {
                return Err(DownloadRequestError::Context(error));
            }
            Err(crate::files::SelectionError::UnknownUser(error)) => {
                return Download::unknown_user(error, request.http, identity, deadline, owner)
                    .await
                    .map_err(DownloadRequestError::Download);
            }
        };
        let path = request.path.as_deref().unwrap_or("");
        let context = match selection.resolve_detailed(path) {
            Ok(context) => context,
            Err(crate::files::PathError::Context(error)) => {
                return Err(DownloadRequestError::Context(error));
            }
            Err(crate::files::PathError::Expansion(error)) => {
                return Download::path_expansion(
                    error,
                    request.path.take().unwrap_or_default(),
                    request.http,
                    identity,
                    deadline,
                    owner,
                )
                .await
                .map_err(DownloadRequestError::Download);
            }
        };
        if let ContentType::Detect { original_path, .. } = &mut request.http.content_type {
            *original_path = request.path.take().unwrap_or_default();
        }
        self.download_reserved(context, request.http, limits, local, deadline, owner)
            .await
            .map_err(DownloadRequestError::from)
    }

    /// A single admitted stat -> selection -> open -> streaming request. The
    /// source, gzip/identity worker and response bytes retain original auth.
    /// # Errors
    /// Admission, stat/directory, encoding (with Vary stage), open or preparation.
    /// Fixed post-resolution filesystem/negotiation errors become owned responses.
    /// Authentication/signing and context-resolution errors are not implemented here.
    pub async fn download_negotiated<Z: LocalZone + Send + 'static>(
        &self,
        auth: AuthPermit,
        context: HttpFileContext,
        request: IdentityRequest,
        limits: DownloadLimits,
        local: Z,
    ) -> Result<Download, IdentityDownloadError> {
        let (lease, deadline) = self.admit().map_err(IdentityDownloadError::Admission)?;
        let owner = Arc::new((auth, lease));
        self.download_reserved(context, request, limits, local, deadline, owner)
            .await
    }

    async fn download_reserved<Z: LocalZone + Send + 'static>(
        &self,
        context: HttpFileContext,
        mut request: IdentityRequest,
        limits: DownloadLimits,
        local: Z,
        deadline: tokio::time::Instant,
        owner: Owner,
    ) -> Result<Download, IdentityDownloadError> {
        let transfer = self.limits;
        if context.path.len() > transfer.file.path_bytes {
            return Err(IdentityDownloadError::Download(
                DownloadError::ResourceExhausted,
            ));
        }
        request.disposition_path = Some(context.path.as_bytes().to_vec());
        Download::open_negotiated(
            move || {
                let path =
                    DownloadPath::inspect_detailed(context.path, transfer).map_err(
                        |e| match e {
                            DownloadPathError::Io(Some(errno)) => DownloadError::StatIo(errno),
                            DownloadPathError::Directory => DownloadError::Directory,
                            DownloadPathError::Rejected(TransferIoError::InvalidArgument) => {
                                DownloadError::SourceInvalid
                            }
                            DownloadPathError::Rejected(TransferIoError::ResourceExhausted) => {
                                DownloadError::ResourceExhausted
                            }
                            _ => DownloadError::Stat,
                        },
                    )?;
                let metadata = path.metadata();
                let nanos = u32::try_from(metadata.mtime_nsec())
                    .map_err(|_| DownloadError::SourceInvalid)?;
                let modified = Timestamp::new(metadata.mtime(), nanos)
                    .map_err(|_| DownloadError::SourceInvalid)?;
                Ok((modified, move || {
                    path.open_detailed()
                        .map(|(source, _)| source)
                        .map_err(|e| match e {
                            DownloadPathError::Io(Some(errno)) => DownloadError::OpenIo(errno),
                            _ => DownloadError::Open,
                        })
                }))
            },
            request,
            NegotiationLimits {
                identity: IdentityLimits {
                    plan: limits.response,
                    chunk_bytes: transfer.chunk_bytes,
                    file_bytes: transfer.total_bytes,
                    path_bytes: transfer.file.path_bytes,
                },
                encoding: limits.encoding,
                compressed_bytes: limits.compressed_bytes,
            },
            local,
            deadline,
            owner,
        )
        .await
        .map_err(IdentityDownloadError::Download)
    }
}
#[derive(Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadLimits {
    pub response: Limits,
    pub encoding: crate::file_encoding::Limits,
    pub compressed_bytes: u64,
}
