//! Default-account failures observed by Start, after JSON decoding. Unlike
//! explicit Basic middleware, these use the negotiated Connect stream sender.
use super::{ResponseFormat, reply};
use crate::{
    command_requests::Error as RequestError, owned_body::Owner, upload_response::UnknownUser,
};
use axum::response::Response;
use std::{fmt, sync::Arc};
use tokio::time::Instant;

#[cfg(test)]
mod tests;

pub(crate) enum Rejection {
    Missing,
    Unknown(UnknownUser),
}
struct Source {
    rejection: Option<Rejection>,
    format: Option<ResponseFormat>,
    request_error: RequestError,
    deadline: Instant,
    _owner: Owner,
}
pub struct Failure {
    error: reply::Error,
    source: Arc<Source>,
}
impl Failure {
    #[must_use]
    pub const fn error(&self) -> reply::Error {
        self.error
    }
    #[must_use]
    pub fn request_error(&self) -> RequestError {
        self.source.request_error
    }
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.source.deadline
    }
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandDefaultAccountReplyFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

#[cfg(target_os = "linux")]
impl crate::command_requests::Failure {
    /// Emit only an observed default-account failure from the original selected
    /// Start. Explicit Basic failures use their separate middleware response.
    /// # Errors
    /// Missing selection/observation, limits, expiry or encoder loss, retaining
    /// original request classification, absolute deadline and capacity.
    pub async fn into_default_account_response(
        mut self,
        limits: reply::Limits,
    ) -> Result<Response, Failure> {
        let rejection =
            if self.error() == RequestError::Context(crate::ContextError::Unauthenticated) {
                self.default_account.take()
            } else {
                None
            };
        let source = Source {
            rejection,
            format: self.response,
            request_error: self.error(),
            deadline: self.deadline(),
            _owner: Arc::new(self),
        };
        send(source, limits).await
    }
}
async fn send(source: Source, limits: reply::Limits) -> Result<Response, Failure> {
    let source = Arc::new(source);
    let result = async {
        let format = source.format.ok_or(reply::Error::UnclassifiedFault)?;
        if source.rejection.is_none() {
            return Err(reply::Error::UnclassifiedFault);
        }
        let worker = source.clone();
        reply::send_stream_error(
            format,
            limits,
            source.deadline,
            source.clone(),
            move |limit| {
                let rejection = worker
                    .rejection
                    .as_ref()
                    .ok_or(reply::Error::UnclassifiedFault)?;
                crate::go_json::bounded(limit as u64, |put| {
                    put(br#"{"error":{"code":"unauthenticated","message":""#)?;
                    match rejection {
                        Rejection::Missing => put(b"no user specified")?,
                        Rejection::Unknown(user) => {
                            put(b"invalid default user: '")?;
                            crate::go_json::string_content(user.name().as_bytes(), put)?;
                            put(b"'")?;
                        }
                    }
                    put(br#""}}"#)
                })
                .map_err(|_| reply::Error::ResponseLimit)
            },
        )
        .await
    }
    .await;
    result.map_err(|error| Failure { error, source })
}
