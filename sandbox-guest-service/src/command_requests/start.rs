//! Authenticated, untagged Start -> original gated worker -> durable PID record.
//! HTTP negotiation, public failures and tag policy remain outside this component.
use std::{path::Path, sync::Arc};

use sandproof_guest_runtime::{
    LaunchSpec, ProcessLimits,
    pid_claims::Binding,
    protocol::{Event, StartRequest, encode_event},
};
use tokio::time::timeout_at;

use super::{
    Authenticated, Decoded, Error, Failure, Instant, Ready, Reply, Request, check_deadline,
};
use crate::{
    AccountPool, AccountSource, ContextError, ExecutionSettings,
    command::{
        AdmissionLimits, CommandBodyError, CommandBodyLimits, CommandOwners, ExecutionDeadline,
        PidRegistrar, PidRegistry, PreparationRequest, PreparationSpec,
    },
    execution::{selected_user_with_limits, validate_command_settings},
};

/// Trusted assembly for one command in one VM/session. These are never decoded
/// from client JSON. The caller supplies a fresh command ID in this session,
/// the original VM's registry/registrar and fixed private worker paths.
pub struct StartPreparation<'a> {
    pub owners: &'a CommandOwners,
    pub registry: &'a PidRegistry,
    pub registrar: &'a PidRegistrar,
    pub binding: Binding,
    pub session: u64,
    pub command: u64,
    pub worker: &'a Path,
    pub private_directory: &'a Path,
    pub process_limits: ProcessLimits,
    pub admission_limits: AdmissionLimits,
}

struct Resolved {
    authenticated: Authenticated,
    request: StartRequest,
    launch: LaunchSpec,
    execution: ExecutionDeadline,
}

impl Ready {
    /// Start only the original decoded, untagged request. Explicit Basic reuses
    /// the preflight account; default lookup retains the same auth/request until
    /// its actual blocking worker exits. Registry capacity and the first response
    /// frame budget are checked before preparing any child. The original child
    /// stays gated until its durable PID claim and active record are installed.
    ///
    /// This is an uninstalled PID-only component, not a public tag policy. A tag
    /// (including an empty one) is unsupported pending that separate assembly.
    /// Cancellation before acceptance stops preparation; afterwards the original
    /// registry owns execution and losing this reply never starts another child.
    /// # Errors
    /// Original expiry, context/worker/storage/registry failure, unsupported tag
    /// or response limits. Failed requests may have burned a PID or, after
    /// acceptance, started a command. Errors are not cleanup receipts.
    pub async fn start_pid<S: AccountSource>(
        self,
        accounts: &AccountPool<S>,
        settings: &ExecutionSettings,
        preparation: StartPreparation<'_>,
        response: CommandBodyLimits,
    ) -> Result<Reply, Failure> {
        let deadline = self.deadline();
        let format = self.response;
        let mut default_account = None;
        let owner = self.decoded.authenticated.admitted.owner.clone();
        let result = async {
            let resolved = self
                .resolve_start(accounts, settings, &mut default_account)
                .await?;
            response.validate(Instant::now()).map_err(Error::Response)?;
            // Later actual PID width cannot invalidate the initial frame budget.
            encode_event(Event::Start(u32::MAX), response.message_bytes)
                .map_err(|_| Error::Response(CommandBodyError::ResourceExhausted))?;
            let reservation = preparation
                .registry
                .reserve_optional(resolved.request.process.as_ref(), None)
                .map_err(Error::Registry)?;
            check_deadline(deadline)?;
            let pending = preparation
                .owners
                .prepare_command_admitted(
                    PreparationSpec {
                        worker: preparation.worker.into(),
                        private_directory: preparation.private_directory.into(),
                        session: preparation.session,
                        command: preparation.command,
                        launch: resolved.launch,
                        process_limits: preparation.process_limits,
                    },
                    resolved.authenticated.auth,
                    resolved.execution,
                    preparation.admission_limits,
                    PreparationRequest {
                        deadline,
                        owner: owner.clone(),
                    },
                )
                .map_err(Error::Command)?;
            let ticket = pending
                .register_pid(preparation.registrar.clone(), preparation.binding)
                .await
                .map_err(Error::Registration)?;
            check_deadline(deadline)?;
            let start = reservation.publish(ticket).map_err(Error::Registry)?;
            let (_command, subscription) = start.started().await.map_err(Error::Command)?;
            check_deadline(deadline)?;
            let body = subscription
                .into_body(owner.clone(), response, deadline)
                .map_err(Error::Response)?;
            Ok(Reply::streaming(body, deadline, format, owner.clone()))
        };
        timeout_at(deadline, result)
            .await
            .unwrap_or(Err(Error::DeadlineExceeded))
            .map_err(|error| {
                let mut failure = Failure::new(error, deadline, owner);
                failure.default_account = default_account;
                failure.response = format;
                failure
            })
    }

    async fn resolve_start<S: AccountSource>(
        self,
        accounts: &AccountPool<S>,
        settings: &ExecutionSettings,
        observed: &mut Option<super::http::default_account::Rejection>,
    ) -> Result<Resolved, Error> {
        let deadline = self.deadline();
        check_deadline(deadline)?;
        let Decoded {
            request,
            authenticated,
            ..
        } = self.decoded;
        let Request::Start(request) = request else {
            return Err(Error::UnsupportedOperation);
        };
        if request.tag.is_some() {
            return Err(Error::UnsupportedOperation);
        }
        validate_command_settings(settings).map_err(Error::Context)?;
        let authenticated = Arc::new(authenticated);
        let fallback = &authenticated.authorization().defaults().user;
        let name = selected_user_with_limits(
            authenticated.headers(),
            fallback,
            settings.limits.username_bytes,
            settings.limits.authorization_bytes,
        )
        .map_err(|error| {
            if self.account.is_none()
                && error == ContextError::Unauthenticated
                && fallback.len() <= settings.limits.username_bytes
                && (fallback.is_empty() || fallback.contains(['\0', '\n', '\r', ':']))
            {
                *observed = Some(if fallback.is_empty() {
                    super::http::default_account::Rejection::Missing
                } else {
                    super::http::default_account::Rejection::Unknown(
                        crate::upload_response::UnknownUser::new(fallback),
                    )
                });
            }
            Error::Context(error)
        })?;
        let account = match self.account {
            Some(account) => account,
            None => accounts
                .lookup_admitted(name.clone(), deadline, authenticated.clone())
                .await
                .map_err(|error| {
                    if error == ContextError::Unauthenticated {
                        *observed = Some(super::http::default_account::Rejection::Unknown(
                            crate::upload_response::UnknownUser::new(&name),
                        ));
                    }
                    Error::Context(error)
                })?,
        };
        if account.name != name {
            return Err(Error::Context(ContextError::Unavailable));
        }
        check_deadline(deadline)?;
        let authenticated = Arc::try_unwrap(authenticated).map_err(|_| Error::Unavailable)?;
        // The fixed Start execution clock begins after account resolution;
        // HTTP/response expiry was captured earlier and remains independent.
        let execution = ExecutionDeadline::from_headers(authenticated.headers(), Instant::now())
            .map_err(Error::Command)?;
        let launch = authenticated
            .authorization()
            .compose_command(&request, account, settings)
            .map_err(Error::Context)?;
        check_deadline(deadline)?;
        Ok(Resolved {
            authenticated,
            request,
            launch,
            execution,
        })
    }
}
