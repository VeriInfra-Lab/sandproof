//! Free roots call the same numeric helpers used by the product services.

pub fn seconds_to_milliseconds(seconds: u64) -> Option<u64> {
    sandproof_protocol::numeric::seconds_to_milliseconds(seconds)
}

pub fn bytes_to_mebibytes(bytes: u64) -> Option<u32> {
    sandproof_protocol::numeric::bytes_to_mebibytes(bytes)
}

pub fn available_resource(limit: u64, used: u64, observed: u64) -> u64 {
    sandproof_protocol::numeric::available_resource(limit, used, observed)
}
