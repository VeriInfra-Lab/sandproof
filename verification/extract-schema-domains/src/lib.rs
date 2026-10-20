//! Direct calls to the production types' fixed byte APIs. No validator is copied.
use sandproof_internal::status::{HostStatus, StatusRequest};

pub fn request_decode(bytes: &[u8]) -> Result<StatusRequest, serde_json::Error> {
    serde_json::from_slice::<StatusRequest>(bytes)
}

pub fn request_encode(request: &StatusRequest) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(request)
}

pub fn response_decode(bytes: &[u8]) -> Result<HostStatus, serde_json::Error> {
    serde_json::from_slice::<HostStatus>(bytes)
}

pub fn response_encode(response: &HostStatus) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(response)
}
