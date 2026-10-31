//! Independent wire fixtures for the fixed dependency's visible GET shape.
//! No VM, CPU compatibility or source provenance is synthesized by these tests.
use super::*;
use crate::RuntimeConfig;
use serde_json::{Value, json};

fn config(networked: bool) -> RuntimeConfig {
    RuntimeConfig {
        guest_instance_id: "guest-instance".into(),
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
        private_netns: networked.then(|| "/run/netns/original".into()),
        access_token_hash: networked.then(|| "a".repeat(128)),
    }
}

fn wire(networked: bool) -> Value {
    json!({
        "balloon":null,
        "boot-source":{"kernel_image_path":"/vmlinux","initrd_path":null,
            "boot_args":"console=ttyS0 reboot=k panic=1 pci=off"},
        "cpu-config":null,
        "drives":[{"drive_id":"rootfs","partuuid":null,"is_root_device":true,
            "cache_type":"Unsafe","is_read_only":false,"path_on_host":"/rootfs.ext4",
            "rate_limiter":null,"io_engine":"Sync","socket":null}],
        "entropy":null,"logger":null,
        "machine-config":{"vcpu_count":2,"mem_size_mib":768,"smt":false,
            "track_dirty_pages":false,"huge_pages":"None"},
        "memory-hotplug":null,"metrics":null,
        "mmds-config":if networked {json!({"version":"V2","network_interfaces":["eth0"],
            "ipv4_address":"169.254.169.254","imds_compat":false})} else {Value::Null},
        "network-interfaces":if networked {json!([{"iface_id":"eth0","host_dev_name":"tap0",
            "guest_mac":"06:00:ac:10:00:02","mtu":null,"rx_rate_limiter":null,
            "tx_rate_limiter":null}])} else {json!([])},
        "pmem":[],"vsock":null
    })
}

fn check(config: &RuntimeConfig, value: &Value) -> Result<(), RuntimeError> {
    BootPlan::for_runtime(config).check_full_configuration(&serde_json::to_vec(value).unwrap())
}

#[test]
fn visible_fixed_get_matches_the_actual_boot_plan_with_and_without_network() {
    for networked in [false, true] {
        let config = config(networked);
        assert!(check(&config, &wire(networked)).is_ok());
        assert!(check(&config, &wire(!networked)).is_err());
        let mut changed = config.clone();
        changed.vcpu_count = 4;
        assert!(check(&changed, &wire(networked)).is_err());
        changed = config.clone();
        changed.memory_mib += 1;
        assert!(check(&changed, &wire(networked)).is_err());
    }
}

#[test]
fn changed_machine_boot_drive_network_and_mmds_fields_are_refused() {
    let config = config(true);
    for (pointer, value) in [
        ("/machine-config/vcpu_count", json!(4)),
        ("/machine-config/mem_size_mib", json!(769)),
        ("/machine-config/smt", json!(true)),
        ("/machine-config/track_dirty_pages", json!(true)),
        ("/machine-config/huge_pages", json!("2M")),
        ("/boot-source/kernel_image_path", json!("/other-kernel")),
        ("/boot-source/initrd_path", json!("/initrd")),
        ("/boot-source/boot_args", json!("console=ttyS0")),
        ("/drives/0/drive_id", json!("other")),
        ("/drives/0/partuuid", json!("other")),
        ("/drives/0/is_root_device", json!(false)),
        ("/drives/0/cache_type", json!("Writeback")),
        ("/drives/0/is_read_only", json!(true)),
        ("/drives/0/path_on_host", json!("/other-disk")),
        ("/drives/0/rate_limiter", json!({})),
        ("/drives/0/io_engine", json!("Async")),
        ("/drives/0/socket", json!("/socket")),
        ("/network-interfaces/0/iface_id", json!("eth1")),
        ("/network-interfaces/0/host_dev_name", json!("tap1")),
        (
            "/network-interfaces/0/guest_mac",
            json!("06:00:ac:10:00:03"),
        ),
        ("/network-interfaces/0/mtu", json!(1500)),
        ("/network-interfaces/0/rx_rate_limiter", json!({})),
        ("/network-interfaces/0/tx_rate_limiter", json!({})),
        ("/mmds-config/version", json!("V1")),
        ("/mmds-config/network_interfaces", json!(["eth1"])),
        ("/mmds-config/ipv4_address", json!("169.254.169.253")),
        ("/mmds-config/imds_compat", json!(true)),
    ] {
        let mut changed = wire(true);
        *changed.pointer_mut(pointer).unwrap() = value;
        assert!(check(&config, &changed).is_err(), "accepted {pointer}");
    }
    let mut changed = wire(true);
    changed["machine-config"]["cpu_template"] = json!("T2");
    assert!(check(&config, &changed).is_err());
}

#[test]
fn unsupported_extra_devices_unknown_fields_and_duplicate_keys_are_refused() {
    let config = config(true);
    for field in [
        "balloon",
        "cpu-config",
        "logger",
        "metrics",
        "vsock",
        "entropy",
        "memory-hotplug",
    ] {
        let mut changed = wire(true);
        changed[field] = json!({});
        assert!(check(&config, &changed).is_err(), "accepted {field}");
    }
    for field in ["drives", "network-interfaces"] {
        let mut changed = wire(true);
        let extra = changed[field][0].clone();
        changed[field].as_array_mut().unwrap().push(extra);
        assert!(check(&config, &changed).is_err(), "accepted extra {field}");
        changed[field] = json!([]);
        assert!(
            check(&config, &changed).is_err(),
            "accepted missing {field}"
        );
    }
    for pointer in [
        "",
        "/boot-source",
        "/machine-config",
        "/drives/0",
        "/network-interfaces/0",
        "/mmds-config",
    ] {
        let mut changed = wire(true);
        changed
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), json!(null));
        assert!(
            check(&config, &changed).is_err(),
            "accepted extra field at {pointer}"
        );
    }
    let mut changed = wire(true);
    changed["pmem"] = json!([null]);
    assert!(check(&config, &changed).is_err());
    let original = serde_json::to_string(&wire(true)).unwrap();
    for (old, duplicate) in [
        ("\"vcpu_count\":2", "\"vcpu_count\":2,\"vcpu_count\":4"),
        ("\"balloon\":null", "\"balloon\":null,\"balloon\":null"),
    ] {
        let changed = original.replacen(old, duplicate, 1);
        assert_ne!(changed, original);
        assert!(
            BootPlan::for_runtime(&config)
                .check_full_configuration(changed.as_bytes())
                .is_err()
        );
    }
}

#[test]
fn malformed_truncated_and_missing_required_reply_fields_are_refused() {
    let plan = BootPlan::for_runtime(&config(true));
    for bytes in [b"".as_slice(), b"{", b"null", b"[]"] {
        assert!(plan.check_full_configuration(bytes).is_err());
    }
    assert!(
        plan.check_full_configuration(&vec![b' '; 1_048_577])
            .is_err()
    );
    for field in [
        "boot-source",
        "machine-config",
        "drives",
        "network-interfaces",
        "balloon",
        "memory-hotplug",
    ] {
        let mut changed = wire(true);
        changed.as_object_mut().unwrap().remove(field);
        assert!(
            check(&config(true), &changed).is_err(),
            "accepted missing {field}"
        );
    }
}
