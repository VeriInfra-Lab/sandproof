#!/usr/bin/env python3
"""Build a new CI-only ext4 image with the pinned E2B envd, without mounting it.

This accepts only the recorded Firecracker CI base image. It is not an OCI
builder and must not be used to parse arbitrary tenant-provided images.
"""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile


BASE_SHA256 = "144ab0e1581ec14b18b536eab8824ef8414da484a1e4d99773b1ab0ef94ef2d2"
ENVD_SHA256 = "ba9241eb02f6eb7a540b32f30966635d23dab35535811ecadc6965eb365164ab"
ENVD_COMMIT = "f455fd58fa3f289e78ce1de690c35968d8d29322"

SERVICE = """[Unit]
Description=Sandproof pinned E2B guest agent
Requires=fcnet.service
After=fcnet.service

[Service]
Type=simple
ExecStartPre=/usr/sbin/ip route replace 169.254.169.254/32 dev eth0
ExecStart=/usr/local/bin/envd -verbose
Restart=on-failure
RestartSec=1
StandardOutput=journal+console
StandardError=journal+console

[Install]
WantedBy=multi-user.target
"""


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def debugfs(image, request, *, writable=False):
    command = ["/usr/sbin/debugfs"]
    if writable:
        command.append("-w")
    result = subprocess.run(
        [*command, "-R", request, str(image)],
        check=True, capture_output=True, text=True, timeout=60,
    )
    # debugfs can return zero for failed requests. Verify every installed
    # object below instead of treating its exit code as an installation check.
    return result.stdout


def safe_path(value):
    path = Path(value).absolute()
    if not re.fullmatch(r"[A-Za-z0-9_./-]+", str(path)):
        raise ValueError("debugfs input paths must use simple ASCII path characters")
    return path


def build(base, envd, output):
    if digest(base) != BASE_SHA256 or digest(envd) != ENVD_SHA256:
        raise ValueError("input fingerprints differ from the pinned CI artifacts")
    if output.exists() or output.is_symlink():
        raise ValueError("output must be a new file; existing images are never edited")
    subprocess.run(["/usr/sbin/e2fsck", "-fn", str(base)], check=True, timeout=60)
    with output.open("xb") as destination, base.open("rb") as source:
        shutil.copyfileobj(source, destination, length=1024 * 1024)
    try:
        with tempfile.TemporaryDirectory(prefix="sandproof-image-") as work:
            work = safe_path(work)
            unit = work / "sandproof-envd.service"
            unit.write_text(SERVICE)
            debugfs(output, f"write {envd} /usr/local/bin/envd", writable=True)
            debugfs(output, "set_inode_field /usr/local/bin/envd mode 0100755", writable=True)
            debugfs(output, "set_inode_field /usr/local/bin/envd uid 0", writable=True)
            debugfs(output, "set_inode_field /usr/local/bin/envd gid 0", writable=True)
            debugfs(output, f"write {unit} /etc/systemd/system/sandproof-envd.service", writable=True)
            debugfs(output, "set_inode_field /etc/systemd/system/sandproof-envd.service mode 0100644", writable=True)
            debugfs(output, "set_inode_field /etc/systemd/system/sandproof-envd.service uid 0", writable=True)
            debugfs(output, "set_inode_field /etc/systemd/system/sandproof-envd.service gid 0", writable=True)
            link = "/etc/systemd/system/multi-user.target.wants/sandproof-envd.service"
            debugfs(output, f"symlink {link} /etc/systemd/system/sandproof-envd.service", writable=True)
            extracted = work / "envd-check"
            debugfs(output, f"dump /usr/local/bin/envd {extracted}")
            if digest(extracted) != ENVD_SHA256:
                raise ValueError("installed envd fingerprint mismatch")
            if debugfs(output, "cat /etc/systemd/system/sandproof-envd.service") != SERVICE:
                raise ValueError("installed service differs from expected content")
            if 'Fast link dest: "/etc/systemd/system/sandproof-envd.service"' not in debugfs(output, f"stat {link}"):
                raise ValueError("envd startup link was not installed")
            if "Mode:  0755" not in debugfs(output, "stat /usr/local/bin/envd"):
                raise ValueError("envd executable mode was not installed")
        subprocess.run(["/usr/sbin/e2fsck", "-fn", str(output)], check=True, timeout=60)
        if digest(base) != BASE_SHA256:
            raise ValueError("base image changed during preparation")
        output.chmod(0o444)
        print(json.dumps({
            "base_sha256": BASE_SHA256, "envd_commit": ENVD_COMMIT,
            "envd_sha256": ENVD_SHA256, "image_sha256": digest(output),
            "image": str(output), "scope": "CI guest only; no OCI build or production image claim",
        }, indent=2))
    except BaseException:
        # Keep this precise partial output for diagnosis. It cannot be mistaken
        # for a completed image because no successful manifest was emitted.
        print(f"image preparation failed; inspect partial output: {output}")
        raise


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=safe_path, required=True)
    parser.add_argument("--envd", type=safe_path, required=True)
    parser.add_argument("--output", type=safe_path, required=True)
    arguments = parser.parse_args()
    build(arguments.base, arguments.envd, arguments.output)
