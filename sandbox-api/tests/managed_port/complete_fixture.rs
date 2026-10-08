//! Component tests use the same complete opener as the service. All budgets
//! below are explicit fixture inputs, not fallback deployment defaults.
use super::*;
use sandproof_controller_service::physical_storage::{PhysicalBuildStorage, PoolRole};
use sandproof_controller_service::{BoundedHostConfig, MaintenanceConfig};
use std::path::Path;

#[path = "../../../controller-service/tests/support/complete_fixture.rs"]
mod shared;

pub(super) const CLEANUP: u64 = shared::CLEANUP_BYTES + 1000;

pub(super) fn maintenance() -> MaintenanceConfig {
    let mut value = shared::maintenance();
    value.retention_ms = 60_000;
    let capacity = value.capacity.as_mut().unwrap();
    capacity.max_live_sandboxes = 64;
    value
}

pub(super) fn host(path: &Path, namespace: &str, config: &ManagedConfig) -> BoundedHostConfig {
    let mut host = shared::host(
        path,
        namespace,
        16,
        config.limits.max_disk_bytes_global - config.limits.cleanup_reserve_bytes,
        config.limits.cleanup_reserve_bytes,
    );
    let pool = &mut host.physical_storage.as_mut().unwrap().pools[0];
    pool.kernel_copy_bytes = 16 * 1024 * 1024;
    pool.runtime_copy_bytes = 16 * 1024 * 1024;
    host
}

pub(super) fn open_with_config(
    path: impl AsRef<Path>,
    namespace: &str,
    config: &ManagedConfig,
) -> Result<ControllerJournal, sandproof_controller_service::JournalError> {
    let path = path.as_ref();
    let bounded = host(path, namespace, config);
    let policy = maintenance();
    ControllerJournal::open_complete(path, namespace, bounded, policy, 0)
}

pub(super) fn open(
    path: impl AsRef<Path>,
    namespace: &str,
) -> Result<ControllerJournal, sandproof_controller_service::JournalError> {
    open_with_config(path, namespace, &config())
}

pub(super) fn builder_host(path: &Path) -> BoundedHostConfig {
    let mut bounded = host(path, NAMESPACE, &config());
    bounded.maintenance.windows =
        sandbox_controller_core::maintenance::windows::WindowLimits::new(2, 3_600_000).unwrap();
    let profile = bounded.physical_storage.as_mut().unwrap();
    let mut runtime = profile.pools[0].clone();
    runtime.namespace = "fedcba9876543210".into();
    let parent = path.parent().unwrap();
    runtime.database = parent.join("runtime.db");
    runtime.node_state_dir = parent.join("runtime-node");
    runtime.chroot_base = parent.join("runtime-jails");
    profile.pools[0].role = PoolRole::Builder;
    profile.pools.insert(0, runtime);
    profile.public_build = Some(PhysicalBuildStorage {
        root: parent.join("public-build"),
        registry_root: parent.join("registry"),
        workspace_bytes: 1024 * 1024,
        registry_bytes: 1024 * 1024,
        max_jobs: 2,
        max_templates: 4,
        inode_budget: 128,
    });
    bounded
}

pub(super) fn builder_port(path: &Path, host: FakeHost) -> ManagedPort<FakeHost> {
    let journal =
        ControllerJournal::open_complete(path, NAMESPACE, builder_host(path), maintenance(), 0)
            .unwrap();
    ManagedPort::new(journal, host, config())
        .with_internal_build_vms(true)
        .with_build_retry_windows(sandproof_controller_service::BuildRetryConfig {
            window_duration_ms: 3_600_000,
            max_anchors: 16,
        })
        .unwrap()
}

pub(super) fn open_registry(
    path: &Path,
    config: &ManagedConfig,
    registry: &sandproof_controller_service::template_registry::RegistryConfig,
) -> Result<ControllerJournal, sandproof_controller_service::JournalError> {
    let mut bounded = builder_host(path);
    let profile = bounded.physical_storage.as_mut().unwrap();
    profile.pools[0].role = PoolRole::Builder;
    profile.pools[0].namespace = "fedcba9876543210".into();
    profile.pools[1].role = PoolRole::Runtime;
    let pool = &mut profile.pools[1];
    pool.rootfs_budget_bytes =
        config.limits.max_disk_bytes_global - config.limits.cleanup_reserve_bytes;
    let public = profile.public_build.as_mut().unwrap();
    public.registry_root.clone_from(&registry.root);
    public.max_templates = registry.max_templates_global as u64;
    public.registry_bytes =
        registry.max_artifact_bytes_global + public.max_templates * (64 * 1024 + 87);
    ControllerJournal::open_complete(path, NAMESPACE, bounded, maintenance(), 0)
}
