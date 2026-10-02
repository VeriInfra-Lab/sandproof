"""Compare fixed BuildKit exporters inside the synthetic offline builder VM."""

import json
from pathlib import Path
import stat
import shutil
import sys
import threading
import time

sys.path.insert(0, "/input")
from template_inputs import bounded_json, inspect_oci
from template_worker import Commands, buildkit_layout, terminate


def exercise():
    work = Path("/output/job")
    limits = {"max_bytes": 512 * 1024 * 1024, "max_entries": 10000}
    linked = work / "oci"
    original = inspect_oci(linked, **limits)
    assert not (work / "cache").exists(), "worker must release its finished build cache"

    # Compare both exporters from one fresh synthetic solve. The production
    # worker has already discarded its cache; a rebuild can have new timestamps,
    # so do not compare its image digest with the earlier candidate's digest.
    baseline = Path("/output/baseline")
    baseline.mkdir(mode=0o700)
    cache = baseline / "cache"
    address = "unix:///output/baseline/buildkit.sock"
    errors = []
    with (baseline / "build.log").open("xb") as log:
        commands = Commands(time.monotonic() + 120, log, 2 * 1024 * 1024)
        daemon = commands.run([
            "/input/tools/buildkitd", "--root", str(cache), "--addr", address,
            "--oci-worker=true", "--oci-worker-binary", "/input/tools/buildkit-runc",
            "--oci-worker-snapshotter", "auto", "--oci-worker-net", "host",
            "--containerd-worker=false"], daemon=True)
        def drain():
            try:
                with (baseline / "daemon.log").open("xb") as output:
                    Commands(commands.deadline, output, 0, budget=commands.budget).drain(daemon)
            except Exception as error:
                errors.append(error)
                terminate(daemon)
            finally:
                daemon.stdout.close()
        reader = threading.Thread(target=drain, daemon=True)
        reader.start()
        try:
            commands.run([
                "/input/tools/buildctl", "--addr", address, "--wait", "build", "--progress", "plain",
                "--frontend", "dockerfile.v0", "--local", "context=" + str(work / "context"),
                "--local", "dockerfile=" + str(work / "context"), "--opt", "platform=linux/amd64",
                "--metadata-file", str(baseline / "metadata.json"),
                "--output", "type=image,oci-mediatypes=true,compression=gzip,push=false,store=false",
                "--output", "type=oci,tar=false,compression=gzip,dest=" + str(baseline / "oci")])
        finally:
            terminate(daemon)
            reader.join(timeout=4)
        assert not reader.is_alive() and not errors
    linked = baseline / "linked"
    buildkit_layout(cache, baseline / "metadata.json", linked, max_bytes=limits["max_bytes"])
    selected = bounded_json(linked / "index.json")["manifests"][0]
    manifest = bounded_json(linked / "blobs/sha256" / selected["digest"][7:])
    stores = list(cache.glob("runc-*/content/blobs/sha256"))
    assert len(stores) == 1
    blobs = []
    for entry in [selected, manifest["config"], *manifest["layers"]]:
        source = (stores[0] / entry["digest"][7:]).lstat()
        target = (linked / "blobs/sha256" / entry["digest"][7:]).lstat()
        assert stat.S_ISREG(source.st_mode) and stat.S_ISREG(target.st_mode)
        assert (source.st_dev, source.st_ino) == (target.st_dev, target.st_ino)
        assert source.st_size == target.st_size == entry["size"] and source.st_nlink >= 2
        blobs.append({"digest": entry["digest"], "size": entry["size"], "inode": source.st_ino})
    actual = inspect_oci(linked, **limits)
    expected = inspect_oci(baseline / "oci", **limits)
    # Full image and config digests, layer digests, diff_ids and startup config
    # must match; no timestamp or metadata normalization hides differences.
    assert actual == expected, (actual, expected)
    expected_descriptor = bounded_json(baseline / "oci/index.json")["manifests"][0]
    expected_manifest = bounded_json(baseline / "oci/blobs/sha256" / expected_descriptor["digest"][7:])
    assert manifest == expected_manifest
    for entry in [selected, manifest["config"], *manifest["layers"]]:
        old = (baseline / "oci/blobs/sha256" / entry["digest"][7:]).stat()
        new = (linked / "blobs/sha256" / entry["digest"][7:]).stat()
        assert (old.st_dev, old.st_ino) != (new.st_dev, new.st_ino)
    # Remove all private cache state, including snapshots and metadata. Only
    # the independently linked, validated OCI bytes are needed for unpacking.
    shutil.rmtree(cache)
    assert not cache.exists() and inspect_oci(linked, **limits) == actual
    for entry in blobs:
        assert (linked / "blobs/sha256" / entry["digest"][7:]).stat().st_nlink == 1
    assert inspect_oci(work / "oci", **limits) == original
    print(json.dumps({"same_image_and_config": True, "same_manifest_and_layers": True,
                      "content_survives_daemon_stop_and_cache_removal": True,
                      "worker_cache_absent": True, "original_candidate_unchanged": True,
                      "image_digest": actual["image_digest"], "linked_blobs": blobs}))


if __name__ == "__main__":
    exercise()
