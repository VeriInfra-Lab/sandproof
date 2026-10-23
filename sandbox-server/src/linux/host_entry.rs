//! Sole protected production host composition, reusing the original authority.
mod cache;
mod configuration;
mod receiver;
mod tls;

use super::{
    config::{application_config, node_configuration},
    preflight::{
        open_journal, preflight_journal_configuration, private_root_path, validate_config,
        validate_physical_storage,
    },
    reconcile::audit_host_inventory,
};
use cache::TemplateCache;
use configuration::HostConfig;
use sandproof_api::admission::RequestAdmission;
use sandproof_controller_service::{
    ControllerJournal, NeverCrash, application::ControllerApplication,
};
use sandproof_host_service::{DriverConfig, HostFacade};
use sandproof_internal::{InternalServer, Limits, ServerTls, read_protected};
use sandproof_node::{LinuxWorker, NodeHost, ProtectedHostEpochSource};
use sandproof_protocol::internal::NodeIdentity;
use std::{error::Error, io::Write, net::SocketAddr, path::Path, sync::Arc};
use tokio::sync::{oneshot, watch};

#[derive(serde::Serialize)]
struct HostAuditView<'a> {
    schema_version: u16,
    versions: sandproof_protocol::internal::ContractVersions,
    #[serde(flatten)]
    view: &'a sandproof_controller_service::LifecycleAudit,
}

const MAX_CONFIG_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
struct DueWorkFailure(sandproof_controller_service::application::ApplicationError);
impl std::fmt::Display for DueWorkFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // ApplicationError is a closed payload-free category. Preserve that
        // exact category instead of treating every startup fault as Busy.
        write!(formatter, "host due-work admission failed: {:?}", self.0)
    }
}
impl Error for DueWorkFailure {}

/// Explicit operator action only. It creates new protected material, never
/// overwrites a registration and does not open or initialize the journal.
pub(crate) fn provision_bootstrap(path: &Path) -> Result<(), Box<dyn Error>> {
    let config = read_configuration(path)?;
    let (contract, _) = config.validate_static()?;
    let node = node_configuration(&config.native, contract.bounded);
    let source = ProtectedHostEpochSource::at(config.protected_bootstrap_path)?;
    let registration = source.provision(&config.inbox.node, &node)?;
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, &registration)?;
    writeln!(output)?;
    Ok(())
}

fn read_configuration(path: &Path) -> Result<HostConfig, Box<dyn Error>> {
    private_root_path(&std::env::current_exe()?)?;
    Ok(serde_json::from_slice(&read_protected(
        path,
        0,
        MAX_CONFIG_BYTES,
    )?)?)
}

/// Non-mutating online observation. No SQLite/VM lease, initialization,
/// recovery or local template material is opened by this command.
pub(crate) async fn observe_center(path: &Path) -> Result<(), Box<dyn Error>> {
    let config = read_configuration(path)?;
    config.validate_static()?;
    let view = config
        .artifact_source
        .client(config.inbox.node)?
        .readiness()
        .await?;
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, &view)?;
    writeln!(output)?;
    Ok(())
}

/// Explicit provisioning; neither normal service startup nor audit may create
/// missing authority or replace an older experimental format.
pub(crate) fn initialize(path: &Path) -> Result<(), Box<dyn Error>> {
    let config = read_configuration(path)?;
    let (contract, _) = config.validate_static()?;
    validate_config(&config.native)?;
    preflight_journal_configuration(&config.native, &contract)?;
    config.internal.tls.load()?;
    let node_config = node_configuration(&config.native, contract.bounded.clone());
    let _physical = validate_physical_storage(&config.native, &node_config)?;
    let node = LinuxWorker::new(node_config)?;
    let source = ProtectedHostEpochSource::at(config.protected_bootstrap_path.clone())?;
    let registration = node.host_epoch_registration(&source, &config.inbox.node)?;
    if config.inbox.host_epoch.as_ref() != Some(&registration) {
        return Err("explicit host bootstrap registration does not match protected material".into());
    }
    let mut journal = open_journal(
        config.native.database.clone(),
        &config.native.namespace,
        &contract,
    )?;
    audit_host_inventory(&node, &journal)?;
    journal.initialize_lifecycle_inbox(config.inbox, config.source_observation, &mut NeverCrash)?;
    Ok(())
}

/// Original verified database view plus the original offline node lease/layout
/// validation. This command does not recover, expire, checkpoint or establish
/// complete physical ownership; the actual per-resource observer owns that.
pub(crate) fn audit(path: &Path) -> Result<(), Box<dyn Error>> {
    let config = read_configuration(path)?;
    let (contract, _) = config.validate_static()?;
    validate_config(&config.native)?;
    let view = ControllerJournal::audit_lifecycle_complete(
        &config.native.database,
        &config.native.namespace,
        &contract.bounded,
        &contract.maintenance,
        &config.inbox,
    )?;
    if view.source_observation != config.source_observation {
        return Err("stored source observation limits do not match protected configuration".into());
    }
    let observation =
        LinuxWorker::inspect_existing(node_configuration(&config.native, contract.bounded))?;
    let source = ProtectedHostEpochSource::at(config.protected_bootstrap_path)?;
    if config.inbox.host_epoch.as_ref() != Some(&observation.host_epoch_registration(&source, &config.inbox.node)?) {
        return Err("stored host bootstrap registration does not match protected material".into());
    }
    observation.host_slot_inventory()?;
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(
        &mut output,
        &HostAuditView {
            schema_version: 2,
            versions: sandproof_protocol::internal::ContractVersions::current(),
            view: &view,
        },
    )?;
    writeln!(output)?;
    Ok(())
}

struct ActivatedHost {
    receiver: receiver::RegisteredHost,
    templates: Arc<TemplateCache>,
    driver: DriverConfig,
    node: NodeIdentity,
    bind: SocketAddr,
    tls: ServerTls,
    limits: Limits,
    center_peer: sandproof_internal::lifecycle::CenterPeerRegistration,
}

fn application(
    config: &HostConfig,
    journal: ControllerJournal,
    node: LinuxWorker,
    physical: sandproof_node::PhysicalProfileLease,
    disk_bytes: u64,
) -> Result<Arc<ControllerApplication<NodeHost>>, Box<dyn Error>> {
    let managed = application_config(&config.native, disk_bytes)?;
    let application = ControllerApplication::with_templates(
        journal,
        NodeHost::new(node, physical, std::env::current_exe()?)
            .with_host_epoch_source(ProtectedHostEpochSource::at(config.protected_bootstrap_path.clone())?),
        managed,
        config
            .native
            .templates
            .clone()
            .ok_or("missing original template catalog")?,
    )
    .map_err(|_| "invalid original templates")?;
    Ok(Arc::new(application))
}

fn assemble(path: &Path) -> Result<ActivatedHost, Box<dyn Error>> {
    let config = read_configuration(path)?;
    let (contract, driver) = config.validate_static()?;
    let disk_bytes = validate_config(&config.native)?;
    preflight_journal_configuration(&config.native, &contract)?;
    // The read-only original snapshot must find CURRENT authority and the
    // exact explicitly provisioned inbox BEFORE any activating constructor.
    let audit = ControllerJournal::audit_lifecycle_complete(
        &config.native.database,
        &config.native.namespace,
        &contract.bounded,
        &contract.maintenance,
        &config.inbox,
    )?;
    if audit.source_observation != config.source_observation {
        return Err("stored source observation limits do not match protected configuration".into());
    }
    let tls = config.internal.tls.load()?;
    let center_peer = config.internal.tls.center_registration(config.inbox.center.clone(), config.inbox.node.clone())?;
    let limits = config.internal.limits()?;
    let node_config = node_configuration(&config.native, contract.bounded.clone());
    let physical = validate_physical_storage(&config.native, &node_config)?;
    let node = LinuxWorker::new(node_config)?;
    let journal = open_journal(
        config.native.database.clone(),
        &config.native.namespace,
        &contract,
    )?;
    if journal.build_retry_config()?.is_some() || journal.has_template_registry()? {
        return Err("host cannot adopt a removed local build or publication authority".into());
    }
    audit_host_inventory(&node, &journal)?;
    let application = application(&config, journal, node, physical, disk_bytes)?;
    // Re-establish against the registration already stored by initialize.
    // Reading today's file is never permission to replace that registration.
    application.check_lifecycle_authority(&config.inbox)?;
    application.check_lifecycle_source_observations(&config.source_observation)?;
    application.establish_lifecycle_host_epoch()?;
    // Register due termination/revocation in short transactions only. Slow
    // recovery belongs to the bounded per-session driver after assembly; one
    // unknown or large VM must not delay every other session's control path.
    application.accept_due_work().map_err(DueWorkFailure)?;
    let facade = HostFacade::new(
        Arc::clone(&application),
        contract.transfers.clone(),
        RequestAdmission::new(config.data_requests(&contract))?,
    )?;
    let templates = Arc::new(TemplateCache::open(
        config.template_cache.clone(),
        config.inbox.node.clone(),
        config.artifact_source.client(config.inbox.node.clone())?,
        facade.lifecycle(),
    )?);
    let receiver = receiver::RegisteredHost {
        facade,
        templates: Arc::clone(&templates),
        compute: driver.compute,
        node: config.inbox.node.clone(),
        retention_slots: config.inbox.max_sessions,
    };
    Ok(ActivatedHost {
        receiver,
        templates,
        driver,
        node: config.inbox.node,
        bind: config.internal.bind,
        tls,
        limits,
        center_peer,
    })
}

/// Internal HTTPS and autonomous recovery share the same original application
/// and non-waiting work gate. Service shutdown is not a VM-destroy operation.
pub(crate) async fn serve(path: &Path) -> Result<(), Box<dyn Error>> {
    let host = assemble(path)?;
    let lifecycle = host.receiver.facade.lifecycle();
    let server = InternalServer::new(host.tls, host.node, host.limits, host.receiver)
        .with_center_peer(host.center_peer)?;
    let listener = tokio::net::TcpListener::bind(host.bind).await?;
    let (stop_server, server_shutdown) = oneshot::channel();
    let (stop_driver, driver_shutdown) = watch::channel(false);
    let server_work = server.serve(listener, server_shutdown);
    let driver_work =
        lifecycle.run_driver(host.driver, Arc::clone(&host.templates), driver_shutdown);
    tokio::pin!(server_work, driver_work);
    let (result, finished) = tokio::select! {
        result = &mut server_work => (result.map_err(|error| Box::new(error) as Box<dyn Error>), 0),
        result = &mut driver_work => (result.map_err(|_| "original host driver halted".into()), 1),
        result = shutdown_signal() => (result, 2),
    };
    let _ = stop_driver.send(true);
    let _ = stop_server.send(());
    // Join remaining work; no detached second scanner. Accepted native work
    // retains its original permit and deadline until the actual helper exits.
    let server_result = if finished == 0 {
        Ok(())
    } else {
        server_work
            .await
            .map_err(|error| Box::new(error) as Box<dyn Error>)
    };
    let driver_result = if finished == 1 {
        Ok(())
    } else {
        driver_work
            .await
            .map_err(|_| "original host driver halted".into())
    };
    let cache_result = host.templates.drain().await;
    // A sibling error cannot skip actual cache-worker drain or its lease.
    result
        .and(server_result)
        .and(driver_result)
        .and(cache_result)
}

async fn shutdown_signal() -> Result<(), Box<dyn Error>> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        _ = terminate.recv() => {},
    }
    Ok(())
}

#[cfg(test)]
mod tests;
