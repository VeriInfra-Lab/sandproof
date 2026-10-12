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
    reconcile::{audit_host_inventory, recover_before_serving},
};
use cache::TemplateCache;
use configuration::HostConfig;
use sandproof_api::admission::RequestAdmission;
use sandproof_controller_service::{
    ControllerJournal, NeverCrash, application::ControllerApplication,
};
use sandproof_host_service::{DriverConfig, HostFacade};
use sandproof_internal::{InternalServer, Limits, ServerTls, read_protected};
use sandproof_node::{LinuxWorker, NodeHost};
use sandproof_protocol::internal::NodeIdentity;
use std::{error::Error, io::Write, net::SocketAddr, path::Path, sync::Arc};
use tokio::sync::{oneshot, watch};

#[derive(serde::Serialize)]
struct HostAuditView<'a> {
    schema_version: u16,
    versions: sandproof_protocol::internal::ContractVersions,
    #[serde(flatten)]
    view: &'a sandproof_controller_service::InternalAudit,
}

const MAX_CONFIG_BYTES: usize = 16 * 1024 * 1024;

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
    let mut journal = open_journal(
        config.native.database.clone(),
        &config.native.namespace,
        &contract,
    )?;
    audit_host_inventory(&node, &journal)?;
    journal.initialize_internal_inbox(config.inbox, &mut NeverCrash)?;
    Ok(())
}

/// Original verified database view plus the original offline node lease/layout
/// validation. This command does not recover, expire, checkpoint or establish
/// complete physical ownership; the actual per-resource observer owns that.
pub(crate) fn audit(path: &Path) -> Result<(), Box<dyn Error>> {
    let config = read_configuration(path)?;
    let (contract, _) = config.validate_static()?;
    validate_config(&config.native)?;
    let view = ControllerJournal::audit_complete(
        &config.native.database,
        &config.native.namespace,
        &contract.bounded,
        &contract.maintenance,
        &config.inbox,
    )?;
    let observation =
        LinuxWorker::inspect_existing(node_configuration(&config.native, contract.bounded))?;
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
        NodeHost::new(node, physical, std::env::current_exe()?),
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
    ControllerJournal::audit_complete(
        &config.native.database,
        &config.native.namespace,
        &contract.bounded,
        &contract.maintenance,
        &config.inbox,
    )?;
    let tls = config.internal.tls.load()?;
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
    recover_before_serving(&application)?;
    application.check_internal_authority(&config.inbox.node)?;
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
        retention_slots: config.inbox.max_sandboxes,
    };
    Ok(ActivatedHost {
        receiver,
        templates,
        driver,
        node: config.inbox.node,
        bind: config.internal.bind,
        tls,
        limits,
    })
}

/// Internal HTTPS and autonomous recovery share the same original application
/// and non-waiting work gate. Service shutdown is not a VM-destroy operation.
pub(crate) async fn serve(path: &Path) -> Result<(), Box<dyn Error>> {
    let host = assemble(path)?;
    let lifecycle = host.receiver.facade.lifecycle();
    let server = InternalServer::new(host.tls, host.node, host.limits, host.receiver);
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
