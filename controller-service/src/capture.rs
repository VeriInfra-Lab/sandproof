//! Capture values shared by the original durable controller and host adapter.
//!
//! Physical observations are not database commit receipts or Stop permits.
//! The controller must bind them to its original work and commit that work
//! before a host effect; deserializing these values grants no authority.

use serde::{Deserialize, Serialize};

/// Identity observed for the exact original VMM process, including its boot.
/// PID reuse, process-start reuse across boots and manifest self-reporting
/// cannot substitute for a command-specific original host observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceProcessIdentity {
    pub pid: u32,
    pub start_time_ticks: u64,
    pub boot_id: String,
}
