//! Original material-plan consumer. Only the service's private permit authorizes
//! deletion; preparation and filesystem absence never settle a reservation.
use super::{LinuxWorker, NodeResult, core};
use super::{capture::require_source_binding, capture_profile::ProtectedCaptureProfile};
use crate::full_snapshot::capture_directory_key;
use sandbox_runtime::FullMaterialRetirement;
use sandproof_controller_service::capture::{
    CaptureMaterialRetirementRequest, MATERIAL_RETIREMENT_PLAN_MAX_BYTES,
    MaterialRetirementObservation, MaterialRetirementPermit,
};
use sandproof_protocol::lifecycle::ContentDigest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedMaterialRetirement {
    work_binding: ContentDigest,
    physical: FullMaterialRetirement,
}
fn bytes_digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_sha256(Sha256::digest(bytes).into())
}
fn bounded_plan(bytes: &[u8]) -> NodeResult<()> {
    if bytes.is_empty() || bytes.len() > MATERIAL_RETIREMENT_PLAN_MAX_BYTES {
        return Err("material retirement plan exceeds original bound".into());
    }
    Ok(())
}
impl LinuxWorker {
    fn material_request(&self, request: &CaptureMaterialRetirementRequest) -> NodeResult<()> {
        let full = request.full();
        require_source_binding(&full.pause, request.source())?;
        if request.sandbox().0 == 0 || request.generation() == 0 || !full.valid() {
            return Err("invalid original material retirement work".into());
        }
        if let Some(action) = request.seal_action() {
            super::decisions::validate_action_identity(action, &self.config.namespace)?;
            let core::Command::SealSnapshot {
                sandbox,
                generation,
                vm,
                ..
            } = action.command.command()
            else {
                return Err("material retirement has no original Seal action".into());
            };
            if *sandbox != request.sandbox()
                || *generation != request.generation()
                || self.vm_name(*vm) != full.pause.source_resource.id.as_str()
            {
                return Err("material retirement native work differs from original source".into());
            }
        }
        Ok(())
    }
    pub(crate) fn prepare_material_retirement(
        &self,
        profile: &ProtectedCaptureProfile,
        request: &CaptureMaterialRetirementRequest,
    ) -> NodeResult<Vec<u8>> {
        self.host_lease.validate()?;
        self.material_request(request)?;
        // Bind only the already adopted manifest; cleanup need not read/hash
        // the multi-GiB RAM and disk payloads again. This is not adoption.
        if let Some(expected) = request.manifest_digest()
            && profile
                .layout
                .retirement_manifest_sha256(capture_directory_key(&request.full().pause.binding)?)?
                != *expected.as_bytes()
        {
            return Err("retiring material manifest differs from original adoption".into());
        }
        let executable = self
            .config
            .firecracker
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .ok_or("protected Firecracker name")?;
        let value = SavedMaterialRetirement {
            work_binding: request.binding_digest()?,
            physical: profile.layout.full_material_retirement_plan(
                executable,
                request.full().pause.source_resource.id.as_str(),
                capture_directory_key(&request.full().pause.binding)?,
            )?,
        };
        let bytes = serde_json::to_vec(&value)?;
        bounded_plan(&bytes)?;
        Ok(bytes)
    }
    pub(crate) fn retire_material(
        &self,
        profile: &ProtectedCaptureProfile,
        request: &CaptureMaterialRetirementRequest,
        bytes: &[u8],
        permit: &MaterialRetirementPermit,
    ) -> NodeResult<MaterialRetirementObservation> {
        self.host_lease.validate()?;
        self.material_request(request)?;
        bounded_plan(bytes)?;
        let work_binding = request.binding_digest()?;
        let plan_binding = bytes_digest(bytes);
        if permit.work_binding() != work_binding || permit.plan_binding() != plan_binding {
            return Err("material deletion permit differs from original saved work or plan".into());
        }
        let plan: SavedMaterialRetirement = serde_json::from_slice(bytes)?;
        if plan.work_binding != work_binding
            || *plan.physical.directory_key()
                != capture_directory_key(&request.full().pause.binding)?
            || plan.physical.resource_id() != request.full().pause.source_resource.id.as_str()
            || Some(plan.physical.executable())
                != self
                    .config
                    .firecracker
                    .file_name()
                    .and_then(std::ffi::OsStr::to_str)
        {
            return Err("material retirement plan substituted another capture".into());
        }
        profile.layout.retire_full_material(&plan.physical)?;
        self.host_lease.validate()?;
        Ok(MaterialRetirementObservation {
            work_binding,
            plan_binding,
        })
    }
}
