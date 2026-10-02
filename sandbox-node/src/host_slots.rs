//! Durable host-side half of the core physical-position protocol. Callers keep
//! these checks and all worker I/O inside the existing `ManagedPort` boundary.
use super::{
    BoundedHostConfig, DirBuilder, File, HostSlotContext, LinuxWorker, NodeConfig, NodeResult,
    Path, PathBuf, Runtime, RuntimeConfig, RuntimeError, SlotBinding, WorkAction, artifact_digest,
    check_trusted_path, core, fs, node_staging, require_absent,
};
use sandbox_controller_core::maintenance::slots::{
    HostBinding, HostSlotId, HostSlotPhase, HostSlotRecord, SlotInput, reduce_slot,
};
use sandbox_runtime::RetirementManifest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::os::unix::{
    ffi::OsStrExt,
    fs::{DirBuilderExt, MetadataExt},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredMapping {
    version: u32,
    phase: MappingPhase,
    lease: super::host_lease::LeaseIdentity,
    config: NodeConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum MappingPhase {
    Initializing,
    Ready,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPosition {
    position: HostSlotId,
    helper_gate: sandbox_runtime::RuntimeHelperGate,
    value: Option<HostPosition>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostPosition {
    context: HostSlotContext,
    phase: HostSlotPhase,
    retirement: Option<NodeRetirement>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeRetirement {
    runtime: RuntimeConfig,
    manifest: RetirementManifest,
    files: Vec<(PathBuf, Option<FileStamp>)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileStamp {
    device: u64,
    inode: u64,
    digest: String,
}

impl StoredPosition {
    fn initialize(
        phase: MappingPhase,
        position: HostSlotId,
        gate: sandbox_runtime::RuntimeHelperGate,
        previous: Option<&Self>,
    ) -> NodeResult<Self> {
        if phase != MappingPhase::Initializing {
            return Err("Ready positions may never be initialized again".into());
        }
        if let Some(previous) = previous {
            if previous.position != position
                || previous.value.is_some()
                || previous.helper_gate != gate
            {
                return Err("initialization cannot adopt a used or changed position".into());
            }
            return Ok(previous.clone());
        }
        Ok(Self {
            position,
            helper_gate: gate,
            value: None,
        })
    }
}

impl HostPosition {
    fn record(&self) -> HostSlotRecord {
        HostSlotRecord {
            binding: self.context.binding,
            phase: self.phase,
        }
    }

    fn bind(
        previous: Option<&Self>,
        context: &HostSlotContext,
        allow_install: bool,
    ) -> NodeResult<Self> {
        if let Some(previous) = previous
            && previous.context == *context
            && previous.phase == HostSlotPhase::Occupied
        {
            return Ok(previous.clone());
        }
        if !allow_install {
            return Err("missing occupied binding after possible dispatch".into());
        }
        let record = reduce_slot(
            previous.map(Self::record),
            SlotInput::Bind {
                position: context.position,
                expected_previous: context.expected_previous,
                binding: context.binding,
            },
        )
        .map_err(|_| "stale or unavailable host position")?;
        Ok(Self {
            context: *context,
            phase: record.phase,
            retirement: None,
        })
    }

    fn require_occupied(&self, binding: HostBinding) -> NodeResult<()> {
        reduce_slot(
            Some(self.record()),
            SlotInput::CheckOccupied {
                position: self.context.position,
                binding,
            },
        )
        .map_err(|_| "host position is not occupied by this incarnation")?;
        Ok(())
    }
}

pub(super) fn validate_limits(config: &BoundedHostConfig) -> NodeResult<()> {
    config.validate()?;
    Ok(())
}

pub(super) fn position_uid(config: &BoundedHostConfig, position: HostSlotId) -> NodeResult<u32> {
    validate_limits(config)?;
    if position.0 >= config.maintenance.max_host_slots {
        return Err("host position exceeds deployment range".into());
    }
    config
        .uid_base
        .checked_add(u32::try_from(position.0)?)
        .filter(|uid| *uid != u32::MAX)
        .ok_or_else(|| "host position UID overflow".into())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> NodeResult<T> {
    check_trusted_path(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.mode() & 0o077 != 0 || metadata.len() > 16 * 1024 * 1024 {
        return Err("host position record is not private or exceeds its byte bound".into());
    }
    let value = serde_json::from_reader(File::open(path)?)?;
    File::open(path.parent().ok_or("host record parent")?)?.sync_all()?;
    Ok(value)
}

fn stamp(path: &Path) -> NodeResult<Option<FileStamp>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
        Ok(metadata) => {
            check_trusted_path(path)?;
            if !metadata.is_file() || metadata.mode() & 0o077 != 0 {
                return Err("retirement file is not private".into());
            }
            Ok(Some(FileStamp {
                device: metadata.dev(),
                inode: metadata.ino(),
                digest: artifact_digest(File::open(path)?)?,
            }))
        }
    }
}

fn remove_exact(path: &Path, expected: Option<&FileStamp>) -> NodeResult<()> {
    let actual = stamp(path)?;
    if actual.is_none() {
        File::open(path.parent().ok_or("retirement parent")?)?.sync_all()?;
        return Ok(());
    }
    if actual.as_ref() != expected {
        return Err("retirement file identity changed".into());
    }
    fs::remove_file(path)?;
    File::open(path.parent().ok_or("retirement parent")?)?.sync_all()?;
    Ok(())
}

pub(super) fn require_slot_binding(binding: &SlotBinding, expected: HostBinding) -> NodeResult<()> {
    if binding.owner.as_str() != format!("tenant-{}", expected.owner.0)
        || binding.sandbox != expected.sandbox
        || binding.generation != expected.generation
        || binding.vm != expected.vm
        || binding.slot != expected.network
    {
        return Err("host position binding mismatch".into());
    }
    Ok(())
}

fn data_route_has_capacity(
    config: &BoundedHostConfig,
    global: u64,
    own: u64,
    control: bool,
) -> bool {
    if config.validate().is_err() {
        return false;
    }
    let reserve = if control {
        0
    } else {
        config.data_route_control_reserve
    };
    global < config.max_data_routes_global - reserve
        && own < config.max_data_routes_per_slot - reserve
}

fn require_known_node_entry(expected: &BTreeSet<PathBuf>, path: &Path) -> NodeResult<()> {
    if !expected.contains(path) {
        return Err(
            "unknown node-state entry blocks bounded reuse; no identity inferred for cleanup"
                .into(),
        );
    }
    Ok(())
}

fn require_complete_positions(maximum: u64, positions: &BTreeSet<u64>) -> NodeResult<()> {
    if positions.len() as u64 != maximum || positions.iter().copied().ne(0..maximum) {
        return Err("missing initialized host position; refusing identity reset".into());
    }
    Ok(())
}

pub(super) fn stored_lease_identity(
    config: &NodeConfig,
) -> NodeResult<Option<super::host_lease::LeaseIdentity>> {
    let path = config.state_dir.join("host-slot-config.json");
    if !path.try_exists()? {
        return Ok(None);
    }
    // Snapshot only: acquire the host-root lease before any recovery/fsync.
    check_trusted_path(&path)?;
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.mode() & 0o077 != 0 {
        return Err("untrusted host mapping".into());
    }
    Ok(Some(
        serde_json::from_reader::<_, StoredMapping>(File::open(path)?)?.lease,
    ))
}

impl LinuxWorker {
    fn host_position_path(&self, position: HostSlotId) -> PathBuf {
        self.config
            .state_dir
            .join("host-slots")
            .join(format!("{}.json", position.0))
    }

    fn read_host_positions(&self) -> NodeResult<Vec<HostPosition>> {
        let Some(config) = &self.config.bounded_host else {
            return Ok(Vec::new());
        };
        let lease = self
            .host_lease
            .as_ref()
            .ok_or("bounded host lease missing")?;
        lease.validate()?;
        let mapping: StoredMapping =
            read_json(&self.config.state_dir.join("host-slot-config.json"))?;
        if mapping.version != 1
            || mapping.phase != MappingPhase::Ready
            || mapping.lease != lease.identity
            || mapping.config != self.config
        {
            return Err("bounded host mapping changed".into());
        }
        let mut rows = Vec::new();
        let mut positions = BTreeSet::new();
        let mut identities = BTreeSet::new();
        let mut networks = BTreeSet::new();
        let mut sandboxes = BTreeSet::new();
        for entry in fs::read_dir(self.config.state_dir.join("host-slots"))? {
            let entry = entry?;
            if self.known_position_stage(&entry.path(), config.maintenance.max_host_slots)? {
                continue;
            }
            let stored: StoredPosition = read_json(&entry.path())?;
            self.validate_helper_gate(stored.position, &stored.helper_gate)?;
            if entry.path() != self.host_position_path(stored.position)
                || stored.position.0 >= config.maintenance.max_host_slots
                || !positions.insert(stored.position.0)
            {
                return Err("invalid or duplicate host position cell".into());
            }
            if let Some(row) = stored.value {
                if row.context.position != stored.position
                    || !identities.insert(row.context.binding.vm.0)
                    || !networks.insert(row.context.binding.network.0)
                    || !sandboxes.insert(row.context.binding.sandbox.0)
                    || row
                        .context
                        .expected_previous
                        .is_some_and(|previous| previous >= row.context.binding.vm)
                    || (row.phase == HostSlotPhase::Occupied) != row.retirement.is_none()
                {
                    return Err("invalid or duplicate host position record".into());
                }
                rows.push(row);
            }
        }
        require_complete_positions(config.maintenance.max_host_slots, &positions)?;
        self.require_known_node_entries(&rows)?;
        self.require_known_gate_entries(config.maintenance.max_host_slots)?;
        rows.sort_by_key(|row| row.context.position.0);
        Ok(rows)
    }

    fn require_known_node_entries(&self, rows: &[HostPosition]) -> NodeResult<()> {
        let expected: BTreeSet<_> = rows
            .iter()
            .flat_map(|row| self.retirement_paths(row.context.binding))
            .collect();
        for name in ["secrets", "fences", "slots", "startup"] {
            for entry in fs::read_dir(self.config.state_dir.join(name))? {
                let path = entry?.path();
                if expected.contains(&path) {
                    require_known_node_entry(&expected, &path)?;
                    check_trusted_path(&path)?;
                } else {
                    let target = expected
                        .iter()
                        .find(|target| {
                            node_staging::path(target)
                                .as_ref()
                                .is_ok_and(|stage| stage == &path)
                        })
                        .ok_or("unknown node-state entry blocks bounded reuse")?;
                    node_staging::inspect(&self.config, target)?;
                }
            }
        }
        Ok(())
    }

    fn gate_path(&self, position: HostSlotId) -> PathBuf {
        self.config
            .state_dir
            .join("helper-gates")
            .join(format!("{}.lock", position.0))
    }

    fn gate_bytes(&self, position: HostSlotId) -> NodeResult<Vec<u8>> {
        Ok(serde_json::to_vec(&(1_u32, &self.config, position))?)
    }

    fn validate_helper_gate(
        &self,
        position: HostSlotId,
        gate: &sandbox_runtime::RuntimeHelperGate,
    ) -> NodeResult<()> {
        if gate.path != self.gate_path(position)
            || gate.max_data_routes_per_slot
                != self
                    .config
                    .bounded_host
                    .as_ref()
                    .ok_or("missing bounded mapping")?
                    .max_data_routes_per_slot
        {
            return Err("helper gate path changed".into());
        }
        check_trusted_path(&gate.path)?;
        let metadata = fs::symlink_metadata(&gate.path)?;
        if !metadata.is_file()
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
            || metadata.dev() != gate.device
            || metadata.ino() != gate.inode
            || fs::read(&gate.path)? != self.gate_bytes(position)?
        {
            return Err("helper gate identity changed".into());
        }
        Ok(())
    }

    fn helper_gate(&self, position: HostSlotId) -> NodeResult<sandbox_runtime::RuntimeHelperGate> {
        let stored: StoredPosition = read_json(&self.host_position_path(position))?;
        if stored.position != position {
            return Err("helper gate position changed".into());
        }
        self.validate_helper_gate(position, &stored.helper_gate)?;
        Ok(stored.helper_gate)
    }

    fn known_position_stage(&self, path: &Path, maximum: u64) -> NodeResult<bool> {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return Ok(false);
        };
        let Some(number) = name
            .strip_prefix('.')
            .and_then(|name| name.strip_suffix(".json.stage"))
        else {
            return Ok(false);
        };
        let position = number.parse::<u64>()?;
        if position >= maximum || position.to_string() != number {
            return Err("unknown position stage".into());
        }
        let target = self.host_position_path(HostSlotId(position));
        if node_staging::path(&target)? != path {
            return Err("position stage path mismatch".into());
        }
        node_staging::inspect(&self.config, &target)?;
        Ok(true)
    }

    fn require_known_gate_entries(&self, maximum: u64) -> NodeResult<()> {
        let mapping: StoredMapping =
            read_json(&self.config.state_dir.join("host-slot-config.json"))?;
        for entry in fs::read_dir(self.config.state_dir.join("helper-gates"))? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().ok_or("helper gate filename")?;
            let normalized = name.strip_prefix(".staging-").unwrap_or(name);
            if let Some((number, route)) = normalized.split_once(".lock.route-") {
                let position = number.parse::<u64>()?;
                let route = route.strip_suffix(".json").ok_or("route marker name")?;
                let index = route.parse::<u64>()?;
                let limit = self
                    .config
                    .bounded_host
                    .as_ref()
                    .ok_or("missing bounded mapping")?
                    .max_data_routes_per_slot;
                if position >= maximum
                    || position.to_string() != number
                    || index >= limit
                    || index.to_string() != route
                    || mapping.phase != MappingPhase::Ready
                {
                    return Err("route marker outside initialized fixed cells".into());
                }
                let cell: StoredPosition =
                    read_json(&self.host_position_path(HostSlotId(position)))?;
                if cell.value.is_none() {
                    return Err("route marker on never-used position".into());
                }
                check_trusted_path(&entry.path())?;
                let metadata = fs::symlink_metadata(entry.path())?;
                if !metadata.is_file() || metadata.mode() & 0o077 != 0 {
                    return Err("untrusted route marker".into());
                }
                continue;
            }
            let (number, staged, inflight) = if let Some(number) = name
                .strip_prefix('.')
                .and_then(|name| name.strip_suffix(".lock.stage"))
            {
                (number, true, false)
            } else if let Some(number) = name
                .strip_prefix(".staging-")
                .and_then(|name| name.strip_suffix(".lock.inflight.json"))
            {
                (number, false, true)
            } else if let Some(number) = name.strip_suffix(".lock.inflight.json") {
                (number, false, true)
            } else {
                (
                    name.strip_suffix(".lock")
                        .ok_or("unknown helper gate entry")?,
                    false,
                    false,
                )
            };
            let position = number.parse::<u64>()?;
            if position >= maximum || position.to_string() != number {
                return Err("helper gate outside fixed range".into());
            }
            if staged {
                node_staging::inspect(&self.config, &self.gate_path(HostSlotId(position)))?;
            }
            if inflight {
                if mapping.phase != MappingPhase::Ready {
                    return Err("helper marker before host initialization completed".into());
                }
                let cell: StoredPosition =
                    read_json(&self.host_position_path(HostSlotId(position)))?;
                if cell.value.is_none() {
                    return Err("helper marker on never-used position".into());
                }
                check_trusted_path(&entry.path())?;
                let metadata = fs::symlink_metadata(entry.path())?;
                if !metadata.is_file() || metadata.mode() & 0o077 != 0 {
                    return Err("untrusted helper marker".into());
                }
                // Runtime validates exact PID/start/boot/context before any
                // gate admission; inventory never deletes a live marker.
            }
        }
        Ok(())
    }

    fn require_initial_host_empty(&self, config: &BoundedHostConfig) -> NodeResult<()> {
        for name in ["secrets", "fences", "slots", "startup"] {
            if fs::read_dir(self.config.state_dir.join(name))?
                .next()
                .is_some()
            {
                return Err("bounded host enable requires empty legacy node resources".into());
            }
        }
        let prefix = format!("vm-{}-", self.config.namespace);
        let mut roots = vec![
            self.config.chroot_base.join(".claims"),
            self.config.chroot_base.join(
                self.config
                    .firecracker
                    .file_name()
                    .ok_or("firecracker name")?,
            ),
            PathBuf::from("/run/netns"),
        ];
        if let Some(resources) = &self.config.host_resources {
            roots.push(resources.cgroup_parent.clone());
        }
        for root in roots {
            if !root.try_exists()? {
                continue;
            }
            check_trusted_path(&root)?;
            for entry in fs::read_dir(root)? {
                if entry?.file_name().as_bytes().starts_with(prefix.as_bytes()) {
                    return Err("bounded host enable found legacy physical resources".into());
                }
            }
        }
        Runtime::verify_bounded_host_empty(
            &self.config.namespace,
            config.uplink_base,
            config.maintenance.max_host_slots,
        )
        .map_err(Into::into)
    }

    fn initialize_position(&self, position: HostSlotId) -> NodeResult<()> {
        let target = self.host_position_path(position);
        node_staging::cleanup(&self.config, &target)?;
        if target.try_exists()? {
            let stored: StoredPosition = read_json(&target)?;
            StoredPosition::initialize(
                MappingPhase::Initializing,
                position,
                stored.helper_gate.clone(),
                Some(&stored),
            )?;
            self.validate_helper_gate(position, &stored.helper_gate)?;
            return Ok(());
        }
        let gate_path = self.gate_path(position);
        node_staging::cleanup(&self.config, &gate_path)?;
        if !gate_path.try_exists()? {
            node_staging::write(&self.config, &gate_path, &self.gate_bytes(position)?, false)?;
        }
        check_trusted_path(&gate_path)?;
        let metadata = fs::symlink_metadata(&gate_path)?;
        let gate = sandbox_runtime::RuntimeHelperGate {
            path: gate_path,
            device: metadata.dev(),
            inode: metadata.ino(),
            max_data_routes_per_slot: self
                .config
                .bounded_host
                .as_ref()
                .ok_or("missing bounded mapping")?
                .max_data_routes_per_slot,
        };
        self.validate_helper_gate(position, &gate)?;
        self.write_node_json(
            &target,
            &StoredPosition::initialize(MappingPhase::Initializing, position, gate, None)?,
        )
    }

    fn recover_record_stages(&self, maximum: u64) -> NodeResult<()> {
        for position in 0..maximum {
            self.sync_node_record(&self.host_position_path(HostSlotId(position)))?;
            self.sync_node_record(&self.gate_path(HostSlotId(position)))?;
        }
        let rows = self.read_host_positions()?;
        for row in rows {
            for path in self.retirement_paths(row.context.binding) {
                self.sync_node_record(&path)?;
            }
        }
        Ok(())
    }

    pub(super) fn initialize_host_slots(&self) -> NodeResult<()> {
        let mapping = self.config.state_dir.join("host-slot-config.json");
        let directories = [
            self.config.state_dir.join("host-slots"),
            self.config.state_dir.join("helper-gates"),
        ];
        let Some(config) = &self.config.bounded_host else {
            if mapping.try_exists()?
                || node_staging::path(&mapping)?.try_exists()?
                || directories.iter().any(|path| path.exists())
                || self
                    .config
                    .state_dir
                    .join("bounded-host.lock")
                    .try_exists()?
            {
                return Err("bounded host mapping disabled".into());
            }
            return Ok(());
        };
        validate_limits(config)?;
        node_staging::cleanup(&self.config, &mapping)?;
        let initial = if mapping.try_exists()? {
            let stored: StoredMapping = read_json(&mapping)?;
            if stored.version != 1 || stored.config != self.config {
                return Err("bounded host mapping changed".into());
            }
            if stored.phase == MappingPhase::Ready {
                self.recover_record_stages(config.maintenance.max_host_slots)?;
                self.read_host_positions()?;
                return Ok(());
            }
            false
        } else {
            true
        };
        self.require_initial_host_empty(config)?;
        if initial {
            for path in &directories {
                if path.try_exists()? && fs::read_dir(path)?.next().is_some() {
                    return Err("positions exist without initialization authority".into());
                }
            }
            self.write_node_json(
                &mapping,
                &StoredMapping {
                    version: 1,
                    phase: MappingPhase::Initializing,
                    lease: self
                        .host_lease
                        .as_ref()
                        .ok_or("missing bounded host lease")?
                        .identity
                        .clone(),
                    config: self.config.clone(),
                },
            )?;
        }
        for path in &directories {
            if !path.try_exists()? {
                DirBuilder::new().mode(0o700).create(path)?;
                File::open(&self.config.state_dir)?.sync_all()?;
            }
            check_trusted_path(path)?;
        }
        for entry in fs::read_dir(&directories[0])? {
            let entry = entry?;
            if self.known_position_stage(&entry.path(), config.maintenance.max_host_slots)? {
                continue;
            }
            let stored: StoredPosition = read_json(&entry.path())?;
            if stored.value.is_some()
                || stored.position.0 >= config.maintenance.max_host_slots
                || entry.path() != self.host_position_path(stored.position)
            {
                return Err("foreign position during initialization".into());
            }
        }
        self.require_known_gate_entries(config.maintenance.max_host_slots)?;
        for position in 0..config.maintenance.max_host_slots {
            self.initialize_position(HostSlotId(position))?;
        }
        self.replace_node_json(
            &mapping,
            &StoredMapping {
                version: 1,
                phase: MappingPhase::Ready,
                lease: self
                    .host_lease
                    .as_ref()
                    .ok_or("missing bounded host lease")?
                    .identity
                    .clone(),
                config: self.config.clone(),
            },
        )?;
        self.read_host_positions()?;
        Ok(())
    }

    /// Persist the exact controller-authorized incarnation before its first
    /// fence or credential. A repeated identical acknowledgement is harmless.
    /// # Errors
    /// Refuses missing mapping, changed predecessors, stale IDs and busy slots.
    pub fn bind_host_slot(&self, context: &HostSlotContext, allow_install: bool) -> NodeResult<()> {
        let config = self
            .config
            .bounded_host
            .as_ref()
            .ok_or("bounded host mapping is disabled")?;
        position_uid(config, context.position)?;
        self.sync_node_record(&self.host_position_path(context.position))?;
        let rows = self.read_host_positions()?;
        for row in &rows {
            if row.context.position != context.position
                && (row.context.binding.vm == context.binding.vm
                    || row.context.binding.network == context.binding.network
                    || row.context.binding.sandbox == context.binding.sandbox)
            {
                return Err("host incarnation is already assigned another position".into());
            }
        }
        let prior = rows
            .iter()
            .find(|row| row.context.position == context.position);
        if let Some(prior) = prior
            && prior.phase == HostSlotPhase::Free
        {
            self.observe_position_free(prior)?;
        }
        let next = HostPosition::bind(prior, context, allow_install)?;
        if prior == Some(&next) {
            return Ok(());
        }
        let gate = self.helper_gate(context.position)?;
        let expected = prior
            .map(|row| self.runtime_host_slot(context.position, row.context.binding.vm))
            .transpose()?;
        let _guard = gate.try_acquire(expected.as_ref())?;
        let path = self.host_position_path(context.position);
        self.replace_node_json(
            &path,
            &StoredPosition {
                position: context.position,
                helper_gate: self.helper_gate(context.position)?,
                value: Some(next),
            },
        )
    }

    /// Read bounded contexts, retaining the last incarnation in every Free
    /// position. Releasing is explicitly not a claim of physical absence.
    /// # Errors
    /// Rejects malformed records and residue attached to a Free position.
    pub fn host_slot_inventory(&self) -> NodeResult<Vec<(HostSlotContext, HostSlotPhase)>> {
        let rows = self.read_host_positions()?;
        for row in &rows {
            if let Some(retired) = &row.retirement {
                self.validate_retirement(row, retired)?;
                for (path, expected) in &retired.files {
                    let actual = stamp(path)?;
                    if actual.is_some() && actual != *expected {
                        return Err("releasing node identity file changed".into());
                    }
                }
            }
            if row.phase == HostSlotPhase::Free {
                self.observe_position_free(row)?;
            }
        }
        Ok(rows
            .into_iter()
            .map(|row| (row.context, row.phase))
            .collect())
    }

    fn retirement_paths(&self, binding: HostBinding) -> Vec<PathBuf> {
        vec![
            self.secret_path(binding.sandbox),
            self.fence_path(binding.sandbox),
            self.slot_path(binding.network),
            self.config
                .state_dir
                .join("startup")
                .join(format!("{}.json", self.vm_name(binding.vm))),
            self.config
                .state_dir
                .join("startup")
                .join(format!("{}.request", self.vm_name(binding.vm))),
        ]
    }

    fn observe_position_free(&self, row: &HostPosition) -> NodeResult<()> {
        let retired = row
            .retirement
            .as_ref()
            .ok_or("free host position lacks retirement manifest")?;
        self.validate_retirement(row, retired)?;
        for (path, _) in &retired.files {
            require_absent(path)?;
            require_absent(&node_staging::path(path)?)?;
        }
        Runtime::for_existing(retired.runtime.clone())?
            .observe_retired(&self.vm_name(row.context.binding.vm), &retired.manifest)?;
        Ok(())
    }

    fn validate_retirement(&self, row: &HostPosition, retired: &NodeRetirement) -> NodeResult<()> {
        let binding = row.context.binding;
        if retired
            .files
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>()
            != self.retirement_paths(binding)
            || retired.runtime.host_slot
                != Some(self.runtime_host_slot(row.context.position, binding.vm)?)
            || retired.runtime.vm_uid
                != position_uid(
                    self.config
                        .bounded_host
                        .as_ref()
                        .ok_or("missing bounded mapping")?,
                    row.context.position,
                )?
            || retired.runtime.chroot_base != self.config.chroot_base
            || retired.runtime.firecracker != self.config.firecracker
            || retired.runtime.jailer != self.config.jailer
            || retired.runtime.private_netns
                != Some(PathBuf::from(format!(
                    "/run/netns/{}",
                    self.vm_name(binding.vm)
                )))
        {
            return Err("retirement manifest differs from occupied identity".into());
        }
        Ok(())
    }

    /// Persist Releasing and an immutable cleanup manifest before removing
    /// any identity evidence. Unknown or interrupted work never frees a slot.
    /// # Errors
    /// Refuses foreign resources, live VMs and unobservable exact cleanup.
    pub fn release_host_slot(&self, context: &HostSlotContext) -> NodeResult<()> {
        let mut row = self
            .read_host_positions()?
            .into_iter()
            .find(|row| row.context.position == context.position)
            .ok_or("missing host position for release")?;
        if row.context != *context {
            return Err("host release context changed".into());
        }
        // Before acting on a recovered manifest or acknowledging Free, make
        // a prior uncertain atomic publication durable on this exact parent.
        self.sync_node_record(&self.host_position_path(context.position))?;
        if row.phase == HostSlotPhase::Free {
            return self.observe_position_free(&row);
        }
        if row.phase == HostSlotPhase::Occupied {
            let binding = context.binding;
            row.retirement = Some(self.capture_retirement(context)?);
            row.phase = reduce_slot(
                Some(row.record()),
                SlotInput::BeginRelease {
                    position: context.position,
                    binding,
                },
            )
            .map_err(|_| "invalid host release")?
            .phase;
            self.replace_node_json(
                &self.host_position_path(context.position),
                &StoredPosition {
                    position: context.position,
                    helper_gate: self.helper_gate(context.position)?,
                    value: Some(row.clone()),
                },
            )?;
        }
        let retired = row
            .retirement
            .as_ref()
            .ok_or("missing immutable retirement manifest")?;
        self.validate_retirement(&row, retired)?;
        let runtime = Runtime::for_existing(retired.runtime.clone())?;
        runtime.retire(&self.vm_name(context.binding.vm), &retired.manifest)?;
        for (path, expected) in &retired.files {
            remove_exact(path, expected.as_ref())?;
        }
        runtime.observe_retired(&self.vm_name(context.binding.vm), &retired.manifest)?;
        row.phase = reduce_slot(
            Some(row.record()),
            SlotInput::ConfirmRelease {
                position: context.position,
                binding: context.binding,
            },
        )
        .map_err(|_| "invalid host release confirmation")?
        .phase;
        self.replace_node_json(
            &self.host_position_path(context.position),
            &StoredPosition {
                position: context.position,
                helper_gate: self.helper_gate(context.position)?,
                value: Some(row.clone()),
            },
        )
    }

    fn capture_retirement(&self, context: &HostSlotContext) -> NodeResult<NodeRetirement> {
        let binding = context.binding;
        for path in self.retirement_paths(binding) {
            node_staging::cleanup(&self.config, &path)?;
        }
        if self.fence_path(binding.sandbox).try_exists()? && !self.fence(binding.sandbox)?.terminal
        {
            return Err("host release requires terminal fence".into());
        }
        if self.slot_path(binding.network).try_exists()? {
            let slot = self.slot(binding.network)?;
            require_slot_binding(&slot, binding)?;
        }
        let startup_path = self
            .config
            .state_dir
            .join("startup")
            .join(format!("{}.json", self.vm_name(binding.vm)));
        let request_path = self
            .config
            .state_dir
            .join("startup")
            .join(format!("{}.request", self.vm_name(binding.vm)));
        if startup_path.try_exists()? {
            let startup: crate::startup::StartupRecord = read_json(&startup_path)?;
            if startup.sandbox != binding.sandbox.0 || startup.vm != binding.vm.0 {
                return Err("retirement startup identity mismatch".into());
            }
            if request_path.try_exists()? {
                check_trusted_path(&request_path)?;
                let expected = crate::startup::start_request(
                    startup
                        .template
                        .startup
                        .as_ref()
                        .ok_or("request without template startup")?,
                )?;
                if fs::read(&request_path)? != expected {
                    return Err("retirement startup request mismatch".into());
                }
            }
        } else {
            require_absent(&request_path)?;
        }
        let runtime = if self.has_secret(binding.sandbox)? {
            self.runtime_config(binding.sandbox, binding.vm)?
        } else {
            require_absent(
                &self
                    .config
                    .chroot_base
                    .join(".claims")
                    .join(self.vm_name(binding.vm)),
            )?;
            RuntimeConfig {
                jailer: self.config.jailer.clone(),
                firecracker: self.config.firecracker.clone(),
                chroot_base: self.config.chroot_base.clone(),
                kernel: self.config.kernel.clone(),
                rootfs: self.config.rootfs.clone(),
                vm_uid: self.vm_uid(binding.vm)?,
                vm_gid: self.vm_uid(binding.vm)?,
                vcpu_count: self.config.vcpu_count,
                memory_mib: self.config.memory_mib,
                host_resources: self.config.host_resources.clone(),
                private_netns: Some(PathBuf::from(format!(
                    "/run/netns/{}",
                    self.vm_name(binding.vm)
                ))),
                access_token_hash: Some("0".repeat(128)),
                host_slot: Some(self.runtime_host_slot(context.position, binding.vm)?),
            }
        };
        let manifest = Runtime::for_existing(runtime.clone())?
            .retirement_manifest(&self.vm_name(binding.vm))?;
        let files = self
            .retirement_paths(binding)
            .into_iter()
            .map(|path| {
                let state = stamp(&path)?;
                Ok((path, state))
            })
            .collect::<NodeResult<Vec<_>>>()?;
        Ok(NodeRetirement {
            runtime,
            manifest,
            files,
        })
    }

    pub(super) fn require_occupied_sandbox(&self, sandbox: core::SandboxId) -> NodeResult<()> {
        if self.config.bounded_host.is_none() {
            return Ok(());
        }
        let rows = self.read_host_positions()?;
        let row = rows
            .iter()
            .find(|row| row.context.binding.sandbox == sandbox)
            .ok_or("sandbox has no occupied host position")?;
        row.require_occupied(row.context.binding)
    }

    /// Pin the exact occupied incarnation while a data request is prepared.
    /// Tracked relay spawning adds the durable process witness before effects.
    /// # Errors
    /// Rejects a stale incarnation, changed gate or active exclusive mutator.
    pub fn acquire_data_gate(
        &self,
        sandbox: core::SandboxId,
        vm: core::VmId,
        generation: core::Generation,
    ) -> NodeResult<Option<File>> {
        if self.config.bounded_host.is_none() {
            return Ok(None);
        }
        let rows = self.read_host_positions()?;
        let row = rows
            .iter()
            .find(|row| row.context.binding.vm == vm)
            .ok_or("data gate VM not bound")?;
        if row.context.binding.sandbox != sandbox || row.context.binding.generation != generation {
            return Err("data gate incarnation mismatch".into());
        }
        row.require_occupied(row.context.binding)?;
        let context = self.runtime_host_slot(row.context.position, vm)?;
        let guard = context.helper_gate.try_acquire_shared(Some(&context))?;
        Ok(Some(guard.into_file()))
    }

    fn durable_route_counts(
        &self,
        rows: &[HostPosition],
        position: HostSlotId,
    ) -> NodeResult<(u64, u64)> {
        let mut global = 0_u64;
        let mut own = 0_u64;
        // NeverUsed cells were already checked to have no helper/route markers
        // by read_host_positions; every used cell, including Releasing/Free,
        // participates in this restart-stable census.
        for row in rows {
            let context = self.runtime_host_slot(row.context.position, row.context.binding.vm)?;
            for route in context.helper_gate.route_snapshot(Some(&context))? {
                if route.state != sandbox_runtime::RuntimeRouteState::Idle {
                    global = global.checked_add(1).ok_or(super::RouteCapacityExceeded)?;
                    if row.context.position == position {
                        own = own.checked_add(1).ok_or(super::RouteCapacityExceeded)?;
                    }
                }
            }
        }
        Ok((global, own))
    }

    /// Start a tracked data relay only while the caller holds the management
    /// boundary and has rechecked the current route permission. Fixed route
    /// cells and the shared helper gate survive the controller process.
    /// # Errors
    /// Rejects stale bindings, unsafe executables and exhausted/unknown cells.
    pub fn spawn_data_relay(
        &self,
        sandbox: core::SandboxId,
        vm: core::VmId,
        generation: core::Generation,
        control: bool,
        executable: &Path,
    ) -> NodeResult<Option<std::process::Child>> {
        if self.config.bounded_host.is_none() {
            return Ok(None);
        }
        check_trusted_path(executable)?;
        if !executable.is_file() {
            return Err("data relay executable is not a trusted file".into());
        }
        let rows = self.read_host_positions()?;
        let row = rows
            .iter()
            .find(|row| row.context.binding.vm == vm)
            .ok_or("relay VM not bound")?;
        if row.context.binding.sandbox != sandbox || row.context.binding.generation != generation {
            return Err("relay incarnation mismatch".into());
        }
        row.require_occupied(row.context.binding)?;
        let (global, own) = self.durable_route_counts(&rows, row.context.position)?;
        let bounded = self
            .config
            .bounded_host
            .as_ref()
            .ok_or("missing bounded mapping")?;
        if !data_route_has_capacity(bounded, global, own, control) {
            return Err(super::RouteCapacityExceeded.into());
        }
        let context = self.runtime_host_slot(row.context.position, vm)?;
        let guard = context.helper_gate.try_acquire_shared(Some(&context))?;
        let name = self.vm_name(vm);
        let executable = executable
            .to_str()
            .ok_or("relay executable path is not UTF-8")?;
        let child = guard.spawn_route(
            &context,
            control,
            "/usr/bin/ip",
            &["netns", "exec", &name, executable, "relay-bounded"],
        )?;
        // The durable route cell survives even if the child closes inherited
        // descriptors. Only exact observation under the management lock may
        // retire it; a late asynchronous waiter cannot erase a successor.
        Ok(Some(child))
    }

    /// Observe the durable gate only after local routes have drained. This
    /// does not replace physical resource/namespace-holder retirement checks.
    /// # Errors
    /// Rejects a mismatched context or untrusted gate.
    pub fn data_gate_drained(&self, context: &HostSlotContext) -> NodeResult<bool> {
        let rows = self.read_host_positions()?;
        let row = rows
            .iter()
            .find(|row| row.context.position == context.position)
            .ok_or("data drain host position missing")?;
        if row.context != *context {
            return Err("data drain incarnation mismatch".into());
        }
        let expected = self.runtime_host_slot(context.position, context.binding.vm)?;
        match expected.helper_gate.try_acquire(Some(&expected)) {
            Ok(_guard) => Ok(true),
            Err(RuntimeError::InvalidConfiguration(_)) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub(super) fn runtime_host_slot(
        &self,
        position: HostSlotId,
        vm: core::VmId,
    ) -> NodeResult<sandbox_runtime::RuntimeHostSlot> {
        let bounded = self
            .config
            .bounded_host
            .as_ref()
            .ok_or("missing bounded mapping")?;
        position_uid(bounded, position)?;
        Ok(sandbox_runtime::RuntimeHostSlot {
            position: position.0,
            vm: vm.0,
            helper_gate: self.helper_gate(position)?,
            uplink_index: bounded
                .uplink_base
                .checked_add(position.0)
                .ok_or("uplink index overflow")?,
        })
    }

    pub(super) fn physical_slot_for_vm(&self, vm: core::VmId) -> NodeResult<Option<HostSlotId>> {
        if self.config.bounded_host.is_none() {
            return Ok(None);
        }
        let row = self
            .read_host_positions()?
            .into_iter()
            .find(|row| row.context.binding.vm == vm)
            .ok_or("VM has no occupied host position")?;
        row.require_occupied(row.context.binding)?;
        Ok(Some(row.context.position))
    }

    pub(super) fn require_occupied_binding(&self, binding: &SlotBinding) -> NodeResult<()> {
        if self.config.bounded_host.is_none() {
            return Ok(());
        }
        let row = self
            .read_host_positions()?
            .into_iter()
            .find(|row| row.context.binding.vm == binding.vm)
            .ok_or("unbound VM incarnation")?;
        let expected = row.context.binding;
        require_slot_binding(binding, expected)?;
        row.require_occupied(expected)
    }

    pub(super) fn require_occupied_action(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> NodeResult<()> {
        if self.config.bounded_host.is_none() {
            return Ok(());
        }
        check_occupied_action(&self.read_host_positions()?, action)
    }
}

fn check_occupied_action(
    rows: &[HostPosition],
    action: &WorkAction<core::EffectId>,
) -> NodeResult<()> {
    let command = action.command.command();
    let mut matches = rows.iter().filter(|row| {
        row.context.binding.sandbox == command.sandbox()
            && row.context.binding.generation == action.identity.generation
    });
    let row = matches
        .next()
        .ok_or("action has no current host position")?;
    if matches.next().is_some() {
        return Err("ambiguous action host position".into());
    }
    let binding = row.context.binding;
    if action.identity.owner.as_str() != format!("tenant-{}", binding.owner.0) {
        return Err("action host owner mismatch".into());
    }
    let (vm, network) = match command {
        core::Command::PrepareVm { vm, slot, .. }
        | core::Command::AbortPrepare { vm, slot, .. }
        | core::Command::ConfigureNetwork { vm, slot, .. } => (Some(*vm), Some(*slot)),
        core::Command::ActivateVm { vm, .. }
        | core::Command::StopVm { vm, .. }
        | core::Command::CleanupVm { vm, .. } => (Some(*vm), None),
        core::Command::CleanupSlot { slot, .. } => (None, Some(*slot)),
        core::Command::InstallFence { .. } | core::Command::ApplyFilePolicy { .. } => (None, None),
        _ => return Err("unsupported bounded host command".into()),
    };
    if vm.is_some_and(|vm| vm != binding.vm) || network.is_some_and(|slot| slot != binding.network)
    {
        return Err("action physical binding mismatch".into());
    }
    row.require_occupied(binding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_controller_core::maintenance::{MaintenanceLimits, windows::WindowLimits};
    use sandproof_protocol::{
        ActionIdentity, LogicalActionId, OperationId, PROTOCOL_VERSION, ResourceId, ResourceKind,
        ResourceRef, TenantId,
    };

    fn config() -> BoundedHostConfig {
        BoundedHostConfig {
            uid_base: 10_000,
            uplink_base: 100,
            max_data_routes_per_slot: 4,
            max_data_routes_global: 8,
            data_route_control_reserve: 1,
            maintenance: MaintenanceLimits {
                max_retained_results: 16,
                max_host_slots: 1,
                windows: WindowLimits::new(2, 1000).unwrap(),
            },
        }
    }

    fn context(vm: u64, predecessor: Option<u64>) -> HostSlotContext {
        HostSlotContext {
            position: HostSlotId(0),
            expected_previous: predecessor.map(core::VmId),
            binding: HostBinding {
                owner: core::TenantId(7),
                sandbox: core::SandboxId(vm),
                generation: 3,
                vm: core::VmId(vm),
                network: core::NetSlotId(vm),
            },
        }
    }

    fn action(command: core::Command) -> WorkAction<core::EffectId> {
        let command = core::EffectId::initial(command);
        WorkAction {
            identity: ActionIdentity {
                protocol_version: PROTOCOL_VERSION,
                owner: TenantId::try_from("tenant-7").unwrap(),
                sandbox: sandproof_protocol::SandboxId::try_from("sb-test-1").unwrap(),
                operation: OperationId::try_from("op-test-1").unwrap(),
                epoch: 1,
                generation: 3,
                logical_action: LogicalActionId::try_from("effect-1").unwrap(),
                attempt: 0,
                resource: ResourceRef {
                    kind: ResourceKind::Sandbox,
                    id: ResourceId::try_from("sb-test-1").unwrap(),
                },
            },
            command,
        }
    }

    #[test]
    fn fixed_cells_cannot_be_recreated_when_first_incarnation_is_missing() {
        let encoded = serde_json::to_vec(&StoredPosition {
            position: HostSlotId(0),
            helper_gate: sandbox_runtime::RuntimeHelperGate {
                path: PathBuf::from("/gates/0"),
                device: 1,
                inode: 2,
                max_data_routes_per_slot: 4,
            },
            value: None,
        })
        .unwrap();
        let restored: StoredPosition = serde_json::from_slice(&encoded).unwrap();
        let mut positions = BTreeSet::from([restored.position.0]);
        require_complete_positions(1, &positions).unwrap();
        let used = HostPosition::bind(None, &context(0, None), true).unwrap();
        let encoded = serde_json::to_vec(&StoredPosition {
            position: HostSlotId(0),
            helper_gate: sandbox_runtime::RuntimeHelperGate {
                path: PathBuf::from("/gates/0"),
                device: 1,
                inode: 2,
                max_data_routes_per_slot: 4,
            },
            value: Some(used),
        })
        .unwrap();
        let restored: StoredPosition = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored.value.unwrap().context.expected_previous, None);
        positions.remove(&0);
        assert!(require_complete_positions(1, &positions).is_err());
        assert!(require_complete_positions(2, &BTreeSet::from([0])).is_err());
        assert!(require_complete_positions(1, &BTreeSet::from([1])).is_err());
    }

    #[test]
    fn missing_after_dispatch_never_reinstalls_and_exact_ack_is_idempotent() {
        let ctx = context(0, None);
        assert!(HostPosition::bind(None, &ctx, false).is_err());
        let occupied = HostPosition::bind(None, &ctx, true).unwrap();
        assert_eq!(
            HostPosition::bind(Some(&occupied), &ctx, false).unwrap(),
            occupied
        );
        assert_eq!(
            HostPosition::bind(Some(&occupied), &ctx, true).unwrap(),
            occupied
        );
        let mut released = occupied.clone();
        released.phase = HostSlotPhase::Free;
        assert!(HostPosition::bind(Some(&released), &ctx, true).is_err());
        assert!(HostPosition::bind(Some(&released), &context(1, Some(0)), false).is_err());
    }

    #[test]
    fn one_physical_uid_can_serve_many_permanent_ids_without_truncation() {
        let config = config();
        let mut prior = None;
        for vm in [0, 3, u64::from(u32::MAX) + 5, u64::MAX] {
            let previous = prior
                .as_ref()
                .map(|row: &HostPosition| row.context.binding.vm.0);
            let context = context(vm, previous);
            let mut row = HostPosition::bind(prior.as_ref(), &context, true).unwrap();
            assert_eq!(position_uid(&config, context.position).unwrap(), 10_000);
            assert_eq!(row.context.binding.vm.0, vm);
            row.phase = HostSlotPhase::Releasing;
            assert!(HostPosition::bind(Some(&row), &context, true).is_err());
            row.phase = HostSlotPhase::Free;
            prior = Some(row);
        }
        assert!(position_uid(&config, HostSlotId(1)).is_err());
        assert!(HostPosition::bind(prior.as_ref(), &context(0, Some(u64::MAX)), true).is_err());
    }

    #[test]
    fn opening_and_terminal_fences_both_require_exact_occupied_incarnation() {
        let ctx = context(10, None);
        let mut row = HostPosition::bind(None, &ctx, true).unwrap();
        for terminal in [false, true] {
            let action = action(core::Command::InstallFence {
                sandbox: ctx.binding.sandbox,
                epoch: 1,
                terminal,
            });
            check_occupied_action(std::slice::from_ref(&row), &action).unwrap();
            for phase in [HostSlotPhase::Releasing, HostSlotPhase::Free] {
                row.phase = phase;
                assert!(check_occupied_action(std::slice::from_ref(&row), &action).is_err());
            }
            row.phase = HostSlotPhase::Occupied;
            let mut wrong = action.clone();
            wrong.identity.owner = TenantId::try_from("tenant-8").unwrap();
            assert!(check_occupied_action(std::slice::from_ref(&row), &wrong).is_err());
            wrong = action.clone();
            wrong.identity.generation += 1;
            assert!(check_occupied_action(std::slice::from_ref(&row), &wrong).is_err());
            assert!(check_occupied_action(&[], &action).is_err());
            let successor = HostPosition::bind(None, &context(11, None), true).unwrap();
            assert!(check_occupied_action(&[successor], &action).is_err());
        }
    }

    #[test]
    fn stale_prepare_cleanup_and_reclaim_commands_fail_the_shared_gate() {
        let context = context(10, None);
        let row = HostPosition::bind(None, &context, true).unwrap();
        let binding = context.binding;
        let commands = [
            core::Command::PrepareVm {
                sandbox: binding.sandbox,
                epoch: 1,
                generation: 3,
                vm: binding.vm,
                slot: binding.network,
                source: None,
            },
            core::Command::AbortPrepare {
                sandbox: binding.sandbox,
                epoch: 1,
                generation: 3,
                vm: binding.vm,
                slot: binding.network,
            },
            core::Command::ActivateVm {
                sandbox: binding.sandbox,
                epoch: 1,
                generation: 3,
                vm: binding.vm,
            },
            core::Command::StopVm {
                sandbox: binding.sandbox,
                epoch: 1,
                generation: 3,
                vm: binding.vm,
            },
            core::Command::CleanupVm {
                sandbox: binding.sandbox,
                generation: 3,
                vm: binding.vm,
            },
            core::Command::CleanupSlot {
                sandbox: binding.sandbox,
                generation: 3,
                slot: binding.network,
            },
        ];
        for command in commands {
            let action = action(command);
            check_occupied_action(std::slice::from_ref(&row), &action).unwrap();
            let mut changed = row.clone();
            changed.context.binding.vm = core::VmId(99);
            changed.context.binding.network = core::NetSlotId(99);
            assert!(check_occupied_action(&[changed], &action).is_err());
            let mut releasing = row.clone();
            releasing.phase = HostSlotPhase::Releasing;
            assert!(check_occupied_action(&[releasing], &action).is_err());
        }
    }

    #[test]
    fn full_slot_binding_includes_owner_sandbox_generation_vm_and_network() {
        let expected = context(10, None).binding;
        let binding = SlotBinding {
            owner: TenantId::try_from("tenant-7").unwrap(),
            sandbox: expected.sandbox,
            generation: expected.generation,
            vm: expected.vm,
            slot: expected.network,
        };
        require_slot_binding(&binding, expected).unwrap();
        for changed in [
            HostBinding {
                owner: core::TenantId(8),
                ..expected
            },
            HostBinding {
                sandbox: core::SandboxId(11),
                ..expected
            },
            HostBinding {
                generation: 4,
                ..expected
            },
            HostBinding {
                vm: core::VmId(11),
                ..expected
            },
            HostBinding {
                network: core::NetSlotId(11),
                ..expected
            },
        ] {
            assert!(require_slot_binding(&binding, changed).is_err());
        }
    }
    #[test]
    fn orphan_atomic_temporary_or_other_incarnation_blocks_reuse() {
        let expected = BTreeSet::from([
            PathBuf::from("/node/startup/vm-ns-1.json"),
            PathBuf::from("/node/startup/vm-ns-1.request"),
        ]);
        for path in &expected {
            require_known_node_entry(&expected, path).unwrap();
        }
        for path in [
            "/node/startup/.tmp-random",
            "/node/startup/vm-ns-0.request",
            "/node/secrets/sb-ns-1",
        ] {
            assert!(require_known_node_entry(&expected, Path::new(path)).is_err());
        }
    }
    #[test]
    fn initialization_restart_only_resumes_never_used_positions() {
        let gate = sandbox_runtime::RuntimeHelperGate {
            path: PathBuf::from("/gates/0"),
            device: 1,
            inode: 2,
            max_data_routes_per_slot: 4,
        };
        let initial = StoredPosition::initialize(
            MappingPhase::Initializing,
            HostSlotId(0),
            gate.clone(),
            None,
        )
        .unwrap();
        let restored: StoredPosition =
            serde_json::from_slice(&serde_json::to_vec(&initial).unwrap()).unwrap();
        assert_eq!(
            StoredPosition::initialize(
                MappingPhase::Initializing,
                HostSlotId(0),
                gate.clone(),
                Some(&restored)
            )
            .unwrap(),
            initial
        );
        assert!(
            StoredPosition::initialize(MappingPhase::Ready, HostSlotId(0), gate.clone(), None)
                .is_err()
        );
        let mut used = restored;
        used.value = Some(HostPosition::bind(None, &context(0, None), true).unwrap());
        assert!(
            StoredPosition::initialize(
                MappingPhase::Initializing,
                HostSlotId(0),
                gate.clone(),
                Some(&used)
            )
            .is_err()
        );
        let mut replaced = gate.clone();
        replaced.inode += 1;
        assert!(
            StoredPosition::initialize(
                MappingPhase::Initializing,
                HostSlotId(0),
                replaced,
                Some(&initial)
            )
            .is_err()
        );
    }
    #[test]
    fn durable_old_routes_preserve_global_and_per_slot_control_reserve() {
        let config = config();
        // Live and unknown old cells both remain charged after local counters
        // reset. Three fill the per-slot normal budget, seven the global one.
        assert!(!data_route_has_capacity(&config, 7, 3, false));
        assert!(data_route_has_capacity(&config, 7, 3, true));
        assert!(!data_route_has_capacity(&config, 8, 4, true));
        assert!(!data_route_has_capacity(&config, 7, 0, false));
        assert!(data_route_has_capacity(&config, 7, 0, true));
        assert!(!data_route_has_capacity(&config, 8, 0, true));
        assert!(!data_route_has_capacity(&config, 3, 3, false));
    }
}
