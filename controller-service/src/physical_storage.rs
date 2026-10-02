//! Fixed deployment-wide physical envelope. These declarations allocate no
//! blocks: the host must validate the single shared-filesystem activation and
//! observe this entire envelope free before new work. Existing allocation is
//! deliberately credited zero; a racy `du`/statvfs subtraction is never used.

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
    pub root: PathBuf,
    pub registry_root: PathBuf,
    /// Existing public workspace quota already includes its journal envelope.
    pub workspace_bytes: u64,
    pub registry_bytes: u64,
    pub max_jobs: u64,
    pub max_templates: u64,
    pub inode_budget: u64,
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
    /// Fresh activation leaves one complete peak free even after every managed
    /// writer has reached its maximum. This deliberate zero-credit contract
    /// costs twice the envelope but permits repeated cleanup/create cycles.
    /// # Errors
    /// Unrepresentable provisioning requirements are unsupported.
    pub fn activation_bytes(&self) -> Result<u64, JournalError> {
        let peak = self.peak_bytes()?;
        add(peak, peak)
    }

    /// # Errors
    /// Only one runtime pool, optionally its one private builder, is supported.
    /// All paths are distinct allocation roots; no defaults or migration exist.
    pub fn validate(&self) -> Result<(), JournalError> {
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
        let mut namespaces = BTreeSet::new();
        let mut roots = vec![self.coordination_file.as_path()];
        for pool in &self.pools {
            if pool.namespace.len() != 16
                || !pool
                    .namespace
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
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
            if public.workspace_bytes == 0
                || public.registry_bytes == 0
                || public.inode_budget == 0
                || public.max_jobs == 0
                || public.max_templates == 0
            {
                return Err(JournalError::UnsupportedStore);
            }
            // Workspace root/journal files, final and preparing job names,
            // fixed candidate staging, and retained registry trees.
            let known_inodes = add(
                add(
                    8,
                    public
                        .max_jobs
                        .checked_mul(19)
                        .ok_or(JournalError::UnsupportedStore)?,
                )?,
                public
                    .max_templates
                    .checked_mul(5)
                    .ok_or(JournalError::UnsupportedStore)?,
            )?;
            if public.inode_budget < known_inodes {
                return Err(JournalError::UnsupportedStore);
            }
            roots.extend([public.root.as_path(), public.registry_root.as_path()]);
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
                root: "/host/build".into(),
                registry_root: "/host/registry".into(),
                workspace_bytes: 500,
                registry_bytes: 700,
                max_jobs: 1,
                max_templates: 1,
                inode_budget: 32,
            }),
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
}
