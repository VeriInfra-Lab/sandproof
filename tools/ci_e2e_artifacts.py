"""Strict candidate identity for the forthcoming center/host KVM assembly.

This is an unenabled preparation/report boundary, not a second runtime mode.
It records two protected binaries from one approved candidate. It does not
claim they were executed, that HTTPS/KVM worked, or that a DK case passed.
The production entry switch must wire this same boundary into both preparation
and the installed supervisor; a legacy single-server report is not compatible.
"""

import hashlib
import os
from pathlib import Path
import re
import stat

from tools.check_kvm_host import absolute_path, require_keys, trusted_path
from tools.system_acceptance import json_file, private_path, write_once


ARCHITECTURE = "center-host-v1"
BINARIES = {"center": "sandproof-center", "host": "sandproof-host"}
VERSION_KEYS = {"internal_protocol", "center_store", "host_store"}
MANIFEST_KEYS = {"schema_version", "architecture", "candidate", "versions", "artifacts"}


def _candidate(value):
    if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{40}", value) is None:
        raise ValueError("exact approved candidate commit required")


def _versions(value):
    require_keys(value, VERSION_KEYS, "service contract versions")
    if any(type(version) is not int or not 0 < version <= 2**32 - 1
           for version in value.values()):
        raise ValueError("positive bounded protocol/storage versions required")


def _paths(paths):
    require_keys(paths, BINARIES, "candidate service paths")
    result = {}
    for role, value in paths.items():
        path = absolute_path(value)
        if path.name != BINARIES[role] or str(path) != value:
            raise ValueError("candidate role must name its independent service binary")
        result[role] = path
    if len(set(result.values())) != len(BINARIES):
        raise ValueError("independent service artifact paths required")
    return result


def _fingerprint(path):
    """Read only the protected regular executable inode, checking replacement.

    The existing trusted-path rule covers every ancestor. A file changing while
    it is being sealed is rejected, not assigned a fingerprint for a mixed read.
    No binary is run here and no production or management credential is read.
    """
    path = trusted_path(path)
    before = path.lstat()
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as source:
        opened = os.fstat(source.fileno())
        identity = lambda info: (info.st_dev, info.st_ino, info.st_size,
                                 info.st_mtime_ns, info.st_ctime_ns)
        if (not stat.S_ISREG(opened.st_mode) or not opened.st_mode & 0o111
                or identity(opened) != identity(before)):
            raise ValueError("protected service executable identity changed")
        sha256 = hashlib.file_digest(source, "sha256").hexdigest()
        after = os.fstat(source.fileno())
        current = path.lstat()
        if identity(after) != identity(opened) or identity(current) != identity(opened):
            raise ValueError("service artifact changed while being fingerprinted")
    return sha256


def record(candidate, paths, versions):
    """Seal service identities after the approved same-candidate build finishes.

    Version values are supplied by that candidate's validated deployment
    contract; this module does not maintain another copy of Rust format numbers.
    Build provenance and clean/exact checkout checks remain preparation's duty.
    """
    _candidate(candidate)
    _versions(versions)
    resolved = _paths(paths)
    return dict(schema_version=1, architecture=ARCHITECTURE, candidate=candidate,
                versions=dict(versions), artifacts={role: dict(path=str(path), sha256=_fingerprint(path))
                                                    for role, path in resolved.items()})


def validate(manifest, *, candidate, paths, versions):
    """Pure shape/binding check; accepting it is NOT an execution attestation."""
    _candidate(candidate)
    _versions(versions)
    resolved = _paths(paths)
    require_keys(manifest, MANIFEST_KEYS, "center/host candidate manifest")
    if (type(manifest["schema_version"]) is not int or manifest["schema_version"] != 1
            or manifest["architecture"] != ARCHITECTURE or manifest["candidate"] != candidate):
        raise ValueError("center/host candidate manifest identity mismatch")
    _versions(manifest["versions"])
    if manifest["versions"] != versions:
        raise ValueError("candidate protocol/storage versions changed")
    require_keys(manifest["artifacts"], BINARIES, "candidate service artifacts")
    for role, artifact in manifest["artifacts"].items():
        require_keys(artifact, {"path", "sha256"}, "candidate service artifact")
        if (artifact["path"] != str(resolved[role]) or not isinstance(artifact["sha256"], str)
                or re.fullmatch(r"[0-9a-f]{64}", artifact["sha256"]) is None):
            raise ValueError("candidate service artifact binding mismatch")
    return manifest


def publish(path, *, candidate, paths, versions):
    """Durable no-clobber publication in an existing private run directory."""
    path = absolute_path(str(path))
    private_path(path.parent, directory=True)
    manifest = record(candidate, paths, versions)
    write_once(path, manifest)
    return manifest


def load(path, *, candidate, paths, versions):
    """Recheck a private persisted manifest and both actual protected binaries."""
    manifest = validate(json_file(private_path(path), limit=16 * 1024),
                        candidate=candidate, paths=paths, versions=versions)
    if manifest != record(candidate, paths, versions):
        raise ValueError("prepared center/host artifacts changed")
    return manifest


def public_identity(manifest):
    """Explicit allowlist for reports; never export private artifact paths."""
    require_keys(manifest, MANIFEST_KEYS, "center/host candidate manifest")
    require_keys(manifest["artifacts"], BINARIES, "candidate service artifacts")
    for artifact in manifest["artifacts"].values():
        require_keys(artifact, {"path", "sha256"}, "candidate service artifact")
    candidate = manifest["candidate"]
    paths = {role: artifact["path"] for role, artifact in manifest["artifacts"].items()}
    validate(manifest, candidate=candidate, paths=paths, versions=manifest["versions"])
    return dict(architecture=ARCHITECTURE, candidate=candidate,
                versions=dict(manifest["versions"]),
                center_sha256=manifest["artifacts"]["center"]["sha256"],
                host_sha256=manifest["artifacts"]["host"]["sha256"])


def require_report_identity(evidence, manifest):
    """Reject old, mixed or missing candidate evidence at the report boundary.

    This is only one prerequisite. The final reducer still must require actual
    center-entry SDK execution, authenticated host transport, DK outcomes,
    scoring and exact cleanup before it can publish a successful check.
    """
    identity = public_identity(manifest)
    require_keys(evidence, identity, "center/host reported artifact identity")
    _versions(evidence["versions"])
    if evidence != identity:
        raise ValueError("reported services do not match the prepared candidate")
    return identity


def require_service_boundary(report, manifest):
    """Unwired prerequisite for the atomic fixture/reducer cutover.

    This returns an allowlisted identity, NEVER a pass result. Original grading,
    DK cases, persisted identities and exact process/network/storage cleanup
    remain independently mandatory. Neither preparation nor an old report may
    claim the new production architecture was actually exercised.
    """
    if not isinstance(report, dict) or "server_sha256" in report:
        raise ValueError("single-server report cannot attest the new services")
    identity = require_report_identity(report.get("services"), manifest)
    boundary = dict(public_api="center", center_unprivileged=True,
                    redis_authority="validated", host_transport="mutual-tls",
                    host_public_management=False)
    observed = report.get("service_boundary")
    if (observed != boundary or not isinstance(observed, dict)
            or any(type(observed[key]) is not type(value)
                   for key, value in boundary.items())):
        raise ValueError("public center/private host execution boundary is unobserved")
    return dict(services=identity, service_boundary=boundary)
