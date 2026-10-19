//! Compile-time interface identifiers. These are not a binary/source digest or
//! evidence of compatibility, successful execution or a deployed template.
pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const WORKER_PROTOCOL_VERSION: u32 = 2;
pub const PID_FORMAT_VERSION: u8 = 2;

/// Machine-readable information without opening configuration or a gate.
#[must_use]
pub fn worker() -> serde_json::Value {
    serde_json::json!({
        "component": "sandproof-guest-worker",
        "package_version": PACKAGE_VERSION,
        "worker_protocol_version": WORKER_PROTOCOL_VERSION,
        "target_os": std::env::consts::OS,
        "target_arch": std::env::consts::ARCH,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_identifiers_match_registered_inputs() {
        let lock: serde_json::Value =
            serde_json::from_str(include_str!("../../references/v1-baseline.json")).unwrap();
        let guest = &lock["code_baseline"]["rust_guest"];
        assert_eq!(worker()["package_version"], guest["worker_package_version"]);
        assert_eq!(
            worker()["worker_protocol_version"],
            guest["worker_protocol_version"]
        );
        assert_eq!(
            u64::from(PID_FORMAT_VERSION),
            guest["pid_format_version"].as_u64().unwrap()
        );
    }
}
