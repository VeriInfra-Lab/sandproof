use super::{Error, config::Configuration};
use crate::{
    AccountPool, MmdsClient, SystemAccounts,
    command::{CommandOwners, PidRegistrar, PidRegistry},
    command_http::{CommandHttp, ControlExecution, Processes, RequestPools},
    command_requests::CommandRequests,
    file_http::{FileHttp, Settings},
    file_io::FilePool,
    file_requests::FileRequests,
    file_rpc_http::FileRpcHttp,
    file_rpc_requests::FileRpcRequests,
    file_transfer::FileStreams,
    guest_http::{Components, GuestHttp},
    listener::Listener,
    startup::{Opened, PinnedMetadata, Startup},
};
use sandproof_guest_runtime::{
    file_download::local_zone::Snapshot, file_mime::Catalog, pid_claims,
};
use std::{os::unix::ffi::OsStrExt, path::Path, sync::Arc};
use tokio::time::Instant;

pub type Http<M = MmdsClient> = GuestHttp<PinnedMetadata<M>, SystemAccounts, Snapshot>;

/// Validated settings and original bounded pools. Construction creates no
/// listener, lookup, worker, metadata exchange or persistent state. Only this
/// object may consume the settings into the formal assembly below.
pub struct Prepared {
    pub(super) config: Configuration,
    pub(super) startup: Startup<MmdsClient>,
    pub(super) listener: Listener,
    pub(super) worker: sandproof_guest_runtime::artifact::Expected,
    accounts: AccountPool<SystemAccounts>,
    control_execution: Option<ControlExecution<SystemAccounts>>,
    owners: CommandOwners,
    commands: RequestPools,
    files: FileRequests,
    rpc: FileRpcRequests,
    operations: FilePool,
    streams: FileStreams,
}

fn checked<T, E>(value: Result<T, E>) -> Result<T, Error> {
    value.map_err(|_| Error::Configuration)
}

impl Prepared {
    /// Check explicit configuration through the original component validators.
    /// No environment, production resource defaults or process identity is
    /// inferred. Decoded settings alone are not a startup capability.
    /// # Errors
    /// Any unsupported schema, invalid budget/path or inconsistent context.
    pub fn new(config: Configuration) -> Result<Self, Error> {
        let now = Instant::now();
        let runtime = &config.runtime;
        if config.version != crate::build_info::CONFIG_VERSION
            || config.listen.port() == 0
            || runtime.worker_threads == 0
            || runtime.blocking_threads == 0
            || runtime.thread_stack_bytes < 16_384
            || runtime
                .worker_threads
                .checked_add(runtime.blocking_threads)
                .and_then(|n| n.checked_mul(runtime.thread_stack_bytes))
                .is_none_or(|n| n > isize::MAX as usize)
            || runtime.shutdown_timeout.is_zero()
            || now.checked_add(runtime.shutdown_timeout).is_none()
            || config.paths.owner_uid == u32::MAX
            || config.paths.owner_uid != sandproof_guest_runtime::startup_files::current_owner()
            || !(1..=pid_claims::MAX_PID).contains(&config.paths.max_pid)
            || config.files.context.daemon_workdir != config.commands.context.daemon_workdir
        {
            return Err(Error::Configuration);
        }
        let worker = checked(config.worker.validate())?;
        for path in [
            &config.paths.pid_directory,
            &config.paths.worker,
            &config.paths.gate_directory,
            &config.snapshots.timezone,
        ] {
            validate_path(path)?;
        }
        checked(config.initialization.validate())?;
        checked(crate::execution::validate_command_settings(
            &config.commands.context,
        ))?;
        checked(config.commands.process.validate())?;
        checked(config.commands.admission.validate(now))?;
        checked(config.commands.registry.validate_public())?;
        checked(config.commands.response.validate())?;
        checked(crate::file_http::check_settings(
            &config.files.context,
            config.files.upload,
            config.files.download,
            config.files.upload_response,
        ))?;
        checked(config.files.rpc_response.validate())?;
        checked(config.snapshots.timezone_limits.snapshot.validate())?;
        let path_limit = config.snapshots.timezone_limits.path_bytes;
        if path_limit == 0
            || path_limit > isize::MAX as usize / 16
            || config.snapshots.timezone.as_os_str().as_bytes().len() > path_limit
        {
            return Err(Error::Configuration);
        }
        // The original MIME constructor validates both the raw source budget
        // and space for builtins without opening any system source files.
        checked(Catalog::from_sources(
            [None; 2],
            [None; 4],
            config.snapshots.mime,
        ))?;
        let accounts = checked(AccountPool::local(
            config.accounts.source,
            config.accounts.in_flight,
            config.accounts.timeout,
        ))?;
        let owners = checked(CommandOwners::new(config.commands.owners))?;
        let commands = RequestPools {
            ordinary: checked(CommandRequests::new(config.commands.ordinary))?,
            control: checked(CommandRequests::new(config.commands.control))?,
        };
        let files = checked(FileRequests::new(config.files.requests))?;
        let rpc = checked(FileRpcRequests::new(config.files.rpc_requests))?;
        let operations = checked(FilePool::new(
            config.files.operations.in_flight,
            config.files.operations.limits,
            config.files.operations.timeout,
        ))?;
        let streams = checked(FileStreams::new(
            config.files.transfers.in_flight,
            config.files.transfers.limits,
            config.files.transfers.timeout,
        ))?;
        let listener = checked(Listener::new(config.transport))?;
        let startup = checked(Startup::new(
            checked(MmdsClient::firecracker(config.metadata))?,
            config.startup,
        ))?;
        Ok(Self {
            config,
            startup,
            listener,
            worker,
            accounts,
            control_execution: None,
            owners,
            commands,
            files,
            rpc,
            operations,
            streams,
        })
    }

    pub(super) fn with_control_executor(
        mut self,
        blocking: tokio::runtime::Handle,
    ) -> Result<Self, Error> {
        let (accounts, control) = checked(self.accounts.reserve_control())?;
        self.accounts = accounts;
        self.control_execution = Some(ControlExecution {
            blocking,
            accounts: control,
        });
        Ok(self)
    }

    pub(super) fn assemble<M: crate::MetadataSource>(
        self,
        opened: Opened<M>,
        mime: Catalog,
        local: Snapshot,
    ) -> Result<(Http<M>, Listener), Error> {
        let binding = opened.binding();
        let service_session = opened.service_session();
        let (session, claims) = opened.into_parts();
        let config = self.config;
        let registry = checked(PidRegistry::new(
            binding,
            service_session,
            config.commands.registry,
        ))?;
        let commands = checked(CommandHttp::new(
            session.clone(),
            self.commands,
            self.accounts.clone(),
            config.commands.context,
            Processes {
                owners: self.owners,
                registry,
                registrar: PidRegistrar::new(claims),
                binding,
                session: service_session,
                worker: config.paths.worker,
                private_directory: config.paths.gate_directory,
                process_limits: config.commands.process,
                admission_limits: config.commands.admission,
            },
            config.commands.response,
        ))?;
        let commands = match self.control_execution {
            Some(control) => checked(commands.with_control_execution(control))?,
            None => commands,
        };
        let filesystem = checked(FileRpcHttp::new(
            session.clone(),
            self.rpc,
            self.operations,
            self.accounts.clone(),
            config.files.context.clone(),
            config.files.rpc_response,
        ))?;
        let files = checked(FileHttp::new(
            session.clone(),
            self.files,
            self.streams,
            self.accounts,
            Settings {
                files: config.files.context,
                upload: config.files.upload,
                download: config.files.download,
                upload_response: config.files.upload_response,
                mime: Arc::new(mime),
                local: Arc::new(local),
            },
        ))?;
        let http = checked(GuestHttp::new(
            session,
            Components {
                commands,
                files,
                filesystem,
            },
        ))?;
        Ok((http, self.listener))
    }
}

fn validate_path(path: &Path) -> Result<(), Error> {
    if !path.is_absolute()
        || path.as_os_str().as_bytes().contains(&0)
        || path.as_os_str().as_bytes().len() > super::config::CONFIG_BYTES
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        Err(Error::Configuration)
    } else {
        Ok(())
    }
}
