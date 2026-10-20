"""Bounded immutable Rust guest build inputs, not an installation/pass receipt.

The trusted consumer pins the manifest SHA-256 separately. A manifest cannot
authorize itself, and package versions cannot stand in for binary fingerprints.
"""
import errno
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys

if __package__:
    from .check_kvm_host import require_keys, unique_keys
    from .system_acceptance import private_path
else:
    from check_kvm_host import require_keys, unique_keys
    from system_acceptance import private_path

BINARIES = {'service': 'sandproof-guest-service', 'worker': 'sandproof-guest-worker'}
SOURCE_LIMIT = 128 * 1024**2
BINARY_LIMIT = 256 * 1024**2
MANIFEST_LIMIT = 16 * 1024
CONFIGURATION_LIMIT = 1024 * 1024
VERSION_KEYS = {'service_package_version', 'worker_package_version', 'config_version',
                'worker_protocol_version', 'pid_format_version'}
TARGET = 'x86_64-unknown-linux-gnu'


def hex_digest(value, length=64):
    if type(value) is not str or re.fullmatch('[0-9a-f]{' + str(length) + '}', value) is None:
        raise ValueError('exact lowercase digest required')
    return value


def descriptor(value, limit):
    require_keys(value, {'bytes', 'sha256'}, 'artifact fingerprint')
    if type(value['bytes']) is not int or not 0 < value['bytes'] <= limit:
        raise ValueError('artifact byte limit')
    hex_digest(value['sha256'])
    return value


def _extent_digest(fd, length):
    """Hash every logical byte without reading filesystem-reported holes."""
    if sys.platform != 'linux' or not all(hasattr(os, key) for key in ('SEEK_DATA', 'SEEK_HOLE', 'pread')):
        return None
    digest, position = hashlib.sha256(), 0
    zeros = bytes(65536)
    while position < length:
        try:
            data = os.lseek(fd, position, os.SEEK_DATA)
        except OSError as error:
            if error.errno in (errno.EINVAL, errno.ENOSYS, errno.EOPNOTSUPP):
                return None
            if error.errno != errno.ENXIO:
                raise
            data = length
        else:
            if not position <= data < length:
                raise ValueError('invalid artifact data extent')
        while position < data:
            count = min(data-position, len(zeros))
            digest.update(zeros[:count])
            position += count
        if position == length:
            break
        try:
            end = min(os.lseek(fd, data, os.SEEK_HOLE), length)
        except OSError as error:
            if error.errno in (errno.EINVAL, errno.ENOSYS, errno.EOPNOTSUPP):
                return None
            raise
        if end <= data:
            raise ValueError('invalid artifact hole extent')
        while position < end:
            # Extent queries move the descriptor offset. Positioned reads also
            # avoid disagreement with a caller's buffered file-object offset.
            block = os.pread(fd, min(end-position, 65536), position)
            if not block:
                raise ValueError('artifact changed while reading')
            digest.update(block)
            position += len(block)
    return digest.hexdigest()


def file_digest(source, length):
    """Full SHA-256 of an opened regular file, with a bounded dense fallback.

    The caller retains its original type, size and inode/metadata checks.
    Unsupported extent queries restart BOTH the hash and read position, even
    after partial progress. Other I/O failures and malformed extents fail.
    """
    result = _extent_digest(source.fileno(), length)
    if result is not None:
        return result
    source.seek(0)
    digest, total = hashlib.sha256(), 0
    while block := source.read(min(65536, length-total+1)):
        total += len(block)
        if total > length:
            raise ValueError('artifact grew while reading')
        digest.update(block)
    if total != length:
        raise ValueError('artifact changed while reading')
    return digest.hexdigest()


def fingerprint(path, limit, *, executable=False, content=False, allow_empty=False, single_link=True):
    """Observe one regular inode with fixed reads and identity checks.

    Parents are trusted/private caller inputs, never tenant-selected paths.
    Same-owner/root mutation and rollback remain outside this observation.
    """
    before = path.lstat()
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    identity = lambda m: (m.st_dev, m.st_ino, m.st_size, m.st_mtime_ns, m.st_ctime_ns,
                          m.st_uid, m.st_gid, m.st_mode, m.st_nlink)
    result = bytearray() if content else None
    try:
        opened = os.fstat(fd)
        if (not stat.S_ISREG(opened.st_mode) or not int(not allow_empty) <= opened.st_size <= limit
                or identity(before) != identity(opened) or (single_link and opened.st_nlink != 1)
                or opened.st_uid not in (0, os.geteuid()) or opened.st_mode & 0o022
                or (executable and not opened.st_mode & 0o111)):
            raise ValueError('artifact type, identity, permissions or size')
        with os.fdopen(fd, 'rb', closefd=False) as source:
            if not content:
                sha256, total = file_digest(source, opened.st_size), opened.st_size
            else:
                digest, total = hashlib.sha256(), 0
                while block := source.read(min(65536, opened.st_size-total+1)):
                    total += len(block)
                    if total > opened.st_size:
                        raise ValueError('artifact grew while reading')
                    digest.update(block)
                    result.extend(block)
                sha256 = digest.hexdigest()
        if (total != opened.st_size or identity(os.fstat(fd)) != identity(opened)
                or identity(path.lstat()) != identity(opened)):
            raise ValueError('artifact changed while reading')
    finally:
        os.close(fd)
    value = {'bytes': total, 'sha256': sha256}
    return (value, bytes(result)) if content else value


def versions(value):
    require_keys(value, VERSION_KEYS, 'Rust guest interface versions')
    for key in VERSION_KEYS:
        if key.endswith('package_version'):
            if type(value[key]) is not str or re.fullmatch(r'[0-9]{1,5}\.[0-9]{1,5}\.[0-9]{1,5}', value[key]) is None:
                raise ValueError('exact guest package version required')
        elif type(value[key]) is not int or not 0 < value[key] <= (255 if key == 'pid_format_version' else 2**32-1):
            raise ValueError('positive bounded guest interface version required')
    return value


def check_reports(expected, service, worker):
    """Both actual binaries must match the candidate's original input lock."""
    versions(expected)
    common = {'target_os': 'linux', 'target_arch': 'x86_64'}
    worker_expected = dict(common, component=BINARIES['worker'],
                           package_version=expected['worker_package_version'],
                           worker_protocol_version=expected['worker_protocol_version'])
    service_expected = dict(common, component=BINARIES['service'],
                            package_version=expected['service_package_version'],
                            **{key: expected[key] for key in VERSION_KEYS - {'service_package_version'}})
    for actual, wanted in ((service, service_expected), (worker, worker_expected)):
        require_keys(actual, wanted, 'guest binary version report')
        if any(type(actual[key]) is not type(value) or actual[key] != value for key, value in wanted.items()):
            raise ValueError('guest binary versions differ from source input lock')


def validate(manifest):
    """Shape only; a caller must independently pin the complete manifest hash."""
    require_keys(manifest, {'schema_version', 'kind', 'source', 'versions', 'build', 'artifacts'}, 'guest release')
    if type(manifest['schema_version']) is not int or manifest['schema_version'] != 1 or manifest['kind'] != 'rust-guest-v1':
        raise ValueError('unsupported guest release')
    versions(manifest['versions'])
    source = manifest['source']
    require_keys(source, {'commit', 'tree', 'archive', 'cargo_lock_sha256', 'input_lock_sha256', 'recipe_sha256'}, 'guest source')
    for key in ('commit', 'tree'):
        hex_digest(source[key], 40)
    for key in ('cargo_lock_sha256', 'input_lock_sha256', 'recipe_sha256'):
        hex_digest(source[key])
    descriptor(source['archive'], SOURCE_LIMIT)
    build = manifest['build']
    require_keys(build, {'target', 'rustc', 'cargo', 'rustc_version', 'cargo_version'}, 'guest build')
    if build['target'] != TARGET:
        raise ValueError('unsupported guest build target')
    for name in ('rustc', 'cargo'):
        descriptor(build[name], BINARY_LIMIT)
        text = build[name + '_version']
        if type(text) is not str or not 0 < len(text.encode()) <= 4096 or '\0' in text:
            raise ValueError('bounded compiler version report required')
    require_keys(manifest['artifacts'], BINARIES, 'guest binaries')
    for artifact in manifest['artifacts'].values():
        descriptor(artifact, BINARY_LIMIT)
    return manifest


def load(directory, expected_sha256):
    """Recheck a separately pinned bundle before a template consumer copies it.

    Never execute bundle contents, create state, repair or accept missing files.
    The source archive is retained with the binaries for independent auditing.
    """
    manifest = load_runtime(directory, expected_sha256)
    if fingerprint(Path(directory) / 'source.tar', SOURCE_LIMIT) != manifest['source']['archive']:
        raise ValueError('guest source archive changed')
    return manifest


def load_runtime(directory, expected_sha256):
    """Verify the pinned runtime payload; source audit remains on the host."""
    directory = private_path(directory, directory=True)
    hex_digest(expected_sha256)
    observed, raw = fingerprint(directory / 'manifest.json', MANIFEST_LIMIT, content=True)
    if observed['sha256'] != expected_sha256:
        raise ValueError('guest manifest differs from trusted input lock')
    manifest = validate(json.loads(raw, object_pairs_hook=unique_keys))
    for role, name in BINARIES.items():
        if fingerprint(directory / name, BINARY_LIMIT, executable=True) != manifest['artifacts'][role]:
            raise ValueError('guest binary fingerprint changed')
    return manifest


def worker_configuration(manifest):
    """The exact v2 daemon fragment; callers still supply/review all budgets."""
    validate(manifest)
    return {'package_version': manifest['versions']['worker_package_version'],
            'protocol_version': manifest['versions']['worker_protocol_version'],
            'artifact': dict(manifest['artifacts']['worker'])}


def template_identity(directory, expected_sha256, configuration, configuration_sha256):
    """Bind separately pinned release/config bytes to the protected template.

    Only the version and worker relation are checked here. The original daemon
    check still validates the complete configuration and budgets before use;
    this function neither runs a binary nor claims the rootfs contains it.
    """
    manifest = load(directory, expected_sha256)
    return _configuration_identity(manifest, expected_sha256, configuration, configuration_sha256)


def runtime_template_identity(directory, expected_sha256, configuration, configuration_sha256):
    """Builder-side payload check, without transferring the audited archive."""
    manifest = load_runtime(directory, expected_sha256)
    return _configuration_identity(manifest, expected_sha256, configuration, configuration_sha256)


def _configuration_identity(manifest, expected_sha256, configuration, configuration_sha256):
    configuration = private_path(configuration)
    hex_digest(configuration_sha256)
    observed, raw = fingerprint(configuration, CONFIGURATION_LIMIT, content=True)
    if observed['sha256'] != configuration_sha256:
        raise ValueError('guest configuration differs from trusted input lock')
    config = json.loads(raw, object_pairs_hook=unique_keys)
    if (type(config) is not dict or type(config.get('version')) is not int
            or config['version'] != manifest['versions']['config_version']):
        raise ValueError('guest configuration version differs from release')
    worker = config.get('worker')
    require_keys(worker, {'package_version', 'protocol_version', 'artifact'}, 'configured worker')
    descriptor(worker['artifact'], BINARY_LIMIT)
    if (type(worker['protocol_version']) is not int
            or worker != worker_configuration(manifest)):
        raise ValueError('configured worker differs from release')
    return {'kind': manifest['kind'], 'manifest_sha256': expected_sha256,
            'source_commit': manifest['source']['commit'],
            'versions': dict(manifest['versions']),
            'artifacts': {role: dict(value) for role, value in manifest['artifacts'].items()},
            'configuration': observed}
