//! One debug-only cancellation before the original Full release pipe opens.
//! This publishes a question, never `NoRequest`, an End receipt or VM success.
use super::restore_gate::{Directory, hex_digest, valid_digest};
use sandbox_controller_core as core;
use sandproof_controller_service::capture::{CaptureWorkerBinding, FullCaptureLaunchPermit};
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema_version: u32,
    run_identity: String,
    sandbox: String,
    generation: u64,
    timeout_ms: u64,
}
#[derive(Serialize)]
struct Observation<'a> {
    schema_version: u32,
    run_identity: &'a str,
    action: &'a WorkAction<core::EffectId>,
    full_work_binding: &'a ContentDigest,
    worker: &'a CaptureWorkerBinding,
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
pub(crate) struct CaptureCancelGate {
    directory: Directory,
    config: Config,
    config_file: File,
    config_bytes: Vec<u8>,
}
impl CaptureCancelGate {
    pub(crate) fn configured(action: &WorkAction<core::EffectId>) -> Result<Option<Self>> {
        let Some(path) = std::env::var_os("SANDPROOF_TEST_CAPTURE_CANCEL_GATE") else {
            return Ok(None);
        };
        Self::select(Directory::open(PathBuf::from(path))?, action)
    }
    fn select(directory: Directory, action: &WorkAction<core::EffectId>) -> Result<Option<Self>> {
        let (config_file, config_bytes) = directory
            .read_file("config.json")?
            .ok_or("capture cancel config missing")?;
        let config: Config = serde_json::from_slice(&config_bytes)?;
        if config.schema_version != 1
            || !valid_digest(&config.run_identity)
            || config.sandbox.is_empty()
            || config.sandbox.len() > 64
            || config.generation == 0
            || !(1000..=120_000).contains(&config.timeout_ms)
        {
            return Err("capture cancel config identity or deadline differs".into());
        }
        if config.sandbox != action.identity.sandbox.as_str()
            || config.generation != action.identity.generation
        {
            return Ok(None);
        }
        if !matches!(action.command.command(), core::Command::SealSnapshot { .. }) {
            return Err("capture cancel requires the exact original Full action".into());
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

    pub(crate) fn wait(&self, permit: &FullCaptureLaunchPermit) -> Result<()> {
        self.wait_for(
            permit.action(),
            permit.full_work_binding(),
            permit.worker(),
            Instant::now() + Duration::from_millis(self.config.timeout_ms),
        )
    }

    fn wait_for(
        &self,
        action: &WorkAction<core::EffectId>,
        full_work_binding: &ContentDigest,
        worker: &CaptureWorkerBinding,
        deadline: Instant,
    ) -> Result<()> {
        if action.identity.sandbox.as_str() != self.config.sandbox
            || action.identity.generation != self.config.generation
            || !matches!(action.command.command(), core::Command::SealSnapshot { .. })
            || !worker.valid()
        {
            return Err("capture cancel original action/worker differs".into());
        }
        let bytes = serde_json::to_vec(&Observation {
            schema_version: 1,
            run_identity: &self.config.run_identity,
            action,
            full_work_binding,
            worker,
        })?;
        if bytes.len() > 16_384 {
            return Err("capture cancel original question exceeds bound".into());
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
                return Err("capture cancel original question changed".into());
            }
            if Instant::now() >= deadline {
                return Err("capture cancel deadline exceeded; Full remains unknown".into());
            }
            if let Some(bytes) = self.directory.read("command.json")? {
                let command: Instruction = serde_json::from_slice(&bytes)?;
                if command.schema_version != 1
                    || command.run_identity != self.config.run_identity
                    || command.observation_sha256 != digest
                    || command.full_work_binding != *full_work_binding
                    || command.command != "cancel"
                {
                    return Err(
                        "capture cancel command differs from the complete original question".into(),
                    );
                }
                self.directory.create("consumed.json", &bytes)?;
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests;
