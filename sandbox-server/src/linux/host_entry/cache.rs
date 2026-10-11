//! Protected immutable host copies. No lifecycle journal or remote path API.
use sandproof_controller_service::{
    InternalTemplate,
    application::{ApplicationError, ControllerHost},
    physical_storage::PhysicalTemplateCache,
    template_registry::{CandidateStore, RegistryConfig},
    templates::TemplateBinding,
};
use sandproof_host_service::{HostLifecycle, TemplateSource};
use sandproof_internal::{
    ArtifactClient, ReceiveError,
    cache::{ArtifactPart, ArtifactRequest, CacheAvailability, CacheRequest},
    read_protected,
};
use sandproof_node::NodeHost;
use sandproof_protocol::internal::{NodeIdentity, lifecycle::CreateBinding};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    error::Error,
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::sync::Semaphore;

const MANIFEST_BYTES: usize = 2 * (64 * 1024 + 4096);

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    request: CacheRequest,
    local: TemplateBinding,
    device: u64,
    inode: u64,
}
#[derive(Default)]
struct State {
    ready: BTreeMap<String, InternalTemplate>,
    /// Full maximum remains charged until exact owned temporary cleanup.
    pending: BTreeMap<String, Pending>,
    ready_bytes: u64,
}
struct Pending {
    manifest: Manifest,
    /// Only this process can own a download while holding the cache lease.
    downloading: bool,
}
pub(super) struct TemplateCache<H: ControllerHost = NodeHost> {
    config: PhysicalTemplateCache,
    node: NodeIdentity,
    client: ArtifactClient,
    store: CandidateStore,
    lifecycle: HostLifecycle<H>,
    state: Mutex<State>,
    downloads: Arc<Semaphore>,
    lease: CacheLease,
}
/// One process owns the fixed cache namespace. This has no journal or quota
/// state; every actual download worker retains the same `TemplateCache` lease.
struct CacheLease {
    file: File,
    path: std::path::PathBuf,
    device: u64,
    inode: u64,
}
impl CacheLease {
    fn open(root: &Path) -> Result<Self, Box<dyn Error>> {
        let path = root.join(".cache.lock");
        sandbox_runtime::check_trusted_path(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(i32::try_from(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits(),
            )?)
            .open(&path)?;
        file.try_lock()?;
        let metadata = file.metadata()?;
        let value = Self {
            file,
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        sandbox_runtime::check_trusted_path(&self.path)?;
        let current = fs::symlink_metadata(&self.path)?;
        let opened = self.file.metadata()?;
        if !current.is_file()
            || current.uid() != 0
            || current.nlink() != 1
            || current.len() != 0
            || current.mode() & 0o7777 != 0o600
            || current.dev() != self.device
            || current.ino() != self.inode
            || (opened.dev(), opened.ino()) != (self.device, self.inode)
        {
            return Err("cache writer lock identity changed".into());
        }
        Ok(())
    }
}
impl Drop for CacheLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
fn key(request: &CacheRequest) -> String {
    let mut hash = Sha256::new();
    hash.update(request.revision.as_str().as_bytes());
    hash.update([0]);
    hash.update(request.digest);
    format!("{:x}", hash.finalize())
}
fn bytes(binding: &TemplateBinding) -> Result<u64, ReceiveError> {
    binding
        .kernel
        .size_bytes
        .checked_add(binding.rootfs.size_bytes)
        .ok_or(ReceiveError::Conflict)
}
fn project(request: &CacheRequest, directory: &Path) -> TemplateBinding {
    let mut local = request.descriptor.clone();
    local.kernel.path = directory.join("kernel");
    local.rootfs.path = directory.join("rootfs.ext4");
    local
}
fn exact_directory(path: &Path, manifest: &Manifest) -> Result<u64, ReceiveError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ReceiveError::Unavailable)?;
    if !metadata.is_dir()
        || metadata.uid() != 0
        || metadata.mode() & 0o077 != 0
        || (metadata.dev(), metadata.ino()) != (manifest.device, manifest.inode)
    {
        return Err(ReceiveError::Unavailable);
    }
    let mut partial_bytes = 0u64;
    for entry in fs::read_dir(path).map_err(|_| ReceiveError::Unavailable)? {
        let entry = entry.map_err(|_| ReceiveError::Unavailable)?;
        if !matches!(
            entry.file_name().to_str(),
            Some("manifest.json" | "kernel" | "rootfs.ext4")
        ) {
            return Err(ReceiveError::Unavailable);
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| ReceiveError::Unavailable)?;
        let maximum = match entry.file_name().to_str() {
            Some("kernel") => manifest.local.kernel.size_bytes,
            Some("rootfs.ext4") => manifest.local.rootfs.size_bytes,
            _ => MANIFEST_BYTES as u64,
        };
        if !metadata.is_file()
            || metadata.uid() != 0
            || metadata.nlink() != 1
            || metadata.dev() != manifest.device
            || metadata.len() > maximum
        {
            return Err(ReceiveError::Unavailable);
        }
        if entry.file_name() != "manifest.json" {
            partial_bytes = partial_bytes
                .checked_add(metadata.len())
                .ok_or(ReceiveError::Capacity)?;
        }
    }
    Ok(partial_bytes)
}
fn load_manifest(
    directory: &Path,
    config: &PhysicalTemplateCache,
    node: &NodeIdentity,
) -> Result<(Manifest, u64), Box<dyn Error>> {
    let manifest: Manifest = serde_json::from_slice(&read_protected(
        &directory.join("manifest.json"),
        0,
        MANIFEST_BYTES,
    )?)?;
    manifest.request.validate(node)?;
    let id = key(&manifest.request);
    if manifest.version != 1
        || directory.file_name().and_then(|v| v.to_str()) != Some(&id)
        || manifest.local != project(&manifest.request, &config.root.join("artifacts").join(&id))
        || bytes(&manifest.local).map_err(|_| "cache length overflow")? > config.max_entry_bytes
    {
        return Err("invalid protected cache manifest".into());
    }
    let partial_bytes =
        exact_directory(directory, &manifest).map_err(|_| "cache directory identity changed")?;
    Ok((manifest, partial_bytes))
}

impl State {
    fn restore(
        config: &PhysicalTemplateCache,
        node: &NodeIdentity,
        store: &CandidateStore,
    ) -> Result<Self, Box<dyn Error>> {
        let mut state = Self::default();
        let mut partial_bytes = 0u64;
        for area in ["inbox", "artifacts"] {
            for entry in fs::read_dir(config.root.join(area))? {
                let (manifest, length) = load_manifest(&entry?.path(), config, node)?;
                let id = key(&manifest.request);
                if area == "inbox" {
                    partial_bytes = partial_bytes
                        .checked_add(length)
                        .ok_or("cache temporary capacity overflow")?;
                    if config.root.join("artifacts").join(&id).try_exists()?
                        || state
                            .pending
                            .insert(
                                id,
                                Pending {
                                    manifest,
                                    downloading: false,
                                },
                            )
                            .is_some()
                    {
                        return Err("ambiguous cache staging identity".into());
                    }
                } else {
                    store.verify_binding(&manifest.local)?;
                    state.ready_bytes = state
                        .ready_bytes
                        .checked_add(bytes(&manifest.local).map_err(|_| "cache length overflow")?)
                        .ok_or("cache capacity overflow")?;
                    let revision = manifest.request.revision.as_str().to_owned();
                    if state
                        .ready
                        .insert(
                            revision,
                            InternalTemplate {
                                revision: manifest.request.revision,
                                digest: manifest.request.digest,
                                registered: manifest.request.descriptor,
                                local: manifest.local,
                            },
                        )
                        .is_some()
                    {
                        return Err("duplicate immutable cache revision".into());
                    }
                }
            }
        }
        if state.ready.len() + state.pending.len() > usize::try_from(config.max_entries)?
            || state.ready_bytes > config.artifact_bytes
            || partial_bytes > config.temporary_bytes
            || state
                .pending
                .values()
                .try_fold(state.ready_bytes, |total, entry| {
                    total
                        .checked_add(
                            bytes(&entry.manifest.local).map_err(|_| "cache length overflow")?,
                        )
                        .ok_or("cache capacity overflow")
                })?
                > config.artifact_bytes
        {
            return Err("existing cache exceeds its immutable reservation".into());
        }
        Ok(state)
    }
}

impl<H: ControllerHost + Send + 'static> TemplateCache<H> {
    pub(super) fn open(
        config: PhysicalTemplateCache,
        node: NodeIdentity,
        client: ArtifactClient,
        lifecycle: HostLifecycle<H>,
    ) -> Result<Self, Box<dyn Error>> {
        config.validate()?;
        let lease = CacheLease::open(&config.root)?;
        let store = CandidateStore::new(
            RegistryConfig {
                root: config.root.clone(),
                max_templates_global: usize::try_from(config.max_entries)?,
                max_templates_per_tenant: usize::try_from(config.max_entries)?,
                max_artifact_bytes_global: config.artifact_bytes,
                max_artifact_bytes_per_tenant: config.max_entry_bytes,
                validation_timeout_secs: 300,
            },
            0,
        )?;
        let state = State::restore(&config, &node, &store)?;
        let downloads = Arc::new(Semaphore::new(usize::try_from(config.max_downloads)?));
        let value = Self {
            config,
            node,
            client,
            store,
            lifecycle,
            state: Mutex::new(state),
            downloads,
            lease,
        };
        // The lease excludes every original worker. Validate the whole restored
        // inventory before changing anything, then discard only exact partial
        // files; the durable manifest and original directory identity remain.
        value
            .reclaim_interrupted_downloads()
            .map_err(|_| "interrupted cache identity could not be reclaimed")?;
        Ok(value)
    }

    pub(super) async fn receive(
        self: Arc<Self>,
        request: CacheRequest,
    ) -> Result<CacheAvailability, ReceiveError> {
        request
            .validate(&self.node)
            .map_err(|_| ReceiveError::Conflict)?;
        let permit = Arc::clone(&self.downloads)
            .try_acquire_owned()
            .map_err(|_| ReceiveError::Capacity)?;
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            runtime.block_on(self.receive_work(request))
        })
        .await
        .map_err(|_| ReceiveError::Unavailable)?
    }
    async fn receive_work(&self, request: CacheRequest) -> Result<CacheAvailability, ReceiveError> {
        self.lease
            .validate()
            .map_err(|_| ReceiveError::Unavailable)?;
        if let Some(value) = self.known(&request)? {
            return Ok(value);
        }
        self.lifecycle
            .cache_storage_ready(self.node.clone(), self.config.clone())
            .await
            .map_err(|_| ReceiveError::Unavailable)?;
        let manifest = self.prepare(request.clone())?;
        let staging = self.config.root.join("inbox").join(key(&request));
        let downloaded = self.download(&manifest, &staging).await;
        if downloaded.is_err() {
            // Do not release an uncertain directory after an identity change.
            // Only these exact private files were ever permitted before seal.
            if self.cleanup_partial(&manifest, &staging).is_ok() {
                let mut state = self.state.lock().map_err(|_| ReceiveError::Unavailable)?;
                let pending = state
                    .pending
                    .get_mut(&key(&request))
                    .ok_or(ReceiveError::Unavailable)?;
                pending.downloading = false;
            }
            return Err(ReceiveError::Unavailable);
        }
        self.publish(manifest, &staging)?;
        self.known(&request)?.ok_or(ReceiveError::Unavailable)
    }
    pub(super) async fn drain(&self) -> Result<(), Box<dyn Error>> {
        let _all = Arc::clone(&self.downloads)
            .acquire_many_owned(u32::try_from(self.config.max_downloads)?)
            .await?;
        Ok(())
    }
    fn known(&self, request: &CacheRequest) -> Result<Option<CacheAvailability>, ReceiveError> {
        let value = self
            .state
            .lock()
            .map_err(|_| ReceiveError::Unavailable)?
            .ready
            .get(request.revision.as_str())
            .cloned();
        let Some(value) = value else {
            return Ok(None);
        };
        if value.digest != request.digest || value.registered != request.descriptor {
            return Err(ReceiveError::Conflict);
        }
        self.store
            .verify_binding(&value.local)
            .map_err(|_| ReceiveError::Unavailable)?;
        let state = self.state.lock().map_err(|_| ReceiveError::Unavailable)?;
        if !state
            .ready
            .get(request.revision.as_str())
            .is_some_and(|same| {
                same.digest == value.digest
                    && same.registered == value.registered
                    && same.local == value.local
            })
        {
            return Err(ReceiveError::Unavailable);
        }
        Ok(Some(CacheAvailability {
            node: self.node.clone(),
            revision: request.revision.clone(),
            digest: request.digest,
        }))
    }
    fn prepare(&self, request: CacheRequest) -> Result<Manifest, ReceiveError> {
        let mut state = self.state.lock().map_err(|_| ReceiveError::Unavailable)?;
        if state
            .pending
            .values()
            .filter(|entry| entry.downloading)
            .count()
            >= usize::try_from(self.config.max_downloads).map_err(|_| ReceiveError::Capacity)?
        {
            return Err(ReceiveError::Capacity);
        }
        let id = key(&request);
        if let Some(pending) = state.pending.get_mut(&id) {
            if pending.downloading || pending.manifest.request != request {
                return Err(ReceiveError::Unavailable);
            }
            exact_directory(&self.config.root.join("inbox").join(&id), &pending.manifest)?;
            pending.downloading = true;
            return Ok(pending.manifest.clone());
        }
        let length = bytes(&request.descriptor)?;
        let reserved = state
            .pending
            .values()
            .try_fold(state.ready_bytes, |total, entry| {
                total
                    .checked_add(bytes(&entry.manifest.local)?)
                    .ok_or(ReceiveError::Capacity)
            })?;
        if length > self.config.max_entry_bytes
            || reserved
                .checked_add(length)
                .is_none_or(|v| v > self.config.artifact_bytes)
            || state.ready.len() + state.pending.len()
                >= usize::try_from(self.config.max_entries).map_err(|_| ReceiveError::Capacity)?
        {
            return Err(ReceiveError::Capacity);
        }
        if state
            .pending
            .values()
            .any(|entry| entry.manifest.request.revision == request.revision)
        {
            return Err(ReceiveError::Unavailable);
        }
        let staging = self.config.root.join("inbox").join(&id);
        if self
            .config
            .root
            .join("artifacts")
            .join(&id)
            .try_exists()
            .map_err(|_| ReceiveError::Unavailable)?
        {
            return Err(ReceiveError::Unavailable);
        }
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&staging)
            .map_err(|_| ReceiveError::Unavailable)?;
        let metadata = fs::symlink_metadata(&staging).map_err(|_| ReceiveError::Unavailable)?;
        let manifest = Manifest {
            version: 1,
            local: project(&request, &self.config.root.join("artifacts").join(&id)),
            request,
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        // Retain full responsibility even if manifest publication is unknown.
        state.pending.insert(
            id,
            Pending {
                manifest: manifest.clone(),
                downloading: true,
            },
        );
        let encoded = sandproof_controller_service::template_registry::evidence::encode_bounded(
            &manifest,
            MANIFEST_BYTES,
        )
        .map_err(|_| ReceiveError::Conflict)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o400)
            .open(staging.join("manifest.json"))
            .map_err(|_| ReceiveError::Unavailable)?;
        file.write_all(&encoded)
            .and_then(|()| file.sync_all())
            .map_err(|_| ReceiveError::Unavailable)?;
        File::open(&staging)
            .and_then(|file| file.sync_all())
            .map_err(|_| ReceiveError::Unavailable)?;
        File::open(self.config.root.join("inbox"))
            .and_then(|file| file.sync_all())
            .map_err(|_| ReceiveError::Unavailable)?;
        Ok(manifest)
    }
    async fn download(&self, manifest: &Manifest, staging: &Path) -> Result<(), ReceiveError> {
        for (part, name, artifact) in [
            (ArtifactPart::Kernel, "kernel", &manifest.local.kernel),
            (ArtifactPart::Rootfs, "rootfs.ext4", &manifest.local.rootfs),
        ] {
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(staging.join(name))
                .map_err(|_| ReceiveError::Unavailable)?;
            self.client
                .download_to(
                    &ArtifactRequest::from_cache(&manifest.request, part),
                    artifact,
                    &mut output,
                )
                .await
                .map_err(|_| ReceiveError::Unavailable)?;
            output.sync_all().map_err(|_| ReceiveError::Unavailable)?;
            output
                .set_permissions(fs::Permissions::from_mode(0o400))
                .map_err(|_| ReceiveError::Unavailable)?;
            output.sync_all().map_err(|_| ReceiveError::Unavailable)?;
        }
        let binding = project(&manifest.request, staging);
        self.store
            .verify_binding(&binding)
            .map_err(|_| ReceiveError::Unavailable)?;
        exact_directory(staging, manifest).map(|_| ())
    }
    fn publish(&self, manifest: Manifest, staging: &Path) -> Result<(), ReceiveError> {
        exact_directory(staging, &manifest)?;
        let destination = self
            .config
            .root
            .join("artifacts")
            .join(key(&manifest.request));
        if destination
            .try_exists()
            .map_err(|_| ReceiveError::Unavailable)?
        {
            return Err(ReceiveError::Unavailable);
        }
        fs::rename(staging, &destination).map_err(|_| ReceiveError::Unavailable)?;
        for directory in [
            self.config.root.join("inbox"),
            self.config.root.join("artifacts"),
        ] {
            File::open(directory)
                .and_then(|file| file.sync_all())
                .map_err(|_| ReceiveError::Unavailable)?;
        }
        self.store
            .verify_binding(&manifest.local)
            .map_err(|_| ReceiveError::Unavailable)?;
        let mut state = self.state.lock().map_err(|_| ReceiveError::Unavailable)?;
        state.ready_bytes = state
            .ready_bytes
            .checked_add(bytes(&manifest.local)?)
            .ok_or(ReceiveError::Capacity)?;
        state.pending.remove(&key(&manifest.request));
        state.ready.insert(
            manifest.request.revision.as_str().into(),
            InternalTemplate {
                revision: manifest.request.revision,
                digest: manifest.request.digest,
                registered: manifest.request.descriptor,
                local: manifest.local,
            },
        );
        Ok(())
    }
    fn reclaim_interrupted_downloads(&self) -> Result<(), ReceiveError> {
        self.lease
            .validate()
            .map_err(|_| ReceiveError::Unavailable)?;
        let state = self.state.lock().map_err(|_| ReceiveError::Unavailable)?;
        for entry in state.pending.values() {
            if entry.downloading {
                return Err(ReceiveError::Unavailable);
            }
            self.cleanup_partial(
                &entry.manifest,
                &self
                    .config
                    .root
                    .join("inbox")
                    .join(key(&entry.manifest.request)),
            )?;
        }
        Ok(())
    }
    fn cleanup_partial(&self, manifest: &Manifest, staging: &Path) -> Result<(), ReceiveError> {
        exact_directory(staging, manifest)?;
        // Never unlink the sole durable identity. A crash during these removals
        // remains recoverable from the same manifest, including an empty inbox.
        for name in ["kernel", "rootfs.ext4"] {
            match fs::remove_file(staging.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(ReceiveError::Unavailable),
            }
        }
        File::open(staging)
            .and_then(|file| file.sync_all())
            .map_err(|_| ReceiveError::Unavailable)
    }
}
impl<H: ControllerHost + Send + 'static> TemplateSource for TemplateCache<H> {
    fn bound(&self, binding: &CreateBinding) -> Result<InternalTemplate, ApplicationError> {
        let state = self.state.lock().map_err(|_| ApplicationError::Busy)?;
        state
            .ready
            .get(binding.template_revision.as_str())
            .filter(|value| value.digest == binding.template_digest)
            .cloned()
            .ok_or(ApplicationError::NotFound)
    }
}

#[cfg(test)]
mod tests;
