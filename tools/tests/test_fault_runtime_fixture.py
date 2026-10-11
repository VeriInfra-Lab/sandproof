"""Fault-probe assembly checks only; no KVM, service or privileged helper runs."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests/system"))

import server_process_crash as faults
import _runtime_services as runtime


class FaultRuntimeFixtureTests(unittest.TestCase):
    def test_private_host_error_or_disconnect_requires_exact_exit_and_never_success(self):
        for status in (500, 502, 503, 504):
            child = Mock()
            child.wait.return_value = 86
            faults.interrupted_host_request(lambda: (status, b""), child)
            child.wait.assert_called_once_with(timeout=30)
        for status in (200, 201, 204, 404, 429):
            with self.subTest(status=status), self.assertRaises(AssertionError):
                faults.interrupted_host_request(lambda: (status, b""), Mock())
        child = Mock()
        child.wait.return_value = 0
        with self.assertRaises(AssertionError):
            faults.interrupted_host_request(lambda: (503, b""), child)

    def test_explicit_new_native_catalog_uses_original_artifact_hash_and_boundaries(self):
        with tempfile.TemporaryDirectory(prefix="sf-", dir="/tmp") as temporary:
            root = Path(temporary)
            kernel, image, firecracker = (root / name for name in ("kernel", "image", "fc"))
            kernel.write_bytes(b"controlled-invalid-kernel")
            image.write_bytes(b"controlled-image")
            firecracker.write_bytes(b"controlled-runtime")
            values = dict(namespace="1234567890abcdef", database=str(root / "db/controller.db"),
                node_state_dir=str(root / "state"), chroot_base=str(root / "jails"),
                jailer="/approved/jailer", firecracker=str(firecracker), kernel=str(kernel),
                rootfs=str(image), uid_base=3200000, vcpu_count=1, memory_mib=256,
                file_policy_digest=1, max_active_global=2, max_active_per_tenant=1,
                max_disk_bytes_global=4096, max_disk_bytes_per_tenant=2048,
                cleanup_reserve_bytes=1024, max_timeout_secs=600, default_timeout_secs=300,
                expiry_scan_seconds=3600)
            args = SimpleNamespace(service_cgroup_record=Path("/approved/service-record"))
            with patch.object(runtime.RuntimeServices, "prepare", return_value="one-fixture") as prepared:
                result = faults.prepare_fault_services(args, root, values, slots=2,
                    key="synthetic-key", port=12345)
            self.assertEqual(result, "one-fixture")
            data = prepared.call_args.kwargs
            native = data["native_pools"][0]
            catalog = data["registered_catalogs"][0]
            self.assertEqual(native["templates"], catalog)
            # `base` is the real template ID. A same-name alias is rejected by
            # the original Rust catalog, not silently treated as a no-op.
            self.assertEqual(catalog["aliases"], {})
            self.assertEqual(catalog["templates"][0]["template_id"], "base")
            self.assertEqual(native["expiry_scan_seconds"], 3600)
            self.assertEqual(native["max_disk_bytes_per_tenant"], 2048)
            self.assertEqual(catalog["compute_limits"]["max_vcpus_global"], 2)
            host_memory=catalog["templates"][0]["resources"]["host_memory_bytes"]
            self.assertEqual(catalog["compute_limits"]["max_memory_bytes_global"],2*host_memory)
            self.assertEqual(catalog["compute_limits"]["max_memory_bytes_per_tenant"],host_memory)
            self.assertEqual(catalog["templates"][0]["resources"]["memory_mib"],256)
            self.assertEqual(catalog["templates"][0]["kernel"]["sha256"],
                             hashlib.sha256(kernel.read_bytes()).hexdigest())
            self.assertEqual(data["job_count"], 4)
            self.assertEqual(native["maintenance"]["max_tail_inputs"], 1000)
            with patch.object(runtime.RuntimeServices, "prepare", return_value="one-fixture") as selected:
                faults.prepare_fault_services(args, root, values, slots=2,
                    key="synthetic-key", port=12345, checkpoint_tail_inputs=128)
            self.assertEqual(selected.call_args.kwargs["native_pools"][0]["maintenance"]["max_tail_inputs"], 128)
            self.assertEqual(selected.call_args.kwargs["native_pools"][0]["maintenance"]["capacity"],
                             native["maintenance"]["capacity"])
            for retired in runtime.RETIRED_NATIVE_FIELDS:
                with self.subTest(retired=retired), self.assertRaises(ValueError), \
                        patch.object(runtime.RuntimeServices, "prepare") as forbidden:
                    faults.prepare_fault_services(args, root, {**values, retired: None},
                        slots=2, key="synthetic-key", port=12345)
                forbidden.assert_not_called()

    def test_controlled_cache_observation_requires_the_exact_registered_projection(self):
        with tempfile.TemporaryDirectory(prefix='sf-cache-') as temporary:
            root=Path(temporary).resolve()
            (root/'artifacts').mkdir(mode=0o700)
            directory=root/'artifacts/controlled'
            directory.mkdir(mode=0o700)
            original={ 'template_id':'base','owner':2,'revision':'r1' }
            for name,leaf in [('kernel','kernel'),('rootfs','rootfs.ext4')]:
                (directory/leaf).write_bytes(('controlled-'+name).encode())
                (directory/leaf).chmod(0o600)
                original[name]=dict(path='/accepted/'+leaf,size_bytes=(directory/leaf).stat().st_size,
                    sha256=hashlib.sha256((directory/leaf).read_bytes()).hexdigest())
            local=json.loads(json.dumps(original))
            for name,leaf in [('kernel','kernel'),('rootfs','rootfs.ext4')]:
                local[name]['path']=str(directory/leaf)
            node={'node_id':'controlled-node'}
            info=directory.stat()
            manifest=dict(version=1,request=dict(node=node,descriptor=original),local=local,
                          device=info.st_dev,inode=info.st_ino)
            path=directory/'manifest.json'
            path.write_text(json.dumps(manifest))
            path.chmod(0o600)
            services=SimpleNamespace(host_configs=[dict(template_cache=dict(root=str(root),max_entries=1),
                inbox=dict(node=node))],center_config=dict(templates=dict(templates=[original])))
            self.assertEqual(faults.registered_cache_template(services,'base'),local)
            kernel,identity=faults.registered_cache_kernel(services)
            self.assertEqual(kernel,directory/'kernel')
            self.assertEqual(identity,(kernel.stat().st_dev,kernel.stat().st_ino))
            for changed in [local|{'owner':1},local|{'revision':'r2'}]:
                path.write_text(json.dumps(manifest|{'local':changed}))
                with self.assertRaises(AssertionError):
                    faults.registered_cache_template(services,'base')

    def test_unknown_create_reads_original_center_intent_and_restores_only_that_role(self):
        processes = Mock()
        processes.children = {"center": Mock(poll=Mock(return_value=None))}
        services = SimpleNamespace(processes=processes,
            center_config={"deployment_id": "synthetic-deployment"})
        original = {"sandbox_id": "synthetic-global-id"}
        marker = {"fault_intent": "synthetic-intent"}
        with patch("_center_host_services.find_runtime_create", return_value=original) as lookup:
            self.assertEqual(faults.original_create_identity(services, marker), original)
        lookup.assert_called_once_with(processes.audit.return_value, marker,
                                       deployment="synthetic-deployment")
        processes.stop.assert_called_once_with("center")
        processes.start.assert_called_once_with("center")
        processes.reset_mock()
        processes.audit.side_effect = ValueError("original audit unknown")
        with self.assertRaises(ValueError):
            faults.original_create_identity(services, marker)
        processes.start.assert_called_once_with("center")


if __name__ == "__main__":
    unittest.main()
