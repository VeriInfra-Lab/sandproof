//! Fixed deployment-wide physical envelope. These declarations allocate no
//! blocks: the host must validate the single shared-filesystem activation and
//! retain the full growth envelope while checking only the new operation's
//! headroom. Existing allocation gets no racy `du`/statvfs subtraction credit.

use crate::{AdmissionLimits, JournalError};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolRole {
    Runtime,
    Builder,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalPool {
    pub role: PoolRole,
    pub namespace: String,
    pub database: PathBuf,
    pub node_state_dir: PathBuf,
    pub chroot_base: PathBuf,
    /// Existing global disk quota minus its separately charged cleanup reserve.
    pub rootfs_budget_bytes: u64,
    pub max_host_slots: u64,
    pub kernel_copy_bytes: u64,
    pub runtime_copy_bytes: u64,
    /// Includes the complete controller and node/runtime metadata peaks.
    pub cleanup_reserve_bytes: u64,
    pub host_metadata_bytes: u64,
    pub host_metadata_inodes: u64,
    /// Operator-validated bound for the pinned jailer tree, including the
    /// three copied files, API socket, devices and all jail directories.
    pub jail_inodes_per_slot: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalBuildStorage {
    /// Separate trusted helper and center owners, never request parameters.
    pub workspace_owner_uid: u32,
    pub registry_owner_uid: u32,
    pub root: PathBuf,
    pub registry_root: PathBuf,
    /// Existing public workspace quota already includes its journal envelope.
    pub workspace_bytes: u64,
    pub registry_bytes: u64,
    pub max_jobs: u64,
    pub max_templates: u64,
    pub inode_budget: u64,
}

/// Bounded immutable copies and incomplete downloads. Both use one payload
/// limit, separate from the center registry and each VM's writable disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalTemplateCache {
    /// Exact original pool namespace. A cache has one host-process writer.
    pub namespace: String,
    pub root: PathBuf,
    pub max_entries: u64,
    pub max_downloads: u64,
    pub max_entry_bytes: u64,
    /// Combined bytes reserved for ready entries and pending downloads.
    pub artifact_bytes: u64,
    /// Sublimit for incomplete payloads, including recovery observations.
    pub temporary_bytes: u64,
    pub metadata_bytes: u64,
    pub inode_budget: u64,
}

impl PhysicalTemplateCache {
    /// # Errors
    /// Rejects unreserved copies, downloads or exact descriptor metadata.
    pub fn validate(&self) -> Result<(), JournalError> {
        let temporary = self
            .max_downloads
            .checked_mul(self.max_entry_bytes)
            .ok_or(JournalError::UnsupportedStore)?;
        let metadata = add(
            4096,
            self.max_entries
                .checked_mul(2 * (64 * 1024 + 4096))
                .ok_or(JournalError::UnsupportedStore)?,
        )?;
        let inodes = add(
            add(
                4, // root, inbox, artifacts, and pre-provisioned cache lock
                self.max_entries
                    .checked_mul(5)
                    .ok_or(JournalError::UnsupportedStore)?,
            )?,
            self.max_downloads
                .checked_mul(5)
                .ok_or(JournalError::UnsupportedStore)?,
        )?;
        if !namespace_valid(&self.namespace)
            || !absolute(&self.root)
            || [
                self.max_entries,
                self.max_downloads,
                self.max_entry_bytes,
                self.artifact_bytes,
                self.temporary_bytes,
                self.metadata_bytes,
                self.inode_budget,
            ]
            .into_iter()
            .any(|value| value == 0 || i64::try_from(value).is_err())
            || self.max_downloads > self.max_entries
            || self.max_entry_bytes > self.artifact_bytes
            || self.temporary_bytes < temporary
            || self.metadata_bytes < metadata
            || self.inode_budget < inodes
        {
            return Err(JournalError::UnsupportedStore);
        }
        self.peak_bytes()?;
        Ok(())
    }
    /// The cache charges ready and pending descriptors to the same total and
    /// publishes by renaming the original directory. Only metadata is additional.
    /// # Errors
    /// The combined provision must be representable in bytes.
    pub fn peak_bytes(&self) -> Result<u64, JournalError> {
        add(self.artifact_bytes, self.metadata_bytes)
    }
}

/// Explicit service material (for example protected binaries, TLS, Redis and
/// retained evidence). Its full logical provision is charged, never `du` credit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalServiceData {
    pub root: PathBuf,
    pub owner_uid: u32,
    pub peak_bytes: u64,
    pub inode_budget: u64,
    /// A single runtime's protected snapshot project has a separate hard quota.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub on_demand_capture: bool,
}

impl PhysicalServiceData {
    fn validate(&self) -> Result<(), JournalError> {
        if !absolute(&self.root)
            || self.peak_bytes == 0
            || self.inode_budget == 0
            || i64::try_from(self.peak_bytes).is_err()
            || i64::try_from(self.inode_budget).is_err()
        {
            return Err(JournalError::UnsupportedStore);
        }
        Ok(())
    }
}

/// One observation of the leased filesystem, shared by its single runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureSpace {
    pub usable_bytes: u64,
    pub available_bytes: u64,
}

/// A pause holds both its new files and the disk blocks that an older snapshot
/// can pin while a restored or new instance writes to its own working disk.
#[cfg(feature = "durable")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CapturePhysicalBytes {
    pub(crate) capture: u64,
    pub(crate) restore: u64,
}

#[cfg(feature = "durable")]
impl CapturePhysicalBytes {
    pub(crate) fn for_capture(
        full: &crate::capture::FullCaptureWorkRequest,
    ) -> Result<Self, JournalError> {
        // Only the VMM's Full RAM/state are newly written. Seal and Load use
        // same-XFS reflinks. Their extent/CoW metadata has its own profile budget.
        Ok(Self {
            capture: add(
                add(full.memory_bytes, full.state_bytes)?,
                2 * 262_144 + 65_536,
            )?,
            restore: full.disk_bytes,
        })
    }

    pub(crate) fn checked_add(self, other: Self) -> Result<Self, JournalError> {
        Ok(Self {
            capture: add(self.capture, other.capture)?,
            restore: add(self.restore, other.restore)?,
        })
    }
}

#[cfg(feature = "durable")]
pub(crate) struct CaptureAdmission {
    pub(crate) space: CaptureSpace,
    pub(crate) fixed_bytes: u64,
    pub(crate) cleanup_and_fs_bytes: u64,
}

#[cfg(feature = "durable")]
impl CaptureAdmission {
    pub(crate) fn fits(
        &self,
        held: CapturePhysicalBytes,
        next: CapturePhysicalBytes,
    ) -> Result<bool, JournalError> {
        let total = held.checked_add(next)?;
        // Already allocated bytes remain included in the durable upper bound,
        // but must not also be deducted from the current free-space sample.
        let required = add(self.fixed_bytes, add(total.capture, total.restore)?)?;
        let needed_now = add(self.cleanup_and_fs_bytes, next.capture)?;
        Ok(self.space.usable_bytes >= required && self.space.available_bytes >= needed_now)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalStorageProfile {
    pub version: u32,
    /// Operator-provisioned empty, private, single-link file on the shared FS.
    /// All managed writers on that filesystem must agree on this one profile.
    pub coordination_file: PathBuf,
    pub pools: Vec<PhysicalPool>,
    pub public_build: Option<PhysicalBuildStorage>,
    /// Finite independent caches, each bound to exactly one registered pool.
    /// Components without artifact caching explicitly register an empty list.
    pub host_template_caches: Vec<PhysicalTemplateCache>,
    pub service_data: Vec<PhysicalServiceData>,
    /// Explicit filesystem-specific block/extent/directory overhead provision.
    pub filesystem_overhead_bytes: u64,
    /// Full inode provision, including temporary files and directories.
    pub filesystem_overhead_inodes: u64,
}

fn add(a: u64, b: u64) -> Result<u64, JournalError> {
    a.checked_add(b)
        .filter(|value| i64::try_from(*value).is_ok())
        .ok_or(JournalError::UnsupportedStore)
}

fn absolute(path: &Path) -> bool {
    path.is_absolute()
        && path.parent().is_some()
        && path.to_str().is_some_and(|p| {
            !p.contains('\0')
                && p.split('/')
                    .skip(1)
                    .all(|part| !matches!(part, "" | "." | ".."))
        })
        && path
            .components()
            .all(|p| matches!(p, Component::RootDir | Component::Normal(_)))
}

fn namespace_valid(namespace: &str) -> bool {
    namespace.len() == 16
        && namespace
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

impl PhysicalBuildStorage {
    fn validate(&self) -> Result<(), JournalError> {
        if [
            self.workspace_bytes,
            self.registry_bytes,
            self.inode_budget,
            self.max_jobs,
            self.max_templates,
        ]
        .contains(&0)
        {
            return Err(JournalError::UnsupportedStore);
        }
        // Existing workspace/journal, preparing jobs, candidate staging and registry trees.
        let known = add(
            add(
                8,
                self.max_jobs
                    .checked_mul(19)
                    .ok_or(JournalError::UnsupportedStore)?,
            )?,
            self.max_templates
                .checked_mul(5)
                .ok_or(JournalError::UnsupportedStore)?,
        )?;
        if self.inode_budget < known {
            return Err(JournalError::UnsupportedStore);
        }
        Ok(())
    }
}

impl PhysicalPool {
    /// # Errors
    /// Rejects overflow; occupied and unknown slots retain the same maximum.
    pub fn peak_bytes(&self) -> Result<u64, JournalError> {
        let copies = add(self.kernel_copy_bytes, self.runtime_copy_bytes)?
            .checked_mul(self.max_host_slots)
            .ok_or(JournalError::UnsupportedStore)?;
        add(
            add(self.rootfs_budget_bytes, copies)?,
            self.cleanup_reserve_bytes,
        )
    }

    /// # Errors
    /// Rejects a changed logical quota or a cleanup reservation counted twice.
    pub fn validate_limits(&self, limits: &AdmissionLimits) -> Result<(), JournalError> {
        if limits.cleanup_reserve_bytes != self.cleanup_reserve_bytes
            || limits
                .max_disk_bytes_global
                .checked_sub(limits.cleanup_reserve_bytes)
                != Some(self.rootfs_budget_bytes)
        {
            return Err(JournalError::UnsupportedStore);
        }
        Ok(())
    }
}

impl PhysicalStorageProfile {
    /// Fresh activation covers all configured growth plus the largest new-work
    /// headroom. Template copies and already bounded writers are not duplicated
    /// in every operation's free-space check. Inode policy remains independent.
    /// # Errors
    /// Unrepresentable provisioning requirements are unsupported.
    pub fn activation_bytes(&self) -> Result<u64, JournalError> {
        add(self.peak_bytes()?, self.new_work_headroom_bytes()?)
    }

    /// Extra space kept available for control records, failure cleanup and
    /// filesystem metadata. These are global, not a second disk/template pool.
    /// # Errors
    /// Arithmetic overflow rejects the declaration.
    pub fn cleanup_headroom_bytes(&self) -> Result<u64, JournalError> {
        self.pools
            .iter()
            .try_fold(self.filesystem_overhead_bytes, |total, pool| {
                add(total, pool.cleanup_reserve_bytes)
            })
    }

    /// # Errors
    /// A foreign pool or a disk above its logical quota cannot borrow capacity.
    pub fn create_headroom_bytes(
        &self,
        pool: &PhysicalPool,
        disk_bytes: u64,
    ) -> Result<u64, JournalError> {
        if !self.pools.contains(pool) || disk_bytes > pool.rootfs_budget_bytes {
            return Err(JournalError::UnsupportedStore);
        }
        add(
            self.cleanup_headroom_bytes()?,
            add(
                disk_bytes,
                add(pool.kernel_copy_bytes, pool.runtime_copy_bytes)?,
            )?,
        )
    }

    /// # Errors
    /// Only the exact registered cache can request download headroom.
    pub fn cache_headroom_bytes(&self, cache: &PhysicalTemplateCache) -> Result<u64, JournalError> {
        if !self.host_template_caches.contains(cache) {
            return Err(JournalError::UnsupportedStore);
        }
        add(
            self.cleanup_headroom_bytes()?,
            add(cache.max_entry_bytes, cache.metadata_bytes)?,
        )
    }

    /// Whole public workspace/registry provision remains conservative; this
    /// does not borrow runtime disks or unrelated host caches for publication.
    /// # Errors
    /// Missing public-build capacity or overflow rejects the request.
    pub fn publication_headroom_bytes(&self) -> Result<u64, JournalError> {
        let public = self
            .public_build
            .as_ref()
            .ok_or(JournalError::UnsupportedStore)?;
        add(
            self.cleanup_headroom_bytes()?,
            add(public.workspace_bytes, public.registry_bytes)?,
        )
    }

    /// Largest immediate operation; all simultaneous writers still share their
    /// original bounded quotas in `peak_bytes`, not this headroom calculation.
    /// # Errors
    /// Unrepresentable requirements are unsupported.
    pub fn new_work_headroom_bytes(&self) -> Result<u64, JournalError> {
        let mut largest = self.cleanup_headroom_bytes()?;
        for pool in &self.pools {
            largest = largest.max(self.create_headroom_bytes(pool, pool.rootfs_budget_bytes)?);
        }
        for cache in &self.host_template_caches {
            largest = largest.max(self.cache_headroom_bytes(cache)?);
        }
        if self.public_build.is_some() {
            largest = largest.max(self.publication_headroom_bytes()?);
        }
        Ok(largest)
    }

    fn validate_roles(&self) -> Result<(), JournalError> {
        if self.version != 1
            || self.pools.is_empty()
            || self.pools.len() > 2
            || self.filesystem_overhead_bytes == 0
            || self.filesystem_overhead_inodes == 0
            || self
                .pools
                .iter()
                .filter(|p| p.role == PoolRole::Runtime)
                .count()
                != 1
            || (self.pools.len() == 2) != self.public_build.is_some()
            || self
                .pools
                .iter()
                .filter(|p| p.role == PoolRole::Builder)
                .count()
                != self.pools.len() - 1
        {
            return Err(JournalError::UnsupportedStore);
        }
        Ok(())
    }

    /// # Errors
    /// Only one runtime pool, optionally its one private builder, is supported.
    /// All paths are distinct allocation roots; no defaults or migration exist.
    pub fn validate(&self) -> Result<(), JournalError> {
        self.validate_roles()?;
        let mut namespaces = BTreeSet::new();
        let mut roots = vec![self.coordination_file.as_path()];
        for pool in &self.pools {
            if !namespace_valid(&pool.namespace)
                || !namespaces.insert(&pool.namespace)
                || [
                    pool.rootfs_budget_bytes,
                    pool.max_host_slots,
                    pool.kernel_copy_bytes,
                    pool.runtime_copy_bytes,
                    pool.cleanup_reserve_bytes,
                    pool.host_metadata_bytes,
                    pool.host_metadata_inodes,
                    pool.jail_inodes_per_slot,
                ]
                .into_iter()
                .any(|value| value == 0 || i64::try_from(value).is_err())
                || pool.cleanup_reserve_bytes < pool.host_metadata_bytes
                || pool.jail_inodes_per_slot < 4
            {
                return Err(JournalError::UnsupportedStore);
            }
            roots.extend([
                pool.database.as_path(),
                pool.node_state_dir.as_path(),
                pool.chroot_base.as_path(),
            ]);
        }
        if let Some(public) = &self.public_build {
            public.validate()?;
            roots.extend([public.root.as_path(), public.registry_root.as_path()]);
        }
        let mut cache_namespaces = BTreeSet::new();
        if self.host_template_caches.len() > self.pools.len() || self.service_data.len() > 8 {
            return Err(JournalError::UnsupportedStore);
        }
        for cache in &self.host_template_caches {
            cache.validate()?;
            if !namespaces.contains(&cache.namespace) || !cache_namespaces.insert(&cache.namespace)
            {
                return Err(JournalError::UnsupportedStore);
            }
            roots.push(cache.root.as_path());
        }
        for service in &self.service_data {
            service.validate()?;
            if service.on_demand_capture
                && (self.pools.len() != 1
                    || service.owner_uid != 0
                    || !self.pools[0].chroot_base.ends_with("v")
                    || self.pools[0].chroot_base.with_file_name("s") != service.root)
            {
                return Err(JournalError::UnsupportedStore);
            }
            roots.push(service.root.as_path());
        }
        for (index, path) in roots.iter().enumerate() {
            if !absolute(path)
                || roots[..index]
                    .iter()
                    .any(|other| path.starts_with(other) || other.starts_with(path))
            {
                return Err(JournalError::UnsupportedStore);
            }
        }
        self.activation_bytes()?;
        add(
            self.filesystem_overhead_inodes,
            self.filesystem_overhead_inodes,
        )?;
        if self.filesystem_overhead_inodes < self.required_inodes()? {
            return Err(JournalError::UnsupportedStore);
        }
        Ok(())
    }

    /// # Errors
    /// Minimum known inode universe; filesystem/jailer overhead remains an
    /// explicit fixed-deployment premise and may demand a larger provision.
    pub fn required_inodes(&self) -> Result<u64, JournalError> {
        let mut count = 1; // shared coordination file
        for pool in &self.pools {
            let jail = pool
                .jail_inodes_per_slot
                .checked_mul(pool.max_host_slots)
                .ok_or(JournalError::UnsupportedStore)?;
            // DB/WAL/SHM/controller lock, database parent, chroot_base and
            // executable-name parent. Node helper includes state and .claims.
            count = add(count, add(add(pool.host_metadata_inodes, jail)?, 7)?)?;
        }
        if let Some(public) = &self.public_build {
            count = add(count, public.inode_budget)?;
        }
        for cache in &self.host_template_caches {
            count = add(count, cache.inode_budget)?;
        }
        for service in &self.service_data {
            count = add(count, service.inode_budget)?;
        }
        Ok(count)
    }

    /// # Errors
    /// Returns the summed physical peak, never a per-pool shared-disk check.
    pub fn peak_bytes(&self) -> Result<u64, JournalError> {
        let mut peak = self.filesystem_overhead_bytes;
        for pool in &self.pools {
            peak = add(peak, pool.peak_bytes()?)?;
        }
        if let Some(public) = &self.public_build {
            peak = add(add(peak, public.workspace_bytes)?, public.registry_bytes)?;
        }
        for cache in &self.host_template_caches {
            peak = add(peak, cache.peak_bytes()?)?;
        }
        for service in &self.service_data {
            if !service.on_demand_capture {
                peak = add(peak, service.peak_bytes)?;
            }
        }
        Ok(peak)
    }

    /// # Errors
    /// A missing or duplicate role/namespace never borrows another pool's quota.
    pub fn pool(&self, namespace: &str) -> Result<&PhysicalPool, JournalError> {
        self.validate()?;
        self.pools
            .iter()
            .find(|pool| pool.namespace == namespace)
            .ok_or(JournalError::UnsupportedStore)
    }
    /// # Errors
    /// Each production host must select its one exact, independently owned cache.
    pub fn template_cache(&self, namespace: &str) -> Result<&PhysicalTemplateCache, JournalError> {
        self.validate()?;
        self.host_template_caches
            .iter()
            .find(|cache| cache.namespace == namespace)
            .ok_or(JournalError::UnsupportedStore)
    }
    /// # Errors
    /// Bind publication before its first durable intent or filesystem rename.
    pub fn validate_registry(
        &self,
        namespace: &str,
        registry: &crate::template_registry::RegistryConfig,
    ) -> Result<(), JournalError> {
        let pool = self.pool(namespace)?;
        let public = self
            .public_build
            .as_ref()
            .ok_or(JournalError::UnsupportedStore)?;
        let count = u64::try_from(registry.max_templates_global)
            .map_err(|_| JournalError::UnsupportedStore)?;
        let metadata = count
            .checked_mul(64 * 1024 + 87)
            .ok_or(JournalError::UnsupportedStore)?;
        if pool.role != PoolRole::Runtime
            || !registry.is_valid()
            || public.registry_root != registry.root
            || public.max_templates != count
            || public.registry_bytes < add(registry.max_artifact_bytes_global, metadata)?
            || pool.kernel_copy_bytes < registry.max_artifact_bytes_per_tenant
        {
            return Err(JournalError::UnsupportedStore);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pool(role: PoolRole, namespace: &str) -> PhysicalPool {
        let root = PathBuf::from(format!("/host/{namespace}"));
        PhysicalPool {
            role,
            namespace: namespace.into(),
            database: root.join("db"),
            node_state_dir: root.join("node"),
            chroot_base: root.join("jail"),
            rootfs_budget_bytes: 1000,
            max_host_slots: 2,
            kernel_copy_bytes: 30,
            runtime_copy_bytes: 20,
            cleanup_reserve_bytes: 200,
            host_metadata_bytes: 100,
            host_metadata_inodes: 10,
            jail_inodes_per_slot: 8,
        }
    }
    fn profile() -> PhysicalStorageProfile {
        PhysicalStorageProfile {
            version: 1,
            coordination_file: "/host/physical-profile".into(),
            pools: vec![
                pool(PoolRole::Runtime, "0123456789abcdef"),
                pool(PoolRole::Builder, "1123456789abcdef"),
            ],
            public_build: Some(PhysicalBuildStorage {
                workspace_owner_uid: 0,
                registry_owner_uid: 0,
                root: "/host/build".into(),
                registry_root: "/host/registry".into(),
                workspace_bytes: 500,
                registry_bytes: 700,
                max_jobs: 1,
                max_templates: 1,
                inode_budget: 32,
            }),
            host_template_caches: vec![],
            service_data: vec![],
            filesystem_overhead_bytes: 50,
            filesystem_overhead_inodes: 100,
        }
    }
    #[test]
    fn two_pools_reserve_all_sparse_potential_copies_and_cleanup() {
        let profile = profile();
        profile.validate().unwrap();
        assert_eq!(profile.peak_bytes().unwrap(), 3850);
        // No allocated-block credit: the same peak applies to older sparse,
        // pending prepare and unknown cleanup, even if one pool is now idle.
        assert_eq!(profile.pools[0].peak_bytes().unwrap(), 1300);
    }

    #[test]
    fn operation_headroom_does_not_duplicate_unrelated_reserved_writers() {
        let mut profile = profile();
        assert_eq!(profile.cleanup_headroom_bytes().unwrap(), 450);
        assert_eq!(
            profile
                .create_headroom_bytes(&profile.pools[0], 100)
                .unwrap(),
            600
        );
        assert_eq!(
            profile
                .create_headroom_bytes(&profile.pools[0], 1000)
                .unwrap(),
            1500
        );
        assert!(
            profile
                .create_headroom_bytes(&profile.pools[0], 1001)
                .is_err()
        );
        let mut foreign = profile.pools[0].clone();
        foreign.namespace = "2123456789abcdef".into();
        assert!(profile.create_headroom_bytes(&foreign, 100).is_err());
        assert_eq!(profile.publication_headroom_bytes().unwrap(), 1650);
        assert_eq!(profile.activation_bytes().unwrap(), 5500);
        // Existing static service material is still fully charged to growth,
        // but a create does not need another copy of that service's storage.
        profile.service_data.push(PhysicalServiceData {
            root: "/host/static".into(),
            owner_uid: 0,
            peak_bytes: 4000,
            inode_budget: 4,
            on_demand_capture: false,
        });
        assert_eq!(profile.activation_bytes().unwrap(), 9500);
        assert_eq!(
            profile
                .create_headroom_bytes(&profile.pools[0], 100)
                .unwrap(),
            600
        );
        // Even full warming leaves every declared operation's guard. Logical
        // quotas, not a free-space sample, still constrain concurrent requests.
        let peak = profile.peak_bytes().unwrap();
        for allocated in [0, peak / 2, peak] {
            let free = profile.activation_bytes().unwrap() - allocated;
            assert!(free >= profile.new_work_headroom_bytes().unwrap());
        }
    }
    #[test]
    fn mismatch_overflow_unknown_roles_and_overlapping_roots_fail() {
        let mut profile = profile();
        profile.pools[1].namespace = profile.pools[0].namespace.clone();
        assert!(profile.validate().is_err());
        profile = self::profile();
        profile.pools[1].role = PoolRole::Runtime;
        assert!(profile.validate().is_err());
        profile = self::profile();
        profile.pools[1].chroot_base = profile.pools[0].node_state_dir.clone();
        assert!(profile.validate().is_err());
        profile = self::profile();
        profile.pools[0].max_host_slots = u64::MAX;
        assert!(profile.validate().is_err());
        profile = self::profile();
        profile.public_build = None;
        assert!(profile.validate().is_err());
    }
    #[test]
    fn registry_root_count_payload_kernel_and_inode_bounds_are_checked_together() {
        let mut profile = profile();
        profile.pools[0].kernel_copy_bytes = 512;
        profile.public_build.as_mut().unwrap().registry_bytes = 1024 + 64 * 1024 + 87;
        let registry = crate::template_registry::RegistryConfig {
            root: "/host/registry".into(),
            max_templates_global: 1,
            max_templates_per_tenant: 1,
            max_artifact_bytes_global: 1024,
            max_artifact_bytes_per_tenant: 512,
            validation_timeout_secs: 60,
        };
        profile
            .validate_registry("0123456789abcdef", &registry)
            .unwrap();
        let mut changed = registry.clone();
        changed.root = "/host/unlisted".into();
        assert!(
            profile
                .validate_registry("0123456789abcdef", &changed)
                .is_err()
        );
        changed = registry.clone();
        changed.max_templates_global = 2;
        assert!(
            profile
                .validate_registry("0123456789abcdef", &changed)
                .is_err()
        );
        profile.pools[0].kernel_copy_bytes -= 1;
        assert!(
            profile
                .validate_registry("0123456789abcdef", &registry)
                .is_err()
        );
        profile.pools[0].kernel_copy_bytes += 1;
        profile.public_build.as_mut().unwrap().registry_bytes -= 1;
        assert!(
            profile
                .validate_registry("0123456789abcdef", &registry)
                .is_err()
        );
        profile.public_build.as_mut().unwrap().registry_bytes += 1;
        profile.public_build.as_mut().unwrap().inode_budget = 31;
        assert!(
            profile
                .validate_registry("0123456789abcdef", &registry)
                .is_err()
        );
    }

    #[test]
    fn canonical_paths_reject_raw_dot_empty_or_trailing_components() {
        for path in ["/host/./node", "/host//node", "/host/node/"] {
            let mut profile = profile();
            profile.pools[0].node_state_dir = path.into();
            assert!(profile.validate().is_err());
        }
    }

    #[test]
    fn cache_shares_its_payload_peak_and_retains_temporary_and_metadata_checks() {
        let mut profile = profile();
        let cache = PhysicalTemplateCache {
            namespace: "0123456789abcdef".into(),
            root: "/host/cache".into(),
            max_entries: 2,
            max_downloads: 1,
            max_entry_bytes: 1000,
            artifact_bytes: 2000,
            temporary_bytes: 1000,
            metadata_bytes: 4096 + 2 * 2 * (64 * 1024 + 4096),
            inode_budget: 19,
        };
        cache.validate().unwrap();
        let extra = cache.peak_bytes().unwrap();
        assert_eq!(extra, 2000 + cache.metadata_bytes);
        profile.filesystem_overhead_inodes += cache.inode_budget;
        profile.host_template_caches = vec![cache.clone()];
        profile.validate().unwrap();
        assert_eq!(profile.peak_bytes().unwrap(), 3850 + extra);
        assert_eq!(
            profile.activation_bytes().unwrap(),
            3850 + extra + 450 + 1000 + cache.metadata_bytes
        );
        for field in ["temporary", "metadata", "inode", "overlap", "namespace"] {
            let mut changed = profile.clone();
            let cache = &mut changed.host_template_caches[0];
            match field {
                "temporary" => cache.temporary_bytes -= 1,
                "metadata" => cache.metadata_bytes -= 1,
                "inode" => cache.inode_budget -= 1,
                "overlap" => cache.root = "/host/registry/cache".into(),
                "namespace" => cache.namespace = "2123456789abcdef".into(),
                _ => unreachable!(),
            }
            assert!(changed.validate().is_err(), "{field}");
        }
        let mut builder_cache = cache;
        builder_cache.namespace = "1123456789abcdef".into();
        builder_cache.root = "/host/builder-cache".into();
        profile.filesystem_overhead_inodes += builder_cache.inode_budget;
        profile.host_template_caches.push(builder_cache);
        profile.validate().unwrap();
        assert_eq!(profile.peak_bytes().unwrap(), 3850 + 2 * extra);
        profile.host_template_caches[1].namespace =
            profile.host_template_caches[0].namespace.clone();
        assert!(profile.validate().is_err());
    }
    #[test]
    fn service_material_has_explicit_peak_and_inodes_not_filesystem_overhead_credit() {
        let mut profile = profile();
        profile.service_data = vec![PhysicalServiceData {
            on_demand_capture: false,
            root: "/host/center-service".into(),
            owner_uid: 64001,
            peak_bytes: 4096,
            inode_budget: 4,
        }];
        profile.filesystem_overhead_inodes += 4;
        profile.validate().unwrap();
        assert_eq!(profile.peak_bytes().unwrap(), 3850 + 4096);
        assert_eq!(profile.activation_bytes().unwrap(), 3850 + 4096 + 1650);
        profile.service_data[0].root = "/host/registry/nested".into();
        assert!(profile.validate().is_err());
        profile.service_data[0].root = "/host/center-service".into();
        profile.service_data[0].peak_bytes = 0;
        assert!(profile.validate().is_err());
    }
}
