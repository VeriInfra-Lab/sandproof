//! One original Full request ended without being sent or with a complete response.
//! Neither outcome is a sealed bundle or a continuation permit.
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
pub struct FullCaptureRequestEnd {
    version: u32,
    resource_id: String,
    source: RuntimeProcessIdentity,
    directory_key: [u8; 32],
    context: FullCaptureApiContext,
    worker: RuntimeProcessIdentity,
    request_sha256: [u8; 32],
    outcome: FullCaptureRequestOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum FullCaptureRequestOutcome {
    NoRequest,
    Returned { http_status: u16, response_sha256: [u8; 32] },
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
impl FullCaptureRequestEnd {
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
            outcome: FullCaptureRequestOutcome::Returned {
                http_status: status,
                response_sha256: Sha256::digest(response).into(),
            },
        })
    }
    fn no_request(
        id: &str, source: &RuntimeProcessIdentity, key: [u8; 32],
        context: FullCaptureApiContext,
    ) -> Result<Self, RuntimeError> {
        source.validate_fields()?;
        Ok(Self {
            version: 1, resource_id: id.into(), source: source.clone(),
            directory_key: key, context,
            worker: RuntimeProcessIdentity::capture(std::process::id())?,
            request_sha256: Runtime::full_capture_request_sha256(key)?,
            outcome: FullCaptureRequestOutcome::NoRequest,
        })
    }
    /// Check both tagged outcomes before Ready can precede SQL authorization.
    /// # Errors
    /// Oversize original bindings cannot create an unreserved future end record.
    pub fn validate_capacity(
        id: &str, source: &RuntimeProcessIdentity, key: [u8; 32],
        context: FullCaptureApiContext,
    ) -> Result<(), RuntimeError> {
        let mut record = Self::no_request(id, source, key, context)?;
        for outcome in [FullCaptureRequestOutcome::NoRequest,
            FullCaptureRequestOutcome::Returned { http_status: 599, response_sha256: [255; 32] }] {
            record.outcome = outcome;
            if serde_json::to_vec(&record)?.len() as u64 > crate::snapshot_store::MAX_COMPLETION_BYTES {
                return Err(invalid("Full request end exceeds original record budget"));
            }
        }
        Ok(())
    }
    #[must_use]
    pub const fn outcome(&self) -> FullCaptureRequestOutcome {
        self.outcome
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
            || matches!(self.outcome, FullCaptureRequestOutcome::Returned { http_status, .. } if !(200..600).contains(&http_status))
            || self.request_sha256 != Runtime::full_capture_request_sha256(key)?
        {
            return Err(invalid(
                "Full request-end differs from original work/request/source/worker",
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
    /// Ready cannot precede a second Full in an already used original end slot.
    /// # Errors
    /// Existing canonical or partial records remain charged and are not removed.
    pub fn require_full_request_unstarted(&self, key: [u8; 32]) -> Result<(), RuntimeError> {
        self.store_for_runtime(&self.runtime_root())?.require_request_end_absent(key)
    }

    /// The complete original worker irrevocably declines its release before
    /// sending any Full bytes. This primitive does not attest that decision.
    /// # Errors
    /// Publication failures retain responsibility and cannot supply end facts.
    pub fn publish_full_no_request(
        &self, id: &str, source: &RuntimeProcessIdentity, key: [u8; 32],
        context: FullCaptureApiContext,
    ) -> Result<(), RuntimeError> {
        let record = FullCaptureRequestEnd::no_request(id, source, key, context)?;
        self.publish_full_request_end(&record, source)
    }

    // The fixed physical slot retains its existing prefix. Only this tagged
    // payload distinguishes NoRequest from Returned; the filename proves neither.
    pub(super) fn publish_full_request_end(
        &self,
        value: &FullCaptureRequestEnd,
        source: &RuntimeProcessIdentity,
    ) -> Result<(), RuntimeError> {
        self.store_for_runtime(&self.runtime_root())?
            .publish_retained_request_end(value.directory_key, &serde_json::to_vec(value)?, || {
                source.is_alive()
            })
    }
    /// Read the original bounded request-end record, never dispatching any request.
    /// # Errors
    /// Missing/changed/partial publications remain unknown.
    pub fn read_full_request_end(&self, key: [u8; 32]) -> Result<FullCaptureRequestEnd, RuntimeError> {
        let bytes = self
            .store_for_runtime(&self.runtime_root())?
            .read_request_end(key)?;
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
    fn no_request_is_tagged_bound_and_survives_two_record_reopens() {
        use crate::snapshot_store::SnapshotStore;
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let owner = rustix::process::geteuid().as_raw();
        let source = process(10);
        let context = FullCaptureApiContext { work_sha256: [255;32], job_sha256: [255;32], action_sha256: [255;32] };
        let key = [255;32];
        let record = FullCaptureRequestEnd::no_request("source-1", &source, key, context).unwrap();
        let worker = record.worker.clone();
        let bytes = serde_json::to_vec(&record).unwrap();
        assert!(bytes.len() as u64 <= crate::snapshot_store::MAX_COMPLETION_BYTES);
        let store = SnapshotStore::open_for_owner(root.path(), owner).unwrap();
        store.require_request_end_absent(key).unwrap();
        store.publish_retained_request_end(key, &bytes, || Ok(true)).unwrap();
        assert!(store.require_request_end_absent(key).is_err());
        drop(store);
        for _ in 0..2 {
            let reopened = SnapshotStore::open_for_owner(root.path(), owner).unwrap();
            let actual: FullCaptureRequestEnd = serde_json::from_slice(&reopened.read_request_end(key).unwrap()).unwrap();
            assert_eq!(actual.outcome(), FullCaptureRequestOutcome::NoRequest);
            actual.require_original("source-1", &source, key, context, &worker).unwrap();
            assert!(actual.require_original("source-1", &source, key, context, &process(11)).is_err());
        }
    }

    #[test]
    fn untagged_end_is_rejected_and_future_outcomes_fit_reserved_budget() {
        let source = process(10);
        let context = FullCaptureApiContext { work_sha256: [255;32], job_sha256: [255;32], action_sha256: [255;32] };
        let key = [255;32];
        let record = FullCaptureRequestEnd::no_request("source-1", &source, key, context).unwrap();
        let mut old = serde_json::to_value(&record).unwrap();
        old.as_object_mut().unwrap().remove("outcome");
        old["status"] = serde_json::json!(400);
        old["response_sha256"] = serde_json::to_value([0_u8;32]).unwrap();
        assert!(serde_json::from_value::<FullCaptureRequestEnd>(old).is_err());
        FullCaptureRequestEnd::validate_capacity(&"x".repeat(64), &source, key, context).unwrap();
        assert!(FullCaptureRequestEnd::validate_capacity(&"x".repeat(16_384), &source, key, context).is_err());
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
            let record = FullCaptureRequestEnd::returned(
                "source-1",
                &source,
                [4; 32],
                context,
                worker.clone(),
                status,
                b"reply",
            )
            .unwrap();
            let record: FullCaptureRequestEnd =
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
                FullCaptureRequestEnd::returned(
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
