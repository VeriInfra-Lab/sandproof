//! The sole formal-service startup sequence.

use super::{
    config::{ConfigFile, DeploymentContract, node_configuration},
    host::NodeHost,
    preflight::{
        ValidatedDeployment, preflight_journal_configuration, private_root_path,
        validate_physical_storage,
    },
    reconcile::{audit_host_inventory, recover_before_serving, spawn_reconciliation_scanner},
};
use crate::gateway;
use sandproof_api::{ApiCredential, managed::ManagedPort, router};
use sandproof_controller_service::ControllerJournal;
use sandproof_node::LinuxWorker;
use std::{error::Error, fs, path::PathBuf, sync::Arc, time::Duration};

fn open_journal(
    database: PathBuf,
    namespace: &str,
    contract: &DeploymentContract,
) -> Result<ControllerJournal, Box<dyn Error>> {
    // Node construction has already independently refused legacy metadata
    // or resources. The journal now rechecks its own empty state/config in
    // the enable transaction; neither check substitutes for the other.
    let now_ms = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?;
    Ok(ControllerJournal::open_complete(
        database,
        namespace,
        contract.bounded.clone(),
        contract.maintenance.clone(),
        now_ms,
    )?)
}

fn read_configuration() -> Result<ConfigFile, Box<dyn Error>> {
    private_root_path(&std::env::current_exe()?)?;
    let mut args = std::env::args_os();
    let _program = args.next();
    let path = PathBuf::from(
        args.next()
            .ok_or("usage: sandproof-server /absolute/root-only/config.json")?,
    );
    if args.next().is_some() {
        return Err("only one configuration path is accepted".into());
    }
    private_root_path(&path)?;
    if !fs::metadata(&path)?.is_file() {
        return Err("configuration must be a regular file".into());
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

pub async fn run() -> Result<(), Box<dyn Error>> {
    let ValidatedDeployment {
        settings: config,
        contract,
        managed,
    } = ValidatedDeployment::validate(read_configuration()?)?;
    preflight_journal_configuration(&config, &contract)?;
    let node_config = node_configuration(&config, contract.bounded.clone());
    let physical_storage = validate_physical_storage(&config, &node_config)?;
    let node = LinuxWorker::new(node_config)?;
    let journal = open_journal(config.database, &config.namespace, &contract)?;
    if journal
        .build_retry_config()?
        .is_some_and(|stored| Some(stored) != config.build_retry_windows)
    {
        return Err(
            "durable private build retry configuration cannot be disabled or changed".into(),
        );
    }
    if journal.has_template_registry()? && config.template_registry.is_none() {
        return Err("durable template registry cannot be disabled for this journal".into());
    }
    audit_host_inventory(&node, &journal)?;
    let port = if let Some(catalog) = config.templates {
        ManagedPort::with_templates(journal, NodeHost(node, physical_storage), managed, catalog)
            .map_err(|_| "invalid prebuilt template configuration")?
    } else {
        ManagedPort::new(journal, NodeHost(node, physical_storage), managed)
    };
    let port = port.with_internal_build_vms(config.internal_build_vms);
    let port = if let Some(retry) = config.build_retry_windows {
        port.with_build_retry_windows(retry)
            .map_err(|_| "invalid durable private build retry policy")?
    } else {
        port
    };
    let port = Arc::new(if let Some(registry) = config.template_registry {
        port.with_registry(
            sandproof_controller_service::template_registry::CandidateStore::new(registry, 0)?,
        )
        .map_err(|_| "invalid durable template registry configuration")?
    } else {
        port
    });
    recover_before_serving(&port)?;
    let credentials = config
        .credentials
        .into_iter()
        .map(|entry| ApiCredential {
            tenant: entry.tenant.to_string(),
            secret: entry.secret,
        })
        .collect();
    let gateway_port = Arc::clone(&port);
    let gateway_namespace = config.namespace.clone();
    let gateway_limits = Arc::new(gateway::Gateway::new(contract.transfers)?);
    let write_idle = Some(Duration::from_secs(
        gateway_limits.limits.write_idle_seconds,
    ));
    let app = router(Arc::clone(&port), credentials)
        .map_err(|_| "invalid API credentials")?
        .fallback(move |request| {
            gateway::handle(
                Arc::clone(&gateway_port),
                gateway_namespace.clone(),
                Some(Arc::clone(&gateway_limits)),
                request,
            )
        });
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    spawn_reconciliation_scanner(Arc::clone(&port), config.expiry_scan_seconds);
    axum::serve(gateway::GatewayListener::new(listener, write_idle), app).await?;
    Ok(())
}

#[cfg(test)]
mod tests;
