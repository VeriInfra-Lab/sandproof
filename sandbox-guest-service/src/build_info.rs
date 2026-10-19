//! Local interface identifiers, independent of Go envd's SDK negotiation
//! version. Binary/source fingerprints belong to the trusted template inputs.
pub const CONFIG_VERSION: u32 = 3;

/// This does not read configuration, contact MMDS or start a service/worker.
#[must_use]
pub fn service() -> serde_json::Value {
    use sandproof_guest_runtime::build_info as worker;
    serde_json::json!({
        "component": "sandproof-guest-service",
        "package_version": env!("CARGO_PKG_VERSION"),
        "config_version": CONFIG_VERSION,
        "worker_package_version": worker::PACKAGE_VERSION,
        "worker_protocol_version": worker::WORKER_PROTOCOL_VERSION,
        "pid_format_version": worker::PID_FORMAT_VERSION,
        "target_os": std::env::consts::OS,
        "target_arch": std::env::consts::ARCH,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_identifiers_match_registered_inputs() {
        let lock: serde_json::Value =
            serde_json::from_str(include_str!("../../references/v1-baseline.json")).unwrap();
        let guest = &lock["code_baseline"]["rust_guest"];
        let info = service();
        assert_eq!(info["package_version"], guest["service_package_version"]);
        for key in [
            "config_version",
            "worker_package_version",
            "worker_protocol_version",
            "pid_format_version",
        ] {
            assert_eq!(info[key], guest[key], "{key}");
        }
    }
}
