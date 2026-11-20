//! Full's exact finite store and source-output set, not the source VM disk.
use super::{SnapshotLayout, error};
use crate::linux::jail::{self, DirectoryIdentity};
use crate::{
    RuntimeError,
    snapshot_store::{CaptureStoreRetirement, material_retirement::RetiredDirectory},
};
use serde::{Deserialize, Serialize};
use std::fs::File;

const SOURCE_FILES: [&str; 2] = ["state.bin", "memory.bin"];

/// Persist before any unlink, under the original service work. Private fields
/// bind descriptors observed from the protected layout and original jail record.
/// This witness grants neither eligibility revocation nor permission to delete.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FullMaterialRetirement {
    version: u32,
    resource_id: String,
    executable: String,
    source_root: Option<DirectoryIdentity>,
    source_output: Option<RetiredDirectory>,
    store: CaptureStoreRetirement,
}
impl FullMaterialRetirement {
    #[must_use]
    pub fn directory_key(&self) -> &[u8; 32] {
        self.store.directory_key()
    }
    #[must_use]
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }
    #[must_use]
    pub fn executable(&self) -> &str {
        &self.executable
    }
    fn source_name(&self) -> String {
        use std::fmt::Write as _;
        let mut name = String::from("full-");
        for byte in self.directory_key() {
            write!(&mut name, "{byte:02x}").expect("String write");
        }
        name
    }
}
impl SnapshotLayout {
    /// Read only the bounded original manifest, without rehashing RAM or disk.
    /// This checks its bytes; it does not attest payloads or grant deletion.
    /// # Errors
    /// Missing, mutable, replaced or oversized manifest input is refused.
    pub fn retirement_manifest_sha256(&self, key: [u8; 32]) -> Result<[u8; 32], RuntimeError> {
        self.store_for_runtime(&self.runtime_root())?
            .retirement_manifest_sha256(key)
    }

    /// Read original-key output identities, including never-sealed stages.
    /// No current CPU, VM process, physical-slot mapping or old token is needed.
    /// # Errors
    /// Changed original layout/jail, foreign files or unsupported identities refuse.
    pub fn full_material_retirement_plan(
        &self,
        executable: &str,
        resource_id: &str,
        key: [u8; 32],
    ) -> Result<FullMaterialRetirement, RuntimeError> {
        let store = self
            .store_for_runtime(&self.runtime_root())?
            .capture_retirement_plan(key)?;
        let source =
            jail::capture_root_for_retirement(&self.runtime, executable, resource_id, self.owner)?;
        let mut plan = FullMaterialRetirement {
            version: 1,
            resource_id: resource_id.into(),
            executable: executable.into(),
            source_root: None,
            source_output: None,
            store,
        };
        if let Some((root, vm_uid)) = source {
            plan.source_root = Some(DirectoryIdentity::capture(&root)?);
            plan.source_output = RetiredDirectory::capture(
                &root,
                &plan.source_name(),
                &[self.owner, vm_uid],
                &SOURCE_FILES,
            )?;
        }
        Ok(plan)
    }
    fn retiring_source(&self, plan: &FullMaterialRetirement) -> Result<Option<File>, RuntimeError> {
        if plan.version != 1 || (plan.source_root.is_none() && plan.source_output.is_some()) {
            return Err(error("unsupported Full retirement plan shape"));
        }
        let Some((root, _)) = jail::capture_root_for_retirement(
            &self.runtime,
            &plan.executable,
            &plan.resource_id,
            self.owner,
        )?
        else {
            return Ok(None);
        };
        if plan.source_root.as_ref() != Some(&DirectoryIdentity::capture(&root)?) {
            return Err(error("Full retirement source root changed or appeared"));
        }
        RetiredDirectory::verify(
            &root,
            &plan.source_name(),
            plan.source_output.as_ref(),
            &SOURCE_FILES,
        )?;
        Ok(Some(root))
    }
    /// Delete only the durable plan's fixed material set. The caller separately
    /// excludes every original Full writer, loader and running backing user.
    /// This never stops a VM, removes rootfs, or releases a service reservation.
    /// # Errors
    /// Replacements, late output and partial unlink/fsync keep the same plan charged.
    pub fn retire_full_material(&self, plan: &FullMaterialRetirement) -> Result<(), RuntimeError> {
        let store = self.store_for_runtime(&self.runtime_root())?;
        store.verify_capture_retirement(&plan.store)?;
        let source = self.retiring_source(plan)?;
        if let Some(root) = source {
            RetiredDirectory::retire(
                &root,
                &plan.source_name(),
                plan.source_output.as_ref(),
                &SOURCE_FILES,
            )?;
        }
        store.retire_capture_material(&plan.store)?;
        if let Some(root) = self.retiring_source(plan)? {
            if crate::snapshot_store::material_retirement::directory_at(&root, &plan.source_name())?
                .is_some()
            {
                return Err(error("Full source output remains after retirement"));
            }
            root.sync_all()?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "snapshot_full_retirement_tests.rs"]
mod tests;
