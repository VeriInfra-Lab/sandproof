#!/usr/bin/env python3
"""Offline fixed-base construction from pinned official SquashFS and envd.

No image mount, guest execution, network access, Dockerfile or tenant input.
Only the checked-in profile is accepted. Existing images are never overwritten.
"""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile

if __package__:
    from .build_ci_guest import SERVICE, debugfs, digest, safe_path
else:
    from build_ci_guest import SERVICE, debugfs, digest, safe_path


PROFILE = Path(__file__).resolve().parents[1] / "tests/system/base-inputs.json"
ENVD_GUEST = "/usr/local/bin/envd"
UNIT_GUEST = "/etc/systemd/system/sandproof-envd.service"
LINK_GUEST = "/etc/systemd/system/multi-user.target.wants/sandproof-envd.service"


def verified_copy(source, destination, expected):
    """Validate the private copy before any parser/executable sees its bytes."""
    metadata = source.lstat()
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_size != expected["bytes"]:
        raise ValueError(f"unexpected input type or size: {source.name}")
    descriptor = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as src, destination.open("xb") as dst:
        metadata = os.fstat(src.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size != expected["bytes"]:
            raise ValueError(f"input changed before copy: {source.name}")
        remaining = expected["bytes"]
        while remaining:
            block = src.read(min(remaining, 1024 * 1024))
            if not block:
                raise ValueError(f"truncated input: {source.name}")
            dst.write(block)
            remaining -= len(block)
        if src.read(1):
            raise ValueError(f"input grew during copy: {source.name}")
    if destination.stat().st_size != expected["bytes"] or digest(destination) != expected["sha256"]:
        raise ValueError(f"input fingerprint mismatch: {source.name}")
    destination.chmod(0o400)


def trusted_directory(path):
    for part in (path, *path.parents):
        metadata = part.lstat()
        if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != 0 or metadata.st_mode & 0o022:
            raise ValueError(f"output ancestors must be real, root-owned, non-writable directories: {part}")


def guest_destination(root, name, *, replace_link=False):
    path = root / name.lstrip("/")
    # A trusted input still must not turn an intended guest write into a host write.
    if not path.parent.resolve().is_relative_to(root.resolve()):
        raise ValueError(f"guest parent escapes staging tree: {name}")
    if path.is_symlink() and not replace_link:
        raise ValueError(f"unexpected symlink at guest write target: {name}")
    return path


def write_guest(root, name, content, mode=0o644):
    path = guest_destination(root, name)
    path.write_bytes(content)
    path.chmod(mode)
    os.chown(path, 0, 0)


def prepare_payload(root, envd, packages, epoch):
    target = guest_destination(root, ENVD_GUEST)
    shutil.copyfile(envd, target)
    target.chmod(0o755)
    write_guest(root, UNIT_GUEST, SERVICE.encode())
    link = guest_destination(root, LINK_GUEST)
    link.symlink_to(UNIT_GUEST)
    write_guest(root, "/etc/sandproof-base-packages.manifest", packages.read_bytes())
    # The CI SquashFS has all-root ownership, including ubuntu's 0750 home.
    # Restore this fixed account's home before running SDK commands as ubuntu.
    passwd = guest_destination(root, "/etc/passwd").read_text().splitlines()
    ubuntu = [line.split(":") for line in passwd if line.startswith("ubuntu:")]
    if len(ubuntu) != 1 or ubuntu[0][2:4] != ["1000", "1000"] or ubuntu[0][5] != "/home/ubuntu":
        raise ValueError("fixed base ubuntu account differs from expected identity")
    home = guest_destination(root, "/home/ubuntu")
    os.chown(home, 1000, 1000)
    for current, directories, files in os.walk(home, followlinks=False):
        for name in directories + files:
            os.chown(Path(current) / name, 1000, 1000, follow_symlinks=False)
    # SDK user selection is setuid in envd; no shared login password/key is needed.
    shadow = guest_destination(root, "/etc/shadow")
    entries = [line.split(":") for line in shadow.read_text().splitlines()]
    for fields in entries:
        if len(fields) != 9:
            raise ValueError("unexpected shadow record")
        fields[1] = "!"
    write_guest(root, "/etc/shadow", ("\n".join(":".join(fields) for fields in entries) + "\n").encode(), 0o600)
    for key in (root / "etc/ssh").glob("ssh_host_*_key*"):
        guest_destination(root, "/" + str(key.relative_to(root))).unlink()
    ssh = root / "root/.ssh"
    if ssh.exists() or ssh.is_symlink():
        guest_destination(root, "/root/.ssh")
        shutil.rmtree(ssh)
    for name in ("/etc/systemd/system/ssh.service", "/etc/systemd/system/ssh.socket"):
        target = guest_destination(root, name, replace_link=True)
        target.unlink(missing_ok=True)
        target.symlink_to("/dev/null")
    guest_destination(root, "/etc/systemd/system/serial-getty@ttyS0.service.d/override.conf").unlink()
    write_guest(root, "/etc/machine-id", b"", 0o444)
    guest_destination(root, "/var/lib/dbus").mkdir(mode=0o755, exist_ok=True)
    dbus_id = guest_destination(root, "/var/lib/dbus/machine-id", replace_link=True)
    dbus_id.unlink(missing_ok=True)
    dbus_id.symlink_to("/etc/machine-id")
    seed = guest_destination(root, "/var/lib/systemd/random-seed")
    seed.unlink(missing_ok=True)
    # Stable input mtimes; filesystem-tool details remain part of the build report.
    for current, directories, files in os.walk(root, followlinks=False):
        for name in directories + files:
            os.utime(Path(current) / name, (epoch, epoch), follow_symlinks=False)
    os.utime(root, (epoch, epoch))


def verify_ubuntu_execution_paths(output):
    """Check the fixed ordinary user's cwd and /bin/sh execution path."""
    for name in ("/", "/home", "/usr", "/usr/bin", "/usr/bin/dash"):
        metadata = debugfs(output, f"stat {name}")
        expected_type = "regular" if name == "/usr/bin/dash" else "directory"
        if f"Type: {expected_type}" not in metadata or "Mode:  0755" not in metadata:
            raise ValueError(f"ubuntu cannot traverse or execute fixed path: {name}")
    home = debugfs(output, "stat /home/ubuntu")
    if ("Type: directory" not in home or "Mode:  0750" not in home
            or not re.search(r"User:\s+1000\s+Group:\s+1000", home)):
        raise ValueError("ubuntu home must be owned and accessible by the fixed non-root user")
    for name, target in (("/bin", "usr/bin"), ("/bin/sh", "dash")):
        metadata = debugfs(output, f"stat {name}")
        if f'Fast link dest: "{target}"' not in metadata:
            raise ValueError(f"unexpected fixed shell link: {name}")


def verify_image(output, work, profile):
    extracted = work / "installed-envd"
    debugfs(output, f"dump {ENVD_GUEST} {extracted}")
    if digest(extracted) != profile["envd"]["sha256"]:
        raise ValueError("installed envd fingerprint mismatch")
    if debugfs(output, f"cat {UNIT_GUEST}") != SERVICE:
        raise ValueError("installed service content mismatch")
    if f'Fast link dest: "{UNIT_GUEST}"' not in debugfs(output, f"stat {LINK_GUEST}"):
        raise ValueError("envd startup link missing")
    metadata = debugfs(output, f"stat {ENVD_GUEST}")
    if "Mode:  0755" not in metadata:
        raise ValueError("installed envd mode mismatch")
    verify_ubuntu_execution_paths(output)
    manifest = work / "installed-packages"
    debugfs(output, f"dump /etc/sandproof-base-packages.manifest {manifest}")
    if digest(manifest) != profile["packages"]["sha256"]:
        raise ValueError("installed package manifest mismatch")
    shadow = debugfs(output, "cat /etc/shadow").splitlines()
    if not shadow or not any(line.startswith("root:") for line in shadow):
        raise ValueError("guest shadow is missing or empty")
    for line in shadow:
        if len(line.split(":")) != 9 or line.split(":")[1] != "!":
            raise ValueError("guest login passwords must be locked")
    machine_id = debugfs(output, "stat /etc/machine-id")
    if "Type: regular" not in machine_id or "Size: 0" not in machine_id:
        raise ValueError("base must not contain a machine identity")
    for name in ("ssh.service", "ssh.socket"):
        if 'Fast link dest: "/dev/null"' not in debugfs(output, f"stat /etc/systemd/system/{name}"):
            raise ValueError("guest SSH must be masked")
    for name in ("/root/.ssh", "/etc/systemd/system/serial-getty@ttyS0.service.d/override.conf"):
        if "Inode:" in debugfs(output, f"stat {name}"):
            raise ValueError(f"unexpected inherited login configuration: {name}")


def build_tools():
    result = {}
    for path, flag in (("/usr/bin/unsquashfs", "-version"), ("/usr/sbin/mke2fs", "-V"),
                       ("/usr/sbin/e2fsck", "-V"), ("/usr/sbin/debugfs", "-V")):
        version = subprocess.run([path, flag], capture_output=True, text=True, timeout=10)
        # squashfs-tools 4.5 reports its version with exit status 1.
        if version.returncode != 0 and not (path.endswith("/unsquashfs") and version.returncode == 1
                                           and version.stdout.startswith("unsquashfs version ")):
            version.check_returncode()
        result[path] = {"version": (version.stdout + version.stderr).strip().splitlines()[0],
                        "sha256": digest(Path(path))}
    result["/etc/mke2fs.conf"] = {"sha256": digest(Path("/etc/mke2fs.conf"))}
    return result


def build(squashfs, envd, packages, output):
    profile = json.loads(PROFILE.read_text())
    if os.geteuid() != 0:
        raise PermissionError("fixed-base construction requires root to preserve guest file ownership")
    trusted_directory(output.parent)
    if output.exists() or output.is_symlink():
        raise ValueError("output must be new; existing images are never edited")
    host_tools = build_tools()
    with tempfile.TemporaryDirectory(prefix=".base-stage-", dir=output.parent) as scratch:
        work = safe_path(scratch)
        for key, source in (("squashfs", squashfs), ("envd", envd), ("packages", packages)):
            verified_copy(source, work / key, profile[key])
        root = work / "root"
        subprocess.run(["/usr/bin/unsquashfs", "-processors", "2", "-no-progress", "-d", str(root), str(work / "squashfs")],
                       check=True, stdout=sys.stderr, timeout=180)
        prepare_payload(root, work / "envd", work / "packages", profile["image"]["source_date_epoch"])
        with output.open("xb") as disk:
            disk.truncate(profile["image"]["bytes"])
        config = profile["image"]
        environment = {"PATH": "/usr/sbin:/usr/bin:/bin", "LC_ALL": "C",
                       "E2FSPROGS_FAKE_TIME": str(config["source_date_epoch"])}
        subprocess.run(["/usr/sbin/mke2fs", "-t", "ext4", "-F", "-b", "4096", "-m", "0",
                        "-U", config["uuid"], "-L", config["label"],
                        "-E", f"lazy_itable_init=0,lazy_journal_init=0,hash_seed={config['uuid']}",
                        "-d", str(root), str(output)], check=True, env=environment, stdout=sys.stderr, timeout=180)
        subprocess.run(["/usr/sbin/e2fsck", "-fn", str(output)], check=True, stdout=sys.stderr, timeout=60)
        verify_image(output, work, profile)
    output.chmod(0o444)
    print(json.dumps({"profile": profile["profile"], "profile_sha256": digest(PROFILE),
                      "builder_sha256": digest(Path(__file__)), "tools": host_tools,
                      "envd_sha256": profile["envd"]["sha256"],
                      "squashfs_sha256": profile["squashfs"]["sha256"],
                      "image": str(output), "image_sha256": digest(output),
                      "image_bytes": output.stat().st_size,
                      "scope": "fixed base payload; KVM, host quotas and per-task initialization require separate acceptance"}, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--squashfs", type=safe_path, required=True)
    parser.add_argument("--envd", type=safe_path, required=True)
    parser.add_argument("--packages", type=safe_path, required=True)
    parser.add_argument("--output", type=safe_path, required=True)
    args = parser.parse_args()
    try:
        build(args.squashfs, args.envd, args.packages, args.output)
    except BaseException:
        print(f"base preparation failed; inspect any partial output at {args.output}", file=sys.stderr)
        raise
