//! Physical Full-end witness, not a journal registration or execution permit.
use super::{CaptureWorkerJob, NodeResult, RuntimeProcessIdentity, WorkAction, core};
use sandbox_runtime::SnapshotLayout;
use sandproof_controller_service::capture::{
    CaptureWorkerBinding, FullCaptureWorkRequest, SourceObservationRequirement,
    SourceProcessIdentity,
};
use sandproof_protocol::{ResourceRef, lifecycle::ContentDigest};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::linux) struct Completion {
    version: u16,
    action: WorkAction<core::EffectId>,
    full_work_binding: ContentDigest,
    job_digest: ContentDigest,
    worker: RuntimeProcessIdentity,
    source_resource: ResourceRef,
    source: SourceProcessIdentity,
    pub(in crate::linux) manifest_digest: ContentDigest,
    pub(in crate::linux) material_bytes: u64,
}

impl CaptureWorkerJob {
    pub(super) fn persist_completion(
        &self,
        layout: &SnapshotLayout,
        job_digest: ContentDigest,
        manifest_digest: ContentDigest,
        material_bytes: u64,
    ) -> NodeResult<()> {
        if material_bytes == 0 {
            return Err("empty completed Full material".into());
        }
        let record = Completion {
            version: 1,
            action: self.action.clone(),
            full_work_binding: self.full.binding_digest()?,
            job_digest,
            worker: RuntimeProcessIdentity::capture(std::process::id())?,
            source_resource: self.source.resource.clone(),
            source: self.source.source.clone(),
            manifest_digest,
            material_bytes,
        };
        layout.publish_capture_completion(
            crate::full_snapshot::capture_directory_key(&self.full.pause.binding)?,
            &serde_json::to_vec(&record)?,
        )?;
        Ok(())
    }
}

pub(in crate::linux) fn read_completion(
    layout: &SnapshotLayout,
    action: &WorkAction<core::EffectId>,
    full: &FullCaptureWorkRequest,
    source: &SourceObservationRequirement,
    worker: &CaptureWorkerBinding,
) -> NodeResult<Completion> {
    let bytes = layout.read_capture_completion(crate::full_snapshot::capture_directory_key(
        &full.pause.binding,
    )?)?;
    let record: Completion = serde_json::from_slice(&bytes)?;
    if record.version != 1
        || record.action != *action
        || record.full_work_binding != full.binding_digest()?
        || record.job_digest != *worker.job_digest()
        || record.worker.pid != worker.process().pid
        || record.worker.start_time_ticks != worker.process().start_time_ticks
        || record.worker.boot_id != worker.process().boot_id
        || record.source != source.source
        || record.source_resource != source.resource
        || record.material_bytes == 0
    {
        return Err(
            "durable physical completion differs from original SQL work/worker/source".into(),
        );
    }
    Ok(record)
}
