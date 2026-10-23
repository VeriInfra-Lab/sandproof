//! Concrete call-through to the original bounded serializer; no writer is copied.
use sandproof_controller_service::{JournalError, template_registry::evidence::encode_bounded};
use sandproof_internal::status::HostStatus;

pub fn encode_status(value: &HostStatus, limit: usize) -> Result<Vec<u8>, JournalError> {
    encode_bounded(value, limit)
}

// Enabled only in the prepared private candidate, never in production.
#[cfg(feature = "candidate")]
pub mod candidate {
    use sandproof_controller_service::{
        JournalError,
        template_registry::evidence::{
            BoundedBuffer, BoundedWriter, finish_bounded, write_bounded,
        },
    };
    use sandproof_internal::{ProofJsonResponseBuilder, proof_json_response_result};

    pub fn write_entry<B: BoundedBuffer>(
        writer: &mut BoundedWriter<B>,
        bytes: &[u8],
    ) -> std::io::Result<usize> {
        write_bounded(writer, bytes)
    }

    pub fn finish_entry<B: BoundedBuffer>(
        writer: BoundedWriter<B>,
        result: Result<(), serde_json::Error>,
    ) -> Result<Vec<u8>, JournalError> {
        finish_bounded(writer, result)
    }

    pub fn response_entry<P: ProofJsonResponseBuilder>(
        result: Result<Vec<u8>, JournalError>,
    ) -> P::Response {
        proof_json_response_result::<P>(result)
    }
}
