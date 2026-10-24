//! The original Full API returned; not a sealed bundle or a continuation permit.
use super::{Runtime, RuntimeError, RuntimeProcessIdentity, SnapshotLayout};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Original controller/worker identities supplied before the physical call.
/// Public construction does not attest SQL authority or physical completion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FullCaptureApiContext {
    pub work_sha256: [u8; 32],
    pub job_sha256: [u8; 32],
    pub action_sha256: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FullCaptureApiEnd {
    version: u32,
    resource_id: String,
    source: RuntimeProcessIdentity,
    directory_key: [u8; 32],
    context: FullCaptureApiContext,
    worker: RuntimeProcessIdentity,
    request_sha256: [u8; 32],
    status: u16,
    response_sha256: [u8; 32],
}

fn invalid(message: &'static str) -> RuntimeError {
    RuntimeError::IdentityMismatch(message)
}
pub(super) fn full_capture_request(key: [u8; 32]) -> Result<Vec<u8>, RuntimeError> {
    use std::fmt::Write as _;
    let mut name = String::from("full-");
    for byte in key {
        write!(&mut name, "{byte:02x}").map_err(|_| invalid("Full key encoding"))?;
    }
    Ok(serde_json::to_vec(
        &serde_json::json!({"snapshot_type":"Full",
        "snapshot_path":format!("/{name}/state.bin"),"mem_file_path":format!("/{name}/memory.bin")}),
    )?)
}
impl FullCaptureApiEnd {
    pub(super) fn returned(
        id: &str,
        source: &RuntimeProcessIdentity,
        key: [u8; 32],
        context: FullCaptureApiContext,
        worker: RuntimeProcessIdentity,
        status: u16,
        response: &[u8],
    ) -> Result<Self, RuntimeError> {
        if !(200..600).contains(&status) {
            return Err(invalid("Full API did not return a final status"));
        }
        source.validate_fields()?;
        worker.validate_fields()?;
        Ok(Self {
            version: 1,
            resource_id: id.into(),
            source: source.clone(),
            directory_key: key,
            context,
            worker,
            request_sha256: Runtime::full_capture_request_sha256(key)?,
            status,
            response_sha256: Sha256::digest(response).into(),
        })
    }
    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }
    #[must_use]
    pub const fn directory_key(&self) -> &[u8; 32] {
        &self.directory_key
    }
    /// Validate against separately persisted original work; no field of this
    /// record supplies its own expected action, source or worker identity.
    /// # Errors
    /// Changed/foreign inputs never establish that the caller's API ended.
    pub fn require_original(
        &self,
        id: &str,
        source: &RuntimeProcessIdentity,
        key: [u8; 32],
        context: FullCaptureApiContext,
        worker: &RuntimeProcessIdentity,
    ) -> Result<(), RuntimeError> {
        if self.version != 1
            || self.resource_id != id
            || self.source != *source
            || self.directory_key != key
            || self.context != context
            || self.worker != *worker
            || !(200..600).contains(&self.status)
            || self.request_sha256 != Runtime::full_capture_request_sha256(key)?
        {
            return Err(invalid(
                "Full API-end differs from original work/request/source/worker",
            ));
        }
        Ok(())
    }
}
impl Runtime {
    /// The exact canonical request sent by the fixed Full provider.
    /// # Errors
    /// Encoding failure supplies no request identity.
    pub fn full_capture_request_sha256(key: [u8; 32]) -> Result<[u8; 32], RuntimeError> {
        Ok(Sha256::digest(full_capture_request(key)?).into())
    }
}
impl SnapshotLayout {
    pub(super) fn publish_full_api_end(
        &self,
        value: &FullCaptureApiEnd,
    ) -> Result<(), RuntimeError> {
        self.store_for_runtime(&self.runtime_root())?
            .publish_api_end(value.directory_key, &serde_json::to_vec(value)?)
    }
    /// Read the original bounded API-end record, never dispatching any request.
    /// # Errors
    /// Missing/changed/partial publications remain unknown.
    pub fn read_full_api_end(&self, key: [u8; 32]) -> Result<FullCaptureApiEnd, RuntimeError> {
        let bytes = self
            .store_for_runtime(&self.runtime_root())?
            .read_api_end(key)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32) -> RuntimeProcessIdentity {
        RuntimeProcessIdentity {
            pid,
            start_time_ticks: u64::from(pid),
            boot_id: "00000000-0000-0000-0000-000000000001".into(),
        }
    }

    #[test]
    fn only_original_final_reply_matches_all_saved_identities() {
        let context = FullCaptureApiContext {
            work_sha256: [1; 32],
            job_sha256: [2; 32],
            action_sha256: [3; 32],
        };
        let source = process(10);
        let worker = process(11);
        for status in [200, 204, 400, 500, 599] {
            let record = FullCaptureApiEnd::returned(
                "source-1",
                &source,
                [4; 32],
                context,
                worker.clone(),
                status,
                b"reply",
            )
            .unwrap();
            let record: FullCaptureApiEnd =
                serde_json::from_slice(&serde_json::to_vec(&record).unwrap()).unwrap();
            record
                .require_original("source-1", &source, [4; 32], context, &worker)
                .unwrap();
            assert!(
                record
                    .require_original("source-2", &source, [4; 32], context, &worker)
                    .is_err()
            );
            assert!(
                record
                    .require_original("source-1", &process(12), [4; 32], context, &worker)
                    .is_err()
            );
            assert!(
                record
                    .require_original("source-1", &source, [5; 32], context, &worker)
                    .is_err()
            );
            assert!(
                record
                    .require_original("source-1", &source, [4; 32], context, &process(12))
                    .is_err()
            );
            for field in 0..3 {
                let mut changed = context;
                match field {
                    0 => changed.work_sha256 = [9; 32],
                    1 => changed.job_sha256 = [9; 32],
                    _ => changed.action_sha256 = [9; 32],
                }
                assert!(
                    record
                        .require_original("source-1", &source, [4; 32], changed, &worker)
                        .is_err()
                );
            }
        }
        for status in [0, 100, 199, 600] {
            assert!(
                FullCaptureApiEnd::returned(
                    "source-1",
                    &source,
                    [4; 32],
                    context,
                    worker.clone(),
                    status,
                    b""
                )
                .is_err()
            );
        }
    }
}
