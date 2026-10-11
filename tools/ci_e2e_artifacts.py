"""Strict candidate identity for the unique center/host KVM assembly.

This is the shared preparation/report boundary, not a second runtime mode.
It records two protected binaries from one approved candidate. It does not
claim they were executed, that HTTPS/KVM worked, or that a DK case passed.
Preparation and the supervisor use this same boundary; a legacy single-server
report is not compatible. Source wiring alone is not deployed CI evidence.
"""

import hashlib
import os
from pathlib import Path
import re
import stat

from tools.check_kvm_host import absolute_path, require_keys, trusted_path
from tools.ci_redis import BINARY_LIMIT as REDIS_BINARY_LIMIT, PIN as REDIS_PIN
from tools.service_cgroup import service_identity
from tools.system_acceptance import json_file, private_path, write_once


ARCHITECTURE = "center-host-v1"
BINARIES = {"center": "sandproof-center", "host": "sandproof-host"}
VERSION_KEYS = {"internal_protocol", "center_store", "host_store"}
MANIFEST_KEYS = {"schema_version", "architecture", "candidate", "versions", "artifacts"}
EXECUTION_KEYS = {"schema_version", "run_id", "candidate_manifest_sha256", "services",
                  "roles", "redis", "tls_sha256", "budget_sha256"}
ROLE_KEYS = {"uid", "gid", "config_sha256", "directory"}
DIRECTORY_KEYS = {"device", "inode", "uid", "gid", "mode"}


def tls_material_files(host_roles):
    """Fixed role-specific leaves; one certificate cannot name two nodes.

    Rust validates actual CA, SAN and both TLS usages. This list only binds the
    operator's preprovisioned material to the exact bounded test cohort.
    """
    if tuple(host_roles) not in (('runtime',), ('runtime', 'builder')):
        raise ValueError('exact finite runtime/builder TLS roles required')
    return ('ca.der', 'center.der', 'center-key.der', *(
        name for role in host_roles for name in
        ('host-' + role + '.der', 'host-' + role + '-key.der')))


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
    require_public_identity(evidence, candidate=identity['candidate'])
    if evidence != identity:
        raise ValueError("reported services do not match the prepared candidate")
    return identity


def require_public_identity(evidence, *, candidate):
    """Validate persisted allowlisted identities without inventing binary paths.

    Used again when reducing already reclaimed batches; the supervisor still
    checks actual binaries against the original private candidate manifest.
    """
    _candidate(candidate)
    require_keys(evidence, {'architecture', 'candidate', 'versions',
                           'center_sha256', 'host_sha256'}, 'reported service identity')
    if evidence['architecture'] != ARCHITECTURE or evidence['candidate'] != candidate:
        raise ValueError('reported services have a foreign candidate or architecture')
    _versions(evidence['versions'])
    for role in BINARIES:
        _sha256(evidence[role + '_sha256'], role + ' service')
    return dict(evidence, versions=dict(evidence['versions']))


def require_boundary(observed):
    """One exact execution-boundary shape, shared by batch/final reduction."""
    boundary = dict(public_api="center", center_unprivileged=True,
                    redis_authority="validated", host_transport="mutual-tls",
                    host_public_management=False)
    if (observed != boundary or not isinstance(observed, dict)
            or any(type(observed[key]) is not type(value)
                   for key, value in boundary.items())):
        raise ValueError("public center/private host execution boundary is unobserved")
    return boundary


def require_service_boundary(report, manifest):
    """Shared prerequisite for the sole center/host fixture and reducer.

    This returns an allowlisted identity, NEVER a pass result. Original grading,
    DK cases, persisted identities and exact process/network/storage cleanup
    remain independently mandatory. Neither preparation nor an old report may
    claim the new production architecture was actually exercised.
    """
    if not isinstance(report, dict) or "server_sha256" in report:
        raise ValueError("single-server report cannot attest the new services")
    identity = require_report_identity(report.get("services"), manifest)
    boundary = require_boundary(report.get("service_boundary"))
    return dict(services=identity, service_boundary=boundary)


def _sha256(value, label):
    if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None:
        raise ValueError(f"exact {label} SHA256 required")
    return value


def _roles(value):
    """Validate only supervisor-reviewed facts, never another config schema."""
    require_keys(value, {"center", "host", "redis"}, "execution service roles")
    result = {}
    directories = set()
    configurations = set()
    host = value['host']
    if not isinstance(host, list) or not 1 <= len(host) <= 3:
        raise ValueError('host role requires one to three exact protected configurations')
    entries = [(role, facts) for role, facts in value.items() if role != 'host']
    entries.extend(('host', facts) for facts in host)
    result['host'] = []
    for role, facts in entries:
        require_keys(facts, ROLE_KEYS, "execution role facts")
        uid, gid = facts["uid"], facts["gid"]
        if role == "host":
            if type(uid) is not int or type(gid) is not int or (uid, gid) != (0, 0):
                raise ValueError("private host must use explicit root UID and GID")
        elif service_identity(uid, gid) is None:
            raise ValueError("center and Redis require explicit unprivileged identities")
        directory = facts["directory"]
        require_keys(directory, DIRECTORY_KEYS, "protected role directory facts")
        if (any(type(directory[key]) is not int for key in DIRECTORY_KEYS)
                or not 0 <= directory["device"] <= 2**64 - 1
                or not 0 < directory["inode"] <= 2**64 - 1
                or (directory["uid"], directory["gid"]) != (uid, gid)
                or directory["mode"] != 0o700):
            raise ValueError("role directory must have its exact identity, owner and 0700 mode")
        identity = directory["device"], directory["inode"]
        if identity in directories:
            raise ValueError("roles must use independent protected directory identities")
        directories.add(identity)
        config_sha = _sha256(facts["config_sha256"], "role configuration")
        if config_sha in configurations:
            raise ValueError('independent protected role configuration digests required')
        configurations.add(config_sha)
        checked = dict(uid=uid, gid=gid, config_sha256=config_sha, directory=dict(directory))
        if role == 'host':
            result[role].append(checked)
        else:
            result[role] = checked
    if result["center"]["uid"] == result["redis"]["uid"]:
        raise ValueError("Redis must be independent of the unprivileged center identity")
    return result


def execution_manifest(manifest, *, run_id, candidate_manifest_sha256, roles,
                       redis, tls_sha256, budget_sha256):
    """Original run binding, NOT execution, scoring or cleanup evidence.

    The trusted supervisor must first collect actual protected directory/owner
    facts and approved configuration/material digests. This pure contract does
    not touch the filesystem, launch a service, inspect secrets, allocate a fresh
    run identity or attest that those facts were observed. Instance-UID-pool
    disjointness belongs to the final validated configuration assembly, not a
    second configuration parser here. This helper never installs a supervisor
    or changes the promotion gate.
    """
    identity = public_identity(manifest)  # The sole candidate/version validator.
    checked_roles = _roles(roles)
    # This is the original fixed-material receipt, not another Redis pin/schema.
    require_keys(redis, {*REDIS_PIN, "binary_sha256", "binary_bytes"}, "fixed Redis material")
    if (any(redis[key] != value for key, value in REDIS_PIN.items())
            or type(redis["binary_bytes"]) is not int
            or not 0 < redis["binary_bytes"] <= REDIS_BINARY_LIMIT):
        raise ValueError("Redis must match the fixed bounded CI material")
    _sha256(redis["binary_sha256"], "Redis binary")
    return dict(schema_version=1, run_id=_sha256(run_id, "run identity"),
                candidate_manifest_sha256=_sha256(candidate_manifest_sha256, "candidate manifest"),
                services=identity, roles=checked_roles, redis=dict(redis),
                tls_sha256=_sha256(tls_sha256, "public TLS material"),
                budget_sha256=_sha256(budget_sha256, "complete resource budget"))


def require_execution_manifest(evidence, manifest, *, expected):
    """Bind a run report to exactly the already approved staging facts.

    Returning this allowlist is only a prerequisite. It deliberately contains
    no pass/Cleaned verdict. Live service identities, original DK and scoring,
    durable receipts and original exact resource cleanup remain mandatory.
    """
    for value in (expected, evidence):
        require_keys(value, EXECUTION_KEYS, "center/host execution manifest")
        if type(value["schema_version"]) is not int or value["schema_version"] != 1:
            raise ValueError("unsupported execution manifest schema")
        require_report_identity(value["services"], manifest)
        checked = execution_manifest(manifest, **{key: value[key] for key in EXECUTION_KEYS
                                                 - {"schema_version", "services"}})
        if checked != value:
            raise ValueError("execution manifest contains unapproved facts")
    if evidence != expected:
        raise ValueError("execution manifest differs from the approved run staging")
    return checked


def require_process_evidence(processes, execution):
    """Validate the bounded ORIGINAL live-observation receipt, not live PIDs.

    The collector checks /proc clocks, executable inodes, credentials and the
    existing service cgroup before producing these facts. This pure reducer
    only binds their exact role set to the approved configuration manifest;
    it never kills or treats an absent process as a cleaned VM.
    """
    roles=_roles(execution['roles'])
    if type(processes) is not dict:
        raise ValueError('actual service process observations required')
    expected={'center':roles['center'],'redis':roles['redis']}
    host_count=len(roles['host'])
    helper='host-helper' in processes
    native_count=host_count-int(helper)
    if not 1<=native_count<=2:
        raise ValueError('one or two exact private host process observations required')
    expected.update({'host-'+str(index):facts for index,facts in enumerate(roles['host'][:native_count])})
    if helper:
        expected['host-helper']=roles['host'][-1]
    require_keys(processes,set(expected),'actual service process set')
    seen=set()
    checked={}
    for name,facts in processes.items():
        require_keys(facts,{'pid','start_time_ticks','uid','gid','executable'},'actual service process facts')
        if (any(type(facts[key]) is not int for key in ('pid','start_time_ticks','uid','gid'))
                or not 0<facts['pid']<2**32 or not 0<facts['start_time_ticks']<2**64
                or (facts['uid'],facts['gid'])!=(expected[name]['uid'],expected[name]['gid'])
                or facts['pid'] in seen):
            raise ValueError('actual service process identity or role owner mismatch')
        seen.add(facts['pid'])
        require_keys(facts['executable'],{'device','inode'},'actual service executable facts')
        if (any(type(value) is not int for value in facts['executable'].values())
                or not 0<=facts['executable']['device']<2**64
                or not 0<facts['executable']['inode']<2**64):
            raise ValueError('actual service executable inode required')
        checked[name]={key:value for key,value in facts.items() if key!='executable'}
        checked[name]['executable']=dict(facts['executable'])
    return checked


def require_dk(evidence, *, candidate):
    """All six mandatory real fault cases; missing/unknown cannot be green.

    DK may use a separate fresh cohort from Harbor. Its one execution receipt
    is bound to the SAME approved candidate, and each real case keeps its own
    bounded private evidence digest. No task scoring is substituted by this.
    """
    _candidate(candidate)
    require_keys(evidence,{'schema_version','candidate','execution_manifest_sha256','cases'},'DK report')
    if (type(evidence['schema_version']) is not int or evidence['schema_version']!=1
            or evidence['candidate']!=candidate or type(evidence['cases']) is not list
            or len(evidence['cases'])!=6):
        raise ValueError('six exact-candidate mandatory DK cases required')
    execution_sha=_sha256(evidence['execution_manifest_sha256'],'DK execution manifest')
    names=set()
    checked=[]
    for case in evidence['cases']:
        require_keys(case,{'id','status','exact_cleanup','execution_manifest_sha256','evidence_sha256'},'DK case')
        if (type(case['id']) is not str or case['id'] not in {'DK01','DK02','DK03','DK04','DK05','DK06'} or case['id'] in names
                or case['status']!='pass' or case['exact_cleanup']!='passed'
                or case['execution_manifest_sha256']!=execution_sha):
            raise ValueError('missing, failed, unknown or mixed DK case')
        names.add(case['id'])
        _sha256(case['evidence_sha256'],'DK private case evidence')
        checked.append(dict(case))
    return dict(schema_version=1,candidate=candidate,execution_manifest_sha256=execution_sha,cases=checked)
