import copy
import json
import os
from pathlib import Path
import tempfile
import unittest

from tools.template_stage import file_hash, stage


class StagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.base = Path(self.temporary.name).resolve()
        self.root = self.base / "registry"
        for path in (self.root, self.root / "inbox", self.root / "artifacts"):
            path.mkdir(mode=0o700)
        self.kernel = self.base / "kernel"
        self.disk = self.base / "disk"
        self.kernel.write_bytes(b"kernel")
        self.disk.write_bytes(b"candidate disk")
        self.status = {"state": "candidate", "ready": False, "startup_applied": False,
                       "disk_bytes": self.disk.stat().st_size, "min_free_bytes": 1,
                       "image_digest": "sha256:" + "a" * 64, "rootfs_sha256": file_hash(self.disk),
                       "runtime_defaults": {"env_vars": {}, "user": "root", "workdir": "/", "argv": []}}

    def tearDown(self):
        self.temporary.cleanup()

    def test_atomic_private_staging_does_not_publish_or_modify_source(self):
        candidate = stage(self.root, self.kernel, self.disk, self.status, 1, 1, 256, 384)
        final = self.root / "inbox" / candidate
        self.assertEqual(len(candidate), 32)
        manifest = json.loads((final / "manifest.json").read_text())
        self.assertEqual(manifest["binding"]["rootfs"]["path"], str(final / "rootfs.ext4"))
        self.assertEqual(manifest["binding"]["owner"], 1)
        self.assertEqual(file_hash(final / "rootfs.ext4"), file_hash(self.disk))
        self.assertEqual(list((self.root / "artifacts").iterdir()), [])
        for name in ("kernel", "rootfs.ext4", "manifest.json"):
            self.assertEqual((final / name).stat().st_mode & 0o222, 0)
        self.assertFalse(any(p.name.startswith(".stage-") for p in (self.root / "inbox").iterdir()))

    def test_candidate_failure_mismatch_and_resource_boundaries(self):
        for field, value in (("ready", True), ("state", "failed"), ("startup_applied", True),
                             ("rootfs_sha256", "0" * 64), ("disk_bytes", 100), ("image_digest", "latest"),
                             ("min_free_bytes", -1), ("runtime_defaults", None)):
            status = copy.deepcopy(self.status)
            status[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                stage(self.root, self.kernel, self.disk, status, 1, 1, 256, 384)
        for owner, cpu, memory, host in ((0, 1, 256, 384), (1, 256, 256, 384),
                                       (1, 1, 127, 384), (1, 1, 256, 256), (True, 1, 256, 384), (2**64, 1, 256, 384)):
            with self.assertRaises(ValueError):
                stage(self.root, self.kernel, self.disk, self.status, owner, cpu, memory, host)
        self.assertEqual(list((self.root / "inbox").iterdir()), [])

    def test_nonprivate_or_symlink_root_and_input_are_rejected(self):
        self.root.chmod(0o755)
        with self.assertRaises(ValueError):
            stage(self.root, self.kernel, self.disk, self.status, 1, 1, 256, 384)
        self.root.chmod(0o700)
        link = self.base / "link"
        link.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(ValueError):
            stage(link, self.kernel, self.disk, self.status, 1, 1, 256, 384)
        source = self.base / "source-link"
        source.symlink_to(self.disk)
        with self.assertRaises(ValueError):
            stage(self.root, self.kernel, source, self.status, 1, 1, 256, 384)


if __name__ == "__main__":
    unittest.main()
