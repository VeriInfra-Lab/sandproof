//! Template defaults and bounded envd launch framing, separate from host I/O.
use sandproof_controller_service::templates::{TemplateBinding, TemplateStartup};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Read};

pub(crate) fn init_payload(
    token: &str,
    overrides: &BTreeMap<String, String>,
    startup: Option<&TemplateStartup>,
) -> serde_json::Value {
    let mut env = startup.map_or_else(BTreeMap::new, |value| value.env_vars.clone());
    env.extend(overrides.clone());
    let mut result = serde_json::json!({
        "accessToken": token,
        "defaultUser": startup.map_or("root", |value| value.user.as_str()),
        "envVars": env,
    });
    if let Some(startup) = startup {
        result["defaultWorkdir"] = serde_json::json!(startup.workdir);
    }
    result
}

pub(crate) fn start_request(startup: &TemplateStartup) -> Result<Vec<u8>, serde_json::Error> {
    let payload = serde_json::to_vec(&serde_json::json!({"process": {
        "cmd": startup.argv[0], "args": &startup.argv[1..],
    }}))?;
    let mut frame = vec![0];
    // Descriptor validation bounds this to < 64 KiB.
    frame.extend(
        u32::try_from(payload.len())
            .expect("validated startup size")
            .to_be_bytes(),
    );
    frame.extend(payload);
    Ok(frame)
}

/// Only an actual Start event confirms dispatch. EOF, errors, a trailer,
/// compression, output or an oversized response never count as success.
/// A malformed header is also complete for transport purposes: stop reading
/// promptly and let `read_started` reject it, without treating it as success.
pub(crate) fn first_frame_complete(bytes: &[u8]) -> bool {
    if bytes.len() < 5 {
        return false;
    }
    let size = u32::from_be_bytes(bytes[1..5].try_into().expect("frame header")) as usize;
    bytes[0] != 0 || !(1..=8192).contains(&size) || bytes.len() >= 5 + size
}

/// Validate a fully observed, size-bounded first frame.
pub(crate) fn read_started(mut source: impl Read) -> Result<u32, Box<dyn std::error::Error>> {
    let mut header = [0; 5];
    source.read_exact(&mut header)?;
    let size = u32::from_be_bytes(header[1..].try_into()?) as usize;
    if header[0] != 0 || !(1..=8192).contains(&size) {
        return Err("invalid envd startup frame".into());
    }
    let mut payload = vec![0; size];
    source.read_exact(&mut payload)?;
    let response: serde_json::Value = serde_json::from_slice(&payload)?;
    let pid = response
        .pointer("/event/start/pid")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or("envd did not confirm process start")?;
    Ok(pid)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartupRecord {
    pub sandbox: u64,
    pub vm: u64,
    pub vmm_pid: u32,
    pub start_ticks: u64,
    pub template: TemplateBinding,
    pub env_vars: BTreeMap<String, String>,
    pub guest_pid: Option<u32>,
}

impl StartupRecord {
    pub fn confirmed_for(&self, expected: &Self) -> Result<(), Box<dyn std::error::Error>> {
        let mut identity = self.clone();
        identity.guest_pid = None;
        if identity != *expected || self.guest_pid.is_none_or(|pid| pid == 0) {
            return Err("template startup is mismatched or has an unknown dispatch outcome".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandproof_controller_service::templates::{ResourceShape, TemplateArtifact};
    #[test]
    fn prefix_detection_waits_for_payload_and_rejects_bad_headers_promptly() {
        for size in 0..5 {
            assert!(!first_frame_complete(&[0; 5][..size]));
        }
        assert!(!first_frame_complete(&[0, 0, 0, 0, 1]));
        assert!(first_frame_complete(&[0, 0, 0, 0, 1, b'x']));
        for header in [[0, 0, 0, 0, 0], [0, 0, 0, 32, 1], [2, 0, 0, 0, 1]] {
            assert!(first_frame_complete(&header));
            assert!(read_started(&header[..]).is_err());
        }
    }
    fn defaults() -> TemplateStartup {
        TemplateStartup {
            env_vars: BTreeMap::from([("VALUE".into(), "image".into())]),
            user: "app".into(),
            workdir: "/workspace".into(),
            argv: vec![
                "/entrypoint".into(),
                "literal space".into(),
                "$VALUE;echo bad".into(),
            ],
        }
    }
    fn record() -> StartupRecord {
        let artifact = TemplateArtifact {
            path: "/protected/image".into(),
            sha256: "a".repeat(64),
            size_bytes: 4096,
        };
        StartupRecord {
            sandbox: 1,
            vm: 1,
            vmm_pid: 123,
            start_ticks: 456,
            template: TemplateBinding {
                guest: None,
                template_id: "one".into(),
                revision: "r1".into(),
                owner: Some(1),
                kernel: artifact.clone(),
                rootfs: artifact,
                resources: ResourceShape {
                    cpu_count: 1,
                    memory_mib: 128,
                    disk_bytes: 4096,
                    host_memory_bytes: 256 * 1024 * 1024,
                },
                envd_version: "0.9.0".into(),
                startup: Some(defaults()),
            },
            env_vars: BTreeMap::new(),
            guest_pid: None,
        }
    }
    #[test]
    fn template_defaults_and_create_overrides_leave_legacy_cwd_unchanged() {
        let env = BTreeMap::from([("VALUE".into(), "create".into())]);
        let legacy = init_payload("token", &env, None);
        assert!(legacy.get("defaultWorkdir").is_none());
        let selected = init_payload("token", &env, Some(&defaults()));
        assert_eq!(selected["defaultUser"], "app");
        assert_eq!(selected["defaultWorkdir"], "/workspace");
        assert_eq!(selected["envVars"]["VALUE"], "create");
        let request = start_request(&defaults()).unwrap();
        assert_eq!(request[0], 0);
        assert_eq!(
            request.len() - 5,
            u32::from_be_bytes(request[1..5].try_into().unwrap()) as usize
        );
        let data: serde_json::Value = serde_json::from_slice(&request[5..]).unwrap();
        assert_eq!(
            data["process"]["args"],
            serde_json::json!(["literal space", "$VALUE;echo bad"])
        );
    }
    #[test]
    fn startup_parser_requires_complete_bounded_start_confirmation() {
        fn frame(value: &serde_json::Value) -> Vec<u8> {
            let body = serde_json::to_vec(value).unwrap();
            let mut result = vec![0];
            result.extend(u32::try_from(body.len()).unwrap().to_be_bytes());
            result.extend(body);
            result
        }
        let good = frame(&serde_json::json!({"event":{"start":{"pid":42}}}));
        assert_eq!(read_started(&good[..]).unwrap(), 42);
        for length in 0..good.len() {
            assert!(read_started(&good[..length]).is_err());
        }
        for bad in [
            serde_json::json!({"event":{"end":{"exitCode":0}}}),
            serde_json::json!({"event":{"start":{"pid":0}}}),
            serde_json::json!({"error":{"code":"permission_denied"}}),
        ] {
            assert!(read_started(&frame(&bad)[..]).is_err());
        }
        for header in [[0, 0, 0, 32, 1], [2, 0, 0, 0, 1], [1, 0, 0, 0, 1]] {
            assert!(read_started(&header[..]).is_err());
        }
    }
    #[test]
    fn unknown_or_changed_startup_can_never_be_reused_as_success() {
        let expected = record();
        assert!(expected.confirmed_for(&expected).is_err());
        let mut done = expected.clone();
        done.guest_pid = Some(55);
        assert!(done.confirmed_for(&expected).is_ok());
        for change in 0..4 {
            let mut other = expected.clone();
            match change {
                0 => other.start_ticks += 1,
                1 => other.vm += 1,
                2 => other
                    .env_vars
                    .insert("VALUE".into(), "changed".into())
                    .map_or((), |_| ()),
                _ => other.template.revision = "other".into(),
            }
            assert!(done.confirmed_for(&other).is_err());
        }
    }
}
