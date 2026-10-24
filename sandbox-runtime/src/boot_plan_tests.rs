use super::*;
use serde_json::json;

fn config() -> RuntimeConfig {
    RuntimeConfig {
        console_policy: None,
        host_slot: None,
        jailer: "/fixed/jailer".into(),
        firecracker: "/fixed/firecracker".into(),
        chroot_base: "/fixed/v".into(),
        kernel: "/original/kernel".into(),
        rootfs: "/original/rootfs".into(),
        vm_uid: 64001,
        vm_gid: 64001,
        vcpu_count: 2,
        memory_mib: 768,
        host_resources: None,
        private_netns: None,
        access_token_hash: None,
    }
}

#[test]
fn actual_boot_requests_preserve_original_wire_values_and_order() {
    for networked in [false, true] {
        let mut config = config();
        if networked {
            config.access_token_hash = Some("a".repeat(128));
            config.private_netns = Some("/run/netns/original".into());
        }
        let plan = BootPlan::for_runtime(&config);
        let mut actual = Vec::new();
        plan.configure(|path, body| {
            actual.push((path.to_owned(), body.clone()));
            Ok(())
        })
        .unwrap();
        let mut original = vec![
            (
                "/machine-config".to_owned(),
                json!({"vcpu_count": 2, "mem_size_mib": 768}),
            ),
            (
                "/boot-source".to_owned(),
                json!({"kernel_image_path":"/vmlinux", "boot_args":"console=ttyS0 reboot=k panic=1 pci=off"}),
            ),
            (
                "/drives/rootfs".to_owned(),
                json!({"drive_id":"rootfs", "path_on_host":"/rootfs.ext4", "is_root_device":true, "is_read_only":false}),
            ),
        ];
        if networked {
            original.push((
                "/network-interfaces/eth0".to_owned(),
                json!({"iface_id":"eth0", "host_dev_name":"tap0", "guest_mac":"06:00:ac:10:00:02"}),
            ));
            original.push((
                "/mmds/config".to_owned(),
                json!({"version":"V2", "network_interfaces":["eth0"]}),
            ));
        }
        assert_eq!(actual, original);
    }
}

#[test]
fn failed_configuration_never_sends_the_later_requests() {
    let mut config = config();
    config.access_token_hash = Some("a".repeat(128));
    config.private_netns = Some("/run/netns/original".into());
    let plan = BootPlan::for_runtime(&config);
    let mut seen = Vec::new();
    assert!(
        plan.configure(|path, _| {
            seen.push(path.to_owned());
            if path == "/drives/rootfs" {
                Err(RuntimeError::Protocol("injected PUT failure"))
            } else {
                Ok(())
            }
        })
        .is_err()
    );
    assert_eq!(seen, ["/machine-config", "/boot-source", "/drives/rootfs"]);
}

#[test]
fn mutable_credential_rotation_does_not_change_the_device_boot_plan() {
    let mut original = config();
    original.access_token_hash = Some("a".repeat(128));
    original.private_netns = Some("/run/netns/original".into());
    let mut rotated = original.clone();
    rotated.access_token_hash = Some("b".repeat(128));
    let before = BootPlan::for_runtime(&original);
    let after = BootPlan::for_runtime(&rotated);
    assert_eq!(before, after);
    assert_eq!(before.digest().unwrap(), after.digest().unwrap());
    rotated.memory_mib += 1;
    assert_ne!(
        before.digest().unwrap(),
        BootPlan::for_runtime(&rotated).digest().unwrap()
    );
    let bytes = serde_json::to_string(&before).unwrap();
    assert!(!bytes.contains(&"a".repeat(128)) && !bytes.contains(&"b".repeat(128)));
    assert!(!bytes.contains("InstanceStart"));
}
