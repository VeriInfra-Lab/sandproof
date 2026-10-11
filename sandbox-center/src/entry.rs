//! Product composition only; explicit provisioning never runs on recovery.
use crate::{audit, cli::Command, runtime};
use sandproof_center_service::{
    business::RegistryMetadata,
    deployment::{AssemblyError, CenterAssemblyInputs},
};
use sandproof_center_store::{RedisLedger, WriterLease, business::BusinessEnvelope};
use std::path::Path;

pub type Envelope = BusinessEnvelope<RegistryMetadata>;

pub fn load(path: &Path) -> Result<CenterAssemblyInputs, &'static str> {
    let owner = rustix::process::geteuid();
    if owner.is_root() || owner != rustix::process::getuid() {
        return Err("center requires its own unprivileged deployment account");
    }
    CenterAssemblyInputs::load(path, owner.as_raw()).map_err(AssemblyError::as_str)
}

pub fn execute(command: Command) -> Result<(), &'static str> {
    match command {
        Command::Initialize(path) => initialize(load(&path)?),
        Command::Audit(path) => inspect(load(&path)?),
        Command::Serve(path) => {
            let inputs = load(&path)?;
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|_| "center execution runtime unavailable")?;
            runtime.block_on(runtime::serve(inputs))
        }
        Command::Versions => Err("contract metadata dispatch rejected"),
    }
}

fn initialize(inputs: CenterAssemblyInputs) -> Result<(), &'static str> {
    let lease = WriterLease::acquire(
        &inputs.writer_root,
        inputs.provisioning.center.deployment().clone(),
    )
    .map_err(|_| "center writer lease unavailable")?;
    let (_ledger, _committed) = RedisLedger::<Envelope>::initialize(
        inputs.redis_endpoint,
        inputs.store,
        lease,
        &inputs.provisioning,
    )
    .map_err(|_| "explicit center provisioning rejected or unknown")?;
    Ok(())
}

/// Offline administration under the ORIGINAL writer lock. This deliberately
/// refuses while serve is active, instead of bypassing its authority lease.
/// Open/read validate the original codec and storage health; no reaffirm, CAS,
/// recovery, initialization or dispatch is performed by this command.
fn inspect(inputs: CenterAssemblyInputs) -> Result<(), &'static str> {
    let lease = WriterLease::acquire(
        &inputs.writer_root,
        inputs.provisioning.center.deployment().clone(),
    )
    .map_err(|_| "center writer lease unavailable")?;
    let (_ledger, observed) =
        RedisLedger::<Envelope>::open(inputs.redis_endpoint, inputs.store, lease)
            .map_err(|_| "original center authority inspection rejected")?;
    if !registration_matches(observed.state(), &inputs.provisioning) {
        return Err("original center registration does not match protected configuration");
    }
    let view = audit::view(observed.state(), observed.revision())?;
    serde_json::to_writer(std::io::stdout().lock(), &view)
        .map_err(|_| "center inspection output failed")
}

/// Check both immutable halves before attachment/reaffirmation. Dynamic health,
/// cache facts, windows and accepted history remain in the original authority.
pub fn registration_matches(original: &Envelope, provisioning: &Envelope) -> bool {
    original.center.registration_matches(&provisioning.center)
        && original
            .metadata
            .registration_matches(&provisioning.metadata)
}
