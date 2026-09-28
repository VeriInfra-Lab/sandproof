"""Check startup storage placement on a private 64 MiB ext4 loop filesystem.

Run as root with reviewed binaries and a root-only template configuration.
This starts the real service but creates no VM and makes no KVM claim. The
optional previous binary must reproduce the omitted database-device check.
Only this run's loop device and mount are removed; evidence is retained.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request


REJECTION = (
    "database, node state and private VM disk must share a filesystem for cleanup reserve"
)
REPO = Path(__file__).resolve().parents[2]


def git(*args):
    return command("git", "-c", f"safe.directory={REPO}", "-C", str(REPO), *args)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def trusted(path, private=False):
    assert path.is_absolute() and path.resolve(strict=True) == path
    for part in (path, *path.parents):
        metadata = part.stat()
        assert metadata.st_uid == 0 and metadata.st_mode & 0o022 == 0
    if private:
        assert path.stat().st_mode & 0o077 == 0


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def run_case(root, mounted, template, binary, label, placement, accepted, index):
    # Keep jail paths short enough for Firecracker's Unix socket address.
    local = root / str(index)
    remote = mounted / str(index)
    local.mkdir(mode=0o700)
    remote.mkdir(mode=0o700)
    paths = {}
    for name, on_loop in zip(("db", "state", "jails"), placement, strict=True):
        paths[name] = (remote if on_loop else local) / name
        paths[name].mkdir(mode=0o700)
    config = dict(template)
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    key = secrets.token_hex(32)
    config.update(
        namespace=secrets.token_hex(8),
        database=str(paths["db"] / "controller.db"),
        node_state_dir=str(paths["state"]), chroot_base=str(paths["jails"]),
        bind=f"127.0.0.1:{port}", credentials=[dict(tenant=1, secret=key)],
    )
    config_path = local / "config.json"
    config_path.write_text(json.dumps(config))
    config_path.chmod(0o600)
    log_path = local / "server.log"
    process = None
    try:
        with log_path.open("xb") as log:
            process = subprocess.Popen(
                [str(binary), str(config_path)], stdin=subprocess.DEVNULL,
                stdout=log, stderr=subprocess.STDOUT,
                env={"PATH": "/usr/sbin:/usr/bin:/bin", "LANG": "C.UTF-8"},
            )
        if accepted:
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            request = urllib.request.Request(
                f"http://127.0.0.1:{port}/v2/sandboxes", headers={"X-API-Key": key}
            )
            deadline = time.monotonic() + 20
            while True:
                assert process.poll() is None, f"{label}: startup exited"
                try:
                    with opener.open(request, timeout=2) as response:
                        assert response.status == 200 and json.load(response) == []
                    break
                except (OSError, urllib.error.URLError):
                    assert time.monotonic() < deadline, f"{label}: startup timed out"
                    time.sleep(0.05)
            assert Path(config["database"]).is_file()
        else:
            assert process.wait(timeout=20) != 0, f"{label}: incorrect success"
            assert REJECTION in log_path.read_text(), f"{label}: wrong rejection"
            assert all(not list(path.iterdir()) for path in paths.values()), (
                f"{label}: rejection created persistent state"
            )
        # The only request above is read-only listing. Check host claims while
        # the service is still running, before teardown could mask a leak.
        claims = paths["jails"] / ".claims"
        assert not claims.exists() or not list(claims.iterdir())
        assert not list(Path("/run/netns").glob(f"vm-{config['namespace']}-*"))
        parent = template.get("host_resources", {}).get("cgroup_parent")
        if parent:
            assert not list(Path(parent).glob(f"vm-{config['namespace']}-*"))
        result = dict(
            case=label, accepted=accepted, vm_claims=0,
            devices={name: path.stat().st_dev for name, path in paths.items()},
            no_state_created_on_rejection=not accepted,
            audited_before_service_exit=accepted,
        )
        print(json.dumps(result), flush=True)
        return result
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            process.wait(timeout=10)


def child(args):
    assert os.stat("/proc/self/ns/mnt").st_ino != args.parent_mount_inode
    assert not git("status", "--porcelain"), "test source must be committed and clean"
    revision = git("rev-parse", "HEAD")
    trusted(args.root.parent, private=True)
    args.root.mkdir(mode=0o700)
    mounted = args.root / "loop"
    mounted.mkdir(mode=0o700)
    image = args.root / "scratch.ext4"
    with image.open("xb") as stream:
        stream.truncate(64 * 1024**2)
    template = json.loads(args.config.read_text())
    base_digest = digest(Path(template["rootfs"]))
    loop = None
    mounted_ok = False
    report = None
    try:
        loop = Path(command("losetup", "--find", "--show", "--nooverlap", str(image)))
        device = loop.stat().st_rdev
        sysfs = Path(f"/sys/dev/block/{os.major(device)}:{os.minor(device)}/loop/backing_file")
        assert Path(sysfs.read_text().strip()).samefile(image)
        command("mkfs.ext4", "-q", "-F", str(loop))
        command("mount", "-t", "ext4", "-o", "nodev,nosuid,noexec", str(loop), str(mounted))
        mounted_ok = True
        mounted.chmod(0o700)
        assert mounted.stat().st_dev == device != args.root.stat().st_dev
        cases = []
        if args.previous_server:
            cases.append(run_case(args.root, mounted, template, args.previous_server,
                                  "old-database-other", (False, True, True), True, len(cases)))
        for label, placement, accepted in (
            ("same-outer", (False, False, False), True),
            ("same-loop", (True, True, True), True),
            ("database-other", (False, True, True), False),
            ("node-other", (True, False, True), False),
            ("jail-other", (True, True, False), False),
        ):
            cases.append(run_case(args.root, mounted, template, args.server,
                                  label, placement, accepted, len(cases)))
        assert digest(Path(template["rootfs"])) == base_digest
        report = dict(
            revision=revision, kernel=os.uname().release,
            cases=cases, server_sha256=args.server_sha256,
            previous_server_sha256=args.previous_sha256,
            script_sha256=digest(Path(__file__)), base_sha256=base_digest,
            scratch_bytes=image.stat().st_size, loop_device=device,
            scope="real startup and filesystem placement only; no VM or exhaustion test",
        )
    finally:
        if mounted_ok:
            command("umount", str(mounted))
            assert mounted.stat().st_dev == args.root.stat().st_dev
        if loop is not None:
            # Do not detach a reused or unrelated loop device on failure.
            assert Path(sysfs.read_text().strip()).samefile(image)
            command("losetup", "--detach", str(loop))
            assert not sysfs.exists(), "loop backing file still attached"
    assert report is not None
    assert git("rev-parse", "HEAD") == revision and not git("status", "--porcelain")
    report.update(unmounted=True, loop_detached=True)
    (args.root / "result.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--server-sha256", required=True)
    parser.add_argument("--previous-server", type=Path)
    parser.add_argument("--previous-sha256")
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--parent-mount-inode", type=int, help=argparse.SUPPRESS)
    args = parser.parse_args()
    assert os.geteuid() == 0 and sys.platform == "linux"
    os.umask(0o077)
    for binary, expected in ((args.server, args.server_sha256),
                             (args.previous_server, args.previous_sha256)):
        if binary:
            trusted(binary, private=True)
            assert digest(binary) == expected, "reviewed binary hash mismatch"
    trusted(args.config, private=True)
    assert args.root.is_absolute() and not args.root.exists()
    if args.parent_mount_inode is None:
        inode = os.stat("/proc/self/ns/mnt").st_ino
        subprocess.run([
            "unshare", "--mount", "--propagation", "private", sys.executable,
            str(Path(__file__).resolve()), *sys.argv[1:], "--parent-mount-inode", str(inode),
        ], check=True)
    else:
        child(args)


if __name__ == "__main__":
    main()
