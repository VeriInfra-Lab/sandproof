//! Debug-only observations of the original Memory clone: SHA reads or sync boundaries.
//! No file/command creates an End, published Full, adoption or VM permission.
use super::restore_gate::{Directory, hex_digest, valid_digest};
use sandbox_controller_core as core;
use sandproof_controller_service::capture::{
    CaptureWorkerBinding, FullCaptureWorkRequest, SourceProcessIdentity,
};
use sandproof_protocol::{WorkAction, lifecycle::ContentDigest};
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fs::File,
    os::unix::fs::MetadataExt,
    path::PathBuf,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum SyncPhase {
    BeforeSync,
    AfterSync,
}

/// The helper inherits no environment; this one path enters its original job digest.
#[cfg(debug_assertions)]
pub(crate) fn configured_path() -> Result<Option<PathBuf>> {
    match (
        std::env::var_os("SANDPROOF_TEST_CAPTURE_MEASUREMENT_GATE"),
        std::env::var_os("SANDPROOF_TEST_CAPTURE_SYNC_GATE"),
    ) {
        (Some(_), Some(_)) => Err("one capture observation gate required".into()),
        (Some(path), None) | (None, Some(path)) => Ok(Some(path.into())),
        (None, None) => Ok(None),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema_version: u32,
    run_identity: String,
    sandbox: String,
    generation: u64,
    timeout_ms: u64,
    sync_phase: Option<SyncPhase>,
}
#[derive(Serialize)]
struct Observation<'a> {
    schema_version: u32,
    run_identity: &'a str,
    action: &'a WorkAction<core::EffectId>,
    full_work_binding: &'a ContentDigest,
    worker: &'a CaptureWorkerBinding,
    source: &'a SourceProcessIdentity,
    phase: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    capture_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measured_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_bytes: Option<u64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Instruction {
    schema_version: u32,
    run_identity: String,
    observation_sha256: String,
    full_work_binding: ContentDigest,
    command: String,
}
pub(crate) struct CaptureCopyGate {
    directory: Directory,
    config: Config,
    config_file: File,
    config_bytes: Vec<u8>,
}
impl CaptureCopyGate {
    pub(crate) fn configured(
        path: PathBuf,
        action: &WorkAction<core::EffectId>,
    ) -> Result<Option<Self>> {
        Self::select(Directory::open(path)?, action)
    }
    fn select(directory: Directory, action: &WorkAction<core::EffectId>) -> Result<Option<Self>> {
        let (config_file, config_bytes) = directory
            .read_file("config.json")?
            .ok_or("capture copy config missing")?;
        let config: Config = serde_json::from_slice(&config_bytes)?;
        if !matches!(
            (config.schema_version, config.sync_phase),
            (2, None) | (3, Some(_))
        ) || !valid_digest(&config.run_identity)
            || config.sandbox.is_empty()
            || config.sandbox.len() > 64
            || config.generation == 0
            || !(1000..=120_000).contains(&config.timeout_ms)
        {
            return Err("capture copy config identity or deadline differs".into());
        }
        if config.sandbox != action.identity.sandbox.as_str()
            || config.generation != action.identity.generation
        {
            return Ok(None);
        }
        if !matches!(action.command.command(), core::Command::SealSnapshot { .. }) {
            return Err("capture copy requires the exact original Full action".into());
        }
        // One successful selection only; later original actions run normally.
        if directory.read("consumed.json")?.is_some() {
            return Ok(None);
        }
        Ok(Some(Self {
            directory,
            config,
            config_file,
            config_bytes,
        }))
    }

    /// Only the real Runtime callback selects a cut, never a command's phase claim.
    pub(crate) fn progress(
        &self,
        artifact: sandbox_runtime::snapshot_store::SnapshotArtifact,
        event: sandbox_runtime::snapshot_store::SnapshotArtifactEvent,
    ) -> Option<(u64, u64)> {
        use sandbox_runtime::snapshot_store::{SnapshotArtifact, SnapshotArtifactEvent};
        if artifact != SnapshotArtifact::Memory {
            return None;
        }
        match (self.config.sync_phase, event) {
            (
                None,
                SnapshotArtifactEvent::Measuring {
                    measured_bytes,
                    total_bytes,
                },
            ) if measured_bytes > 0 && measured_bytes < total_bytes => {
                Some((measured_bytes, total_bytes))
            }
            (Some(SyncPhase::BeforeSync), SnapshotArtifactEvent::BeforeSync)
            | (Some(SyncPhase::AfterSync), SnapshotArtifactEvent::Synced) => Some((0, 0)),
            _ => None,
        }
    }

    pub(crate) fn wait(
        &self,
        action: &WorkAction<core::EffectId>,
        full: &FullCaptureWorkRequest,
        worker: &CaptureWorkerBinding,
        measured_bytes: u64,
        total_bytes: u64,
    ) -> Result<()> {
        if !full.valid() {
            return Err("copy gate original Full is invalid".into());
        }
        self.wait_question(
            action,
            full.binding_digest()?,
            &full.pause.source,
            worker,
            (measured_bytes, total_bytes),
            if self.config.sync_phase.is_some() {
                Some(
                    crate::full_snapshot::capture_directory_key(&full.pause.binding)?
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect(),
                )
            } else {
                None
            },
        )
    }

    fn wait_question(
        &self,
        action: &WorkAction<core::EffectId>,
        full_work_binding: ContentDigest,
        source: &SourceProcessIdentity,
        worker: &CaptureWorkerBinding,
        progress: (u64, u64),
        capture_key: Option<String>,
    ) -> Result<()> {
        let (measured_bytes, total_bytes) = progress;
        let deadline = Instant::now() + Duration::from_millis(self.config.timeout_ms);
        if match self.config.sync_phase {
            None => measured_bytes == 0 || measured_bytes >= total_bytes,
            Some(_) => progress != (0, 0) || !capture_key.as_deref().is_some_and(valid_digest),
        } {
            return Err(
                "capture gate requires incomplete SHA measurement of the complete clone".into(),
            );
        }
        if action.identity.sandbox.as_str() != self.config.sandbox
            || action.identity.generation != self.config.generation
            || !matches!(action.command.command(), core::Command::SealSnapshot { .. })
            || !worker.valid()
        {
            return Err("capture copy original action/worker differs".into());
        }
        let bytes = serde_json::to_vec(&Observation {
            schema_version: self.config.schema_version,
            run_identity: &self.config.run_identity,
            action,
            full_work_binding: &full_work_binding,
            worker,
            source,
            phase: match self.config.sync_phase {
                None => "measuring-cloned-artifact",
                Some(SyncPhase::BeforeSync) => "memory-before-sync",
                Some(SyncPhase::AfterSync) => "memory-after-sync",
            },
            capture_key,
            measured_bytes: self.config.sync_phase.is_none().then_some(measured_bytes),
            total_bytes: self.config.sync_phase.is_none().then_some(total_bytes),
        })?;
        if bytes.len() > 16_384 {
            return Err("capture copy original question exceeds bound".into());
        }
        let digest = hex_digest(&bytes);
        let file = self.directory.create("observation.json", &bytes)?;
        let original = file.metadata()?;
        self.directory.publish_ready(&digest)?;
        loop {
            self.directory.matches("config.json", &self.config_file)?;
            self.directory.matches("observation.json", &file)?;
            let current = file.metadata()?;
            if (
                current.len(),
                current.ctime(),
                current.ctime_nsec(),
                current.mode(),
                current.uid(),
                current.nlink(),
            ) != (
                original.len(),
                original.ctime(),
                original.ctime_nsec(),
                original.mode(),
                original.uid(),
                original.nlink(),
            ) || self.directory.read("config.json")?.as_deref()
                != Some(self.config_bytes.as_slice())
            {
                return Err("capture copy original question changed".into());
            }
            if Instant::now() >= deadline {
                return Err("capture copy deadline exceeded; Full remains unknown".into());
            }
            if let Some(bytes) = self.directory.read("command.json")? {
                let command: Instruction = serde_json::from_slice(&bytes)?;
                if command.schema_version != self.config.schema_version
                    || command.run_identity != self.config.run_identity
                    || command.observation_sha256 != digest
                    || command.full_work_binding != full_work_binding
                    || !matches!(command.command.as_str(), "release" | "fail")
                {
                    return Err(
                        "capture copy command differs from the complete original question".into(),
                    );
                }
                self.directory.create("consumed.json", &bytes)?;
                return if command.command == "release" {
                    Ok(())
                } else {
                    Err(
                        "capture gate requested failure at original materialization boundary"
                            .into(),
                    )
                };
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests;
