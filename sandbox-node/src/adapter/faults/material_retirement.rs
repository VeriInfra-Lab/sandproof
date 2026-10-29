//! One debug-only cut after real material deletion, before receipt delivery.
//! Gate files never authorize deletion, close a reference or refund a byte.
use super::restore_gate::{Directory, hex_digest, valid_digest};
use sandproof_controller_service::capture::{
    CaptureMaterialRetirementRequest, MATERIAL_RETIREMENT_PLAN_MAX_BYTES,
    MaterialRetirementObservation,
};
use sandproof_protocol::ResourceRef;
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fmt::Write,
    fs::File,
    os::unix::fs::MetadataExt,
    path::PathBuf,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const PHASE: &str = "material-deleted-before-settlement";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema_version: u32,
    run_identity: String,
    sandbox: u64,
    generation: u64,
    capture_key: String,
    timeout_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Material {
    sandbox: u64,
    generation: u64,
    capture_key: String,
    work_binding: String,
    plan_binding: String,
    plan_bytes: usize,
}
#[derive(Serialize)]
struct Observation<'a> {
    schema_version: u32,
    run_identity: &'a str,
    phase: &'static str,
    material: &'a Material,
    source_resource: &'a ResourceRef,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Consumption {
    schema_version: u32,
    run_identity: String,
    observation_sha256: String,
    material: Material,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Instruction {
    schema_version: u32,
    run_identity: String,
    observation_sha256: String,
    material: Material,
    command: String,
}
struct Gate {
    directory: Directory,
    config: Config,
    config_file: File,
    config_bytes: Vec<u8>,
}
fn raw_hex(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(2 * bytes.len());
    for byte in bytes {
        write!(&mut result, "{byte:02x}").expect("String write");
    }
    result
}
fn valid_capture(value: &str) -> bool {
    value.strip_prefix("capture-").is_some_and(valid_digest)
}
fn observation_bytes(
    run_identity: &str,
    material: &Material,
    source_resource: &ResourceRef,
) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(&Observation {
        schema_version: 1,
        run_identity,
        phase: PHASE,
        material,
        source_resource,
    })?;
    if bytes.len() > 16_384 {
        return Err("material gate observation exceeds original bound".into());
    }
    Ok(bytes)
}

pub(super) fn after_material_delete(
    request: &CaptureMaterialRetirementRequest,
    plan: &[u8],
    observation: &MaterialRetirementObservation,
) -> Result<()> {
    let Some(path) = std::env::var_os("SANDPROOF_TEST_MATERIAL_RETIREMENT_GATE") else {
        return Ok(());
    };
    let work_binding = request.binding_digest()?;
    let plan_binding = hex_digest(plan);
    if plan.is_empty()
        || plan.len() > MATERIAL_RETIREMENT_PLAN_MAX_BYTES
        || observation.work_binding != work_binding
        || raw_hex(observation.plan_binding.as_bytes()) != plan_binding
    {
        return Err("material gate differs from the actual physical observation".into());
    }
    let material = Material {
        sandbox: request.sandbox().0,
        generation: request.generation(),
        capture_key: format!(
            "capture-{}",
            raw_hex(&crate::full_snapshot::capture_directory_key(
                &request.full().pause.binding
            )?)
        ),
        work_binding: raw_hex(work_binding.as_bytes()),
        plan_binding,
        plan_bytes: plan.len(),
    };
    if let Some(gate) = Gate::select(
        Directory::open(PathBuf::from(path))?,
        &material,
        &request.full().pause.source_resource,
    )? {
        gate.wait(&material, &request.full().pause.source_resource)?;
    }
    Ok(())
}

impl Gate {
    fn select(
        directory: Directory,
        material: &Material,
        source_resource: &ResourceRef,
    ) -> Result<Option<Self>> {
        let (config_file, config_bytes) = directory
            .read_file("config.json")?
            .ok_or("material gate config missing")?;
        let config: Config = serde_json::from_slice(&config_bytes)?;
        if config.schema_version != 1
            || !valid_digest(&config.run_identity)
            || config.sandbox == 0
            || config.generation == 0
            || !valid_capture(&config.capture_key)
            || !(1000..=120_000).contains(&config.timeout_ms)
        {
            return Err("material gate config identity or deadline differs".into());
        }
        if (config.sandbox, config.generation, &config.capture_key)
            != (material.sandbox, material.generation, &material.capture_key)
        {
            return Ok(None);
        }
        if let Some(bytes) = directory.read("consumed.json")? {
            let consumed: Consumption = serde_json::from_slice(&bytes)?;
            if consumed.schema_version != 1
                || consumed.run_identity != config.run_identity
                || !valid_digest(&consumed.observation_sha256)
                || consumed.material != *material
            {
                return Err("material gate consumed another work or saved plan".into());
            }
            let original = directory
                .read_observation()?
                .ok_or("material gate consumed observation is missing")?;
            if hex_digest(&original) != consumed.observation_sha256
                || original != observation_bytes(&config.run_identity, material, source_resource)?
            {
                return Err("material gate consumed observation bytes or binding changed".into());
            }
            // The caller has just repeated real physical deletion/absence
            // verification. Only this testing wait is one-shot on restart.
            // No ready file is inferred or created for a pre-ready crash.
            return Ok(None);
        }
        Ok(Some(Self {
            directory,
            config,
            config_file,
            config_bytes,
        }))
    }

    fn wait(&self, material: &Material, source_resource: &ResourceRef) -> Result<()> {
        let deadline = Instant::now() + Duration::from_millis(self.config.timeout_ms);
        let bytes = observation_bytes(&self.config.run_identity, material, source_resource)?;
        let digest = hex_digest(&bytes);
        let file = self.directory.create("observation.json", &bytes)?;
        let original = file.metadata()?;
        // Persist this cut before publishing ready. A crash before ready is
        // not a reached-cut observation and cannot be repaired by the harness.
        self.directory.create(
            "consumed.json",
            &serde_json::to_vec(&Consumption {
                schema_version: 1,
                run_identity: self.config.run_identity.clone(),
                observation_sha256: digest.clone(),
                material: material.clone(),
            })?,
        )?;
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
                return Err("material gate original question changed".into());
            }
            if Instant::now() >= deadline {
                return Err("material gate deadline exceeded; settlement remains unknown".into());
            }
            if let Some(bytes) = self.directory.read("command.json")? {
                let command: Instruction = serde_json::from_slice(&bytes)?;
                if command.schema_version != 1
                    || command.run_identity != self.config.run_identity
                    || command.observation_sha256 != digest
                    || command.material != *material
                    || !matches!(command.command.as_str(), "release" | "fail")
                {
                    return Err("material gate command differs from the complete saved plan".into());
                }
                return if command.command == "release" {
                    Ok(())
                } else {
                    Err("material gate requested failure before settlement receipt".into())
                };
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests;
