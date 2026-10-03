//! Logical metadata envelope for the closed bounded-host file layout. This is
//! not a filesystem allocation guarantee: block rounding, directories/extents,
//! the jailer's tree, image copies, console policy and database files require
//! the caller's separately checked physical deployment profile.

use super::{NODE_STATE_DIRECTORIES, NodeConfig, NodeResult, host_slots, node_staging};
use sandbox_controller_core::maintenance::slots::HostSlotId;
use sandbox_runtime::{CLAIM_RECORD_NAMES, MAX_CLAIM_RECORD_BYTES, MAX_HELPER_MARKER_BYTES};

/// Upper bounds for node records and runtime claim/helper metadata, including
/// all canonical and interrupted fixed-stage files. Free host cells retain
/// their complete retirement record; dead helper/route markers may remain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostMetadataEnvelope {
    /// File contents only, not allocated blocks or filesystem overhead.
    pub payload_bytes: u64,
    /// Conservative distinct-inode count, even when publication hard-links.
    pub file_count: u64,
    /// Node state root/children, claim root/children, and stage directories.
    /// Excludes `chroot_base` itself and every jailer-created directory.
    pub directory_count: u64,
    /// `file_count` + `directory_count`; excludes image/jailer/socket inodes.
    pub inode_count: u64,
}

fn add(left: u64, right: u64) -> NodeResult<u64> {
    left.checked_add(right)
        .ok_or_else(|| "host metadata envelope overflow".into())
}
fn mul(left: u64, right: u64) -> NodeResult<u64> {
    left.checked_mul(right)
        .ok_or_else(|| "host metadata envelope overflow".into())
}
fn count(length: usize) -> NodeResult<u64> {
    Ok(u64::try_from(length)?)
}

/// Returns the exact largest serialized stage identity and lease marker sizes.
/// Must run before the first lease write: the config is also embedded in gates
/// and mapping, and cleanup can later need a full retirement cell even if the
/// slot was never able to launch. No files are touched during this preflight.
pub(super) fn validate_configuration(config: &NodeConfig) -> NodeResult<(u64, u64)> {
    let bounded = config
        .bounded_host
        .as_ref()
        .ok_or("host metadata requires bounded_host")?;
    bounded.validate()?;
    let position = HostSlotId(bounded.maintenance.max_host_slots - 1);
    let mut targets = vec![
        config.state_dir.join("host-slot-config.json"),
        host_slots::host_position_path(config, position),
        host_slots::gate_path(config, position),
    ];
    targets.extend(host_slots::retirement_paths(
        config,
        host_slots::maximum_binding(),
    ));
    let identity_bytes = targets
        .iter()
        .map(|target| node_staging::identity_bytes(config, target))
        .collect::<NodeResult<Vec<_>>>()?
        .into_iter()
        .max()
        .ok_or("missing metadata targets")?;
    let lease_bytes = count(serde_json::to_vec(&(1_u32, config))?.len())?;
    let (mapping_bytes, gate_bytes, retirement_bytes) = host_slots::maximum_record_bytes(config)?;
    if [
        identity_bytes,
        lease_bytes,
        mapping_bytes,
        gate_bytes,
        retirement_bytes,
    ]
    .into_iter()
    .any(|bytes| bytes > node_staging::MAX_PAYLOAD as u64)
    {
        return Err(
            "bounded host configuration or future retirement exceeds staged byte limit".into(),
        );
    }
    Ok((identity_bytes, lease_bytes))
}

impl HostMetadataEnvelope {
    /// Pure checked derivation from the implementation's caps and closed target
    /// lists. Let N be host slots, `L=min(N,max_live_sandboxes)`, D routes/slot,
    /// R the canonical runtime record count and K incarnation node targets.
    /// Then T=1+2N+KL and payload=T*(2P+I)+C+2RLQ+2N(1+D)H.
    /// P/Q/H are enforced node/claim/helper caps, I the exact maximum encoded
    /// stage identity, and C the exact encoded lease. The two copies represent
    /// each canonical file and its fixed stage, never `max_routes_global`: idle
    /// cells can retain dead markers. Existing free positions still count in N.
    ///
    /// The caller must establish `max_live_sandboxes` as its durable simultaneous
    /// incarnation cap. Calling this does not enable any physical guarantee;
    /// legacy file console output is deliberately outside this metadata sum.
    /// # Errors
    /// Rejects missing/invalid bounded mapping, zero live capacity, arithmetic
    /// overflow, or configuration whose future metadata cannot fit its caps.
    pub fn derive(config: &NodeConfig, max_live_sandboxes: u64) -> NodeResult<Self> {
        if max_live_sandboxes == 0 {
            return Err("host metadata requires a nonzero live incarnation cap".into());
        }
        let (identity_bytes, lease_bytes) = validate_configuration(config)?;
        let bounded = config.bounded_host.as_ref().ok_or("missing bounded_host")?;
        let positions = bounded.maintenance.max_host_slots;
        let live = positions.min(max_live_sandboxes);
        let incarnation_targets =
            count(host_slots::retirement_paths(config, host_slots::maximum_binding()).len())?;
        let targets = add(add(1, mul(2, positions)?)?, mul(incarnation_targets, live)?)?;
        // Runtime stages rename into canonical names; charge both nevertheless.
        let claim_files = mul(mul(2, count(CLAIM_RECORD_NAMES.len())?)?, live)?;
        // A gate's inflight_paths/route_paths each name a canonical+stage pair.
        let helper_files = mul(
            mul(2, positions)?,
            add(1, bounded.max_data_routes_per_slot)?,
        )?;
        let node_payload = mul(
            targets,
            add(mul(2, node_staging::MAX_PAYLOAD as u64)?, identity_bytes)?,
        )?;
        let payload_bytes = add(
            add(
                add(node_payload, lease_bytes)?,
                mul(claim_files, MAX_CLAIM_RECORD_BYTES)?,
            )?,
            mul(helper_files, MAX_HELPER_MARKER_BYTES)?,
        )?;
        let file_count = add(
            add(
                add(
                    mul(targets, add(1, count(node_staging::STAGE_FILES.len())?)?)?,
                    1,
                )?,
                claim_files,
            )?,
            helper_files,
        )?;
        // Shared roots: state_dir, each node child, each bounded child, .claims.
        let roots = add(
            add(2, count(NODE_STATE_DIRECTORIES.len())?)?,
            count(host_slots::BOUNDED_STATE_DIRECTORIES.len())?,
        )?;
        let directory_count = add(add(roots, live)?, targets)?;
        Ok(Self {
            payload_bytes,
            file_count,
            directory_count,
            inode_count: add(file_count, directory_count)?,
        })
    }
}

#[cfg(test)]
#[path = "storage_envelope/tests.rs"]
mod tests;
