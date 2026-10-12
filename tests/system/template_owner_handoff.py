#!/usr/bin/env python3
"""Actual distinct-UID sealed handoff and interrupted handoff recovery.

Requires an isolated root Linux test host. It launches no VM and touches only
the freshly allocated test directory. This is not a mocked chown acceptance.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import json
import os
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools.template_stage import file_hash, stage_downloaded, stage_inventory


def main():
    if sys.platform != "linux" or os.geteuid() != 0:
        raise SystemExit("isolated root Linux host required")
    center_uid = 251001
    with tempfile.TemporaryDirectory(prefix="sandproof-owner-handoff-") as temporary:
        root = Path(temporary).resolve()
        registry = root / "registry"
        for directory in (registry, registry / "inbox", registry / "artifacts"):
            directory.mkdir(mode=0o700)
            os.chown(directory, center_uid, -1)
        kernel = root / "kernel"
        kernel.write_bytes(b"trusted-test-kernel")
        job = root / ("a" * 32)
        job.mkdir(mode=0o700)
        disk = job / "candidate.ext4"
        disk.write_bytes(b"opaque-candidate-test-bytes")
        disk.chmod(0o600)
        identity = {"id": job.name, "nonce": "handoff-test-nonce"}
        status = {"state": "candidate", "ready": False, "startup_applied": False,
                  "disk_bytes": disk.stat().st_size, "min_free_bytes": 0,
                  "image_digest": "sha256:" + "a" * 64, "rootfs_sha256": file_hash(disk),
                  "runtime_defaults": {"env_vars": {}, "user": "root", "workdir": "/", "argv": []}}
        arguments = (registry, kernel, disk, status, 1, 1, 256, 384)
        chown = os.chown
        calls = 0
        def interrupted(*args, **kwargs):
            nonlocal calls
            calls += 1
            if calls == 3:
                raise OSError("injected after actual first two ownership transfers")
            return chown(*args, **kwargs)
        try:
            with patch("tools.template_stage.os.chown", side_effect=interrupted):
                stage_downloaded(*arguments, candidate_id=job.name, ownership=identity,
                                 registry_owner_uid=center_uid)
        except OSError:
            pass
        else:
            raise AssertionError("handoff fault not reached")
        assert not (registry / "inbox" / job.name).exists()
        assert disk.stat().st_uid == 0
        assert file_hash(disk) == status["rootfs_sha256"]
        assert stage_downloaded(*arguments, candidate_id=job.name, ownership=identity,
                                registry_owner_uid=center_uid) == job.name
        final = registry / "inbox" / job.name
        assert not disk.exists()
        assert final.stat().st_uid == center_uid
        assert file_hash(final / "rootfs.ext4") == status["rootfs_sha256"]
        for name in ("kernel", "rootfs.ext4", "manifest.json", ".build-owner.json"):
            assert (final / name).stat().st_uid == center_uid
            if name != ".build-owner.json":
                assert (final / name).stat().st_mode & 0o222 == 0
        stage_inventory(final, identity, registry_owner_uid=center_uid)
        assert stage_downloaded(*arguments, candidate_id=job.name, ownership=identity,
                                registry_owner_uid=center_uid) == job.name
        manifest = json.loads((final / "manifest.json").read_text())
        assert manifest["binding"]["rootfs"]["path"] == str(final / "rootfs.ext4")
    print(json.dumps({"distinct_uid_handoff": "passed", "interrupted_handoff": "passed"}))


if __name__ == "__main__":
    main()
