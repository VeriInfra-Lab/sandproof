//! Explicit operator-selected budgets for the sole formal streaming profile.
//! Low-level transport tests may omit limits; the formal server cannot do so.

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransferLimits {
    pub file_upload_bytes: usize,
    pub file_download_bytes: usize,
    pub command_input_bytes: usize,
    pub rpc_request_bytes: usize,
    pub rpc_response_bytes: usize,
    pub max_transfers: usize,
    pub max_transfers_per_vm: usize,
    /// Included in both maxima; file/Start traffic cannot consume these slots.
    pub control_reserve: usize,
    pub request_idle_seconds: u64,
    pub file_idle_seconds: u64,
    pub write_idle_seconds: u64,
}

impl TransferLimits {
    #[cfg(test)]
    pub fn fixture() -> Self {
        // Test data, not claimed production defaults or measured task needs.
        Self {
            file_upload_bytes: 64 * 1024 * 1024,
            file_download_bytes: 96 * 1024 * 1024,
            command_input_bytes: super::PROCESS_REQUEST_LIMIT,
            rpc_request_bytes: 64 * 1024,
            rpc_response_bytes: 1024 * 1024,
            max_transfers: 8,
            max_transfers_per_vm: 4,
            control_reserve: 1,
            request_idle_seconds: 5,
            file_idle_seconds: 10,
            write_idle_seconds: 5,
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if [
            self.file_upload_bytes,
            self.file_download_bytes,
            self.command_input_bytes,
            self.rpc_request_bytes,
            self.rpc_response_bytes,
        ]
        .contains(&0)
        {
            return Err("data transfer budgets must be positive");
        }
        // Files stream; filesystem RPC metadata is still inspected in memory.
        // Raising a file budget must never also raise that allocation bound.
        if self.rpc_request_bytes > super::FILE_REQUEST_LIMIT {
            return Err("filesystem RPC request budget cannot exceed 8 MiB");
        }
        if self.control_reserve == 0
            || self.control_reserve >= self.max_transfers_per_vm
            || self.max_transfers_per_vm > self.max_transfers
        {
            return Err("transfer concurrency must leave control capacity globally and per VM");
        }
        for seconds in [
            self.request_idle_seconds,
            self.file_idle_seconds,
            self.write_idle_seconds,
        ] {
            if seconds == 0
                || std::time::Instant::now()
                    .checked_add(std::time::Duration::from_secs(seconds))
                    .is_none()
            {
                return Err("transfer idle timeouts must be positive and representable");
            }
        }
        Ok(())
    }
}
