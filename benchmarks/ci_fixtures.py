"""Pinned, operator-prepared materials for a small, network-disabled CI gate.

This is not a template builder or original-environment benchmark acceptance.
The trusted preparer adds only platform bootstrap and fixed scoring dependencies.
The harness must execute the guest preflight before uploading oracle/test files.
"""
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import stat

from benchmarks.ci_verifier import PYTHON, REQUIREMENTS_SHA256
from tools.benchmark_tasks import InvalidInput, canonical, pin_for

MIB = 1024 * 1024
MAX_MANIFEST_BYTES = 128 * 1024
FORBIDDEN_PATHS = ('/solution', '/tests', '/logs/verifier/reward.txt')
SHA256 = re.compile(r'[0-9a-f]{64}')
CI_CATALOG = Path(__file__).with_name('terminal-bench-2.0-suites.json')


def ci_task_names(lock):
    """Resolve the sole reviewed small-CI profile, without task-name branches."""
    try:
        catalog = json.loads(CI_CATALOG.read_text(), object_pairs_hook=_unique_json)
        pin = pin_for(lock)
        if (lock.get('dataset') != 'terminal-bench@2.0'
                or lock.get('source') != dict(repository=pin.repository, commit=pin.commit)
                or type(catalog.get('schema_version')) is not int or catalog['schema_version'] != 2
                or catalog.get('dataset') != lock['dataset'] or catalog.get('source_commit') != pin.commit
                or catalog.get('source_kind') not in ('oci', 'dockerfile')):
            raise InvalidInput('CI selection catalog differs from the fixed 2.0 source')
        tasks = catalog['profiles']['ci']['tasks']
        frozen_names = [entry['name'] for entry in lock['tasks']]
        if (not isinstance(tasks, list) or len(tasks) != 2
                or not all(isinstance(row, dict) for row in tasks)):
            raise InvalidInput('CI selection requires exactly two task entries')
        names = tuple(row['name'] for row in tasks)
        overrides = catalog.get('oracle_modes', {})
        if (not isinstance(overrides, dict)
                or any(not isinstance(n, str) or re.fullmatch(r'[a-z0-9][a-z0-9-]{0,51}', n) is None
                       for n in names)
                or len(set(names)) != 2 or len(set(frozen_names)) != len(frozen_names)
                or not set(names) <= set(frozen_names)
                or any(n in overrides for n in names)):
            raise InvalidInput('CI selection requires unique frozen tasks with original oracles')
        return names
    except (OSError, UnicodeError, json.JSONDecodeError, KeyError, TypeError, AttributeError) as error:
        raise InvalidInput('CI selection catalog is unavailable or malformed: ' + type(error).__name__) from error


def _keys(value, expected, label):
    if not isinstance(value, dict) or set(value) != set(expected):
        raise InvalidInput('invalid CI fixture ' + label + ' fields')


def _hash(value, label):
    if not isinstance(value, str) or SHA256.fullmatch(value) is None:
        raise InvalidInput('invalid CI fixture ' + label + ' digest')
    return value


def _identity(info):
    return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns,
            info.st_uid, info.st_mode, info.st_nlink)


def _path(value, *, private, owner_uid=None):
    owner = os.geteuid() if owner_uid is None else owner_uid
    if (type(owner) is not int or not 0 <= owner <= 2**32 - 2
            or (private and owner != os.geteuid())):
        raise InvalidInput('explicit source owner or operator-owned private material required')
    if not isinstance(value, (str, Path)) or '\0' in str(value):
        raise InvalidInput('invalid CI fixture material path')
    path = Path(value)
    if not path.is_absolute() or path.resolve(strict=True) != path:
        raise InvalidInput('canonical nonsymlink CI fixture material required')
    info = path.lstat()
    if (not stat.S_ISREG(info.st_mode) or info.st_uid != owner
            or info.st_nlink != 1 or not info.st_size
            or info.st_mode & (0o077 if private else 0o022)):
        raise InvalidInput('ordinary trusted CI fixture material required')
    # Sticky root-owned temporary ancestors are safe for an owner-only child;
    # other writable ancestors could replace the supposedly trusted materials.
    for parent in path.parents:
        metadata = parent.lstat()
        sticky_root = bool(metadata.st_mode & stat.S_ISVTX) and metadata.st_uid == 0
        if (not stat.S_ISDIR(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode)
                or metadata.st_uid not in (0, os.geteuid(), owner)
                or (metadata.st_mode & 0o022 and not sticky_root)):
            raise InvalidInput('unsafe CI fixture material ancestry')
    return path, info


def _read(path, *, private=False, maximum=None, expected_identity=None, owner_uid=None):
    path, info = _path(path, private=private, owner_uid=owner_uid)
    if expected_identity is not None and _identity(info) != expected_identity:
        raise InvalidInput('CI fixture material identity changed')
    if maximum is not None and info.st_size > maximum:
        raise InvalidInput('CI fixture material exceeds its byte bound')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    result = hashlib.sha256()
    data = [] if maximum is not None else None
    total = 0
    try:
        if _identity(os.fstat(fd)) != _identity(info):
            raise InvalidInput('CI fixture material changed before read')
        while block := os.read(fd, MIB):
            total += len(block)
            if maximum is not None and total > maximum:
                raise InvalidInput('CI fixture material grew beyond its byte bound')
            result.update(block)
            if data is not None:
                data.append(block)
        if (_identity(os.fstat(fd)) != _identity(info)
                or _identity(_path(path, private=private, owner_uid=owner_uid)[1]) != _identity(info)):
            raise InvalidInput('CI fixture material changed during read')
    finally:
        os.close(fd)
    return result.hexdigest(), _identity(info), b''.join(data) if data is not None else None


def _unique_json(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise InvalidInput('duplicate CI fixture manifest field')
        value[key] = item
    return value


def _startup(value):
    _keys(value, ('env_vars', 'user', 'workdir', 'argv'), 'startup')
    user, cwd, env, argv = (value[k] for k in ('user', 'workdir', 'env_vars', 'argv'))
    if (not isinstance(user, str) or re.fullmatch(r'[A-Za-z_][A-Za-z0-9_-]{0,63}', user) is None
            or not isinstance(cwd, str) or not cwd.startswith('/') or '\\' in cwd or '\0' in cwd
            or '..' in cwd.split('/') or len(cwd.encode()) > 4096
            or not isinstance(env, dict) or len(env) > 256
            or not isinstance(argv, list) or len(argv) > 256):
        raise InvalidInput('unsupported CI fixture startup defaults')
    if any(not isinstance(k, str) or re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*', k) is None
           or not isinstance(v, str) or '\0' in v or len(k.encode()) + len(v.encode()) > 8192
           for k, v in env.items()):
        raise InvalidInput('unsupported CI fixture startup environment')
    if (any(not isinstance(arg, str) or '\0' in arg or len(arg.encode()) > 8192 for arg in argv)
            or (argv and not argv[0])
            or sum(len(k.encode()) + len(v.encode()) for k, v in env.items())
               + sum(len(arg.encode()) for arg in argv) > 32 * 1024):
        raise InvalidInput('unsupported CI fixture startup argv or byte limit')
    return canonical(value)


def guest_preflight_command():
    """Caller must run this in the newly created guest, before phase uploads."""
    return '\n'.join(['set -eu', *[
        'if test -e ' + shlex.quote(path) + ' || test -L ' + shlex.quote(path) + '; then exit 1; fi'
        for path in FORBIDDEN_PATHS], 'test -x ' + shlex.quote(PYTHON)])


@dataclass(frozen=True)
class CIFixture:
    name: str
    files_sha256: str
    reference: str
    digest: str
    kernel: Path
    kernel_sha256: str
    rootfs: Path
    rootfs_sha256: str
    fixture_sha256: str
    _manifest: Path
    _manifest_identity: tuple
    _kernel_identity: tuple
    _rootfs_identity: tuple
    _resources: bytes
    _startup: bytes

    @property
    def resources(self):
        return json.loads(self._resources)

    @property
    def startup(self):
        return json.loads(self._startup)

    def verify_artifacts(self):
        """Revalidate exact identities and bytes immediately before registration/use."""
        for path, expected, ident, private, bound in (
                (self._manifest, self.fixture_sha256, self._manifest_identity, True, MAX_MANIFEST_BYTES),
                (self.kernel, self.kernel_sha256, self._kernel_identity, False, None),
                (self.rootfs, self.rootfs_sha256, self._rootfs_identity, False, None)):
            observed, _, _ = _read(path, private=private, maximum=bound, expected_identity=ident)
            if observed != expected:
                raise InvalidInput('CI fixture material digest changed')
        return self.evidence()

    def evidence(self):
        return dict(mode='prepared-ci', fixture_sha256=self.fixture_sha256, name=self.name,
                    files_sha256=self.files_sha256, original_oci=dict(reference=self.reference, digest=self.digest),
                    kernel_sha256=self.kernel_sha256, rootfs_sha256=self.rootfs_sha256,
                    resources=self.resources, startup_sha256=hashlib.sha256(self._startup).hexdigest(),
                    verifier_python=PYTHON, verifier_requirements_sha256=REQUIREMENTS_SHA256,
                    required_absent_paths=list(FORBIDDEN_PATHS), guest_preflight='required_not_run',
                    public_network=False, original_environment=False, original_verifier=False,
                    original_oracle_verifier=False, original_test_assertions=True, original_oracle=True,
                    scope='prepared small CI fixtures; not generic-template or original-environment acceptance')

    def template_descriptor(self):
        self.verify_artifacts()
        shape = self.resources
        binding = dict(name=self.name, files_sha256=self.files_sha256,
                       rootfs_sha256=self.rootfs_sha256, startup=self.startup)
        # tpl- is reserved for the durable publication registry. Prepared CI
        # is a static operator binding and must not masquerade as a publication.
        identity = 'ci-' + hashlib.sha256(canonical(binding)).hexdigest()[:32]
        return dict(template_id=identity, revision='r1', owner=1,
                    kernel=dict(path=str(self.kernel), sha256=self.kernel_sha256, size_bytes=self._kernel_identity[2]),
                    rootfs=dict(path=str(self.rootfs), sha256=self.rootfs_sha256, size_bytes=self._rootfs_identity[2]),
                    resources=dict(cpu_count=shape['cpus'], memory_mib=shape['memory_mib'],
                                   disk_bytes=shape['disk_mib'] * MIB,
                                   host_memory_bytes=(shape['memory_mib'] + 256) * MIB),
                    envd_version='0.9.0', startup=self.startup)

    def guest_preflight_command(self):
        return guest_preflight_command()


def load_ci_fixtures(path, expected_sha256, lock, selected_names):
    """Validate an externally pinned private manifest, returning selected fixtures.

    The full manifest contains exactly two fixture identities; one-item subsets
    are allowed for serial batches. It cannot authorize a different task/version,
    public networking, altered original task files, or an arbitrary verifier.
    """
    try:
        return _load_ci_fixtures(path, expected_sha256, lock, selected_names)
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise InvalidInput('CI fixture material is unavailable or malformed: ' + type(error).__name__) from error


def _load_ci_fixtures(path, expected_sha256, lock, selected_names):
    expected_sha256 = _hash(expected_sha256, 'manifest')
    observed, manifest_identity, data = _read(path, private=True, maximum=MAX_MANIFEST_BYTES)
    if observed != expected_sha256:
        raise InvalidInput('CI fixture manifest differs from the operator pin')
    manifest = json.loads(data, object_pairs_hook=_unique_json)
    _keys(manifest, ('schema_version', 'dataset', 'source_commit', 'verifier_python',
                     'verifier_requirements_sha256', 'fixtures'), 'manifest')
    pin = pin_for(lock)
    if (type(manifest['schema_version']) is not int or manifest['schema_version'] != 1
            or manifest['dataset'] != lock['dataset'] or manifest['source_commit'] != pin.commit
            or lock.get('source') != dict(repository=pin.repository, commit=pin.commit)
            or manifest['verifier_python'] != PYTHON
            or manifest['verifier_requirements_sha256'] != REQUIREMENTS_SHA256):
        raise InvalidInput('CI fixture manifest differs from the fixed dataset/verifier')
    if (not isinstance(selected_names, (tuple, list)) or not selected_names
            or any(not isinstance(n, str) for n in selected_names)
            or len(set(selected_names)) != len(selected_names)):
        raise InvalidInput('unique explicit CI fixture task selection required')
    entries = {t['name']: t for t in lock['tasks']}
    if len(entries) != len(lock['tasks']):
        raise InvalidInput('duplicate frozen CI fixture task identity')
    fixtures = manifest['fixtures']
    if not isinstance(fixtures, list) or len(fixtures) != 2:
        raise InvalidInput('the small CI fixture manifest requires exactly two entries')
    selected = {}
    names = set()
    for row in fixtures:
        _keys(row, ('name', 'files_sha256', 'original_oci', 'kernel', 'rootfs',
                    'resources', 'startup', 'absent_paths'), 'entry')
        name = row['name']
        if (not isinstance(name, str) or re.fullmatch(r'[a-z0-9][a-z0-9-]{0,51}', name) is None
                or name in names or name not in entries):
            raise InvalidInput('duplicate, unsupported or unlocked CI fixture task')
        names.add(name)
        entry = entries[name]
        if _hash(row['files_sha256'], 'task files') != entry['files_sha256']:
            raise InvalidInput('CI fixture task files differ from the frozen task')
        _keys(row['original_oci'], ('reference', 'digest'), 'original OCI')
        image = row['original_oci']
        if (image['reference'] != entry['upstream_image_reference']
                or not isinstance(image['digest'], str)
                or re.fullmatch(r'sha256:[0-9a-f]{64}', image['digest']) is None):
            raise InvalidInput('CI fixture original OCI identity differs')
        _keys(row['resources'], ('cpus', 'memory_mib', 'disk_mib'), 'resources')
        shape = row['resources']
        if (any(type(v) is not int or v <= 0 for v in shape.values())
                or not 1 <= shape['cpus'] <= 255 or not 128 <= shape['memory_mib'] <= 16384
                or not 1 <= shape['disk_mib'] <= 32768
                or any(v != entry['requirements'][k] for k, v in shape.items())):
            raise InvalidInput('CI fixture resource shape differs from the frozen task')
        startup = _startup(row['startup'])
        if row['absent_paths'] != list(FORBIDDEN_PATHS):
            raise InvalidInput('CI fixture must exclude original phase files and prior reward')
        for label in ('kernel', 'rootfs'):
            _keys(row[label], ('path', 'sha256'), label)
            _hash(row[label]['sha256'], label)
            material_path = row[label]['path']
            if (not isinstance(material_path, str) or '\0' in material_path
                    or not Path(material_path).is_absolute() or '..' in Path(material_path).parts):
                raise InvalidInput('invalid CI fixture material path')
        if name not in selected_names:
            continue
        kernel = Path(row['kernel']['path'])
        rootfs = Path(row['rootfs']['path'])
        kernel_hash, kernel_identity, _ = _read(kernel)
        rootfs_hash, rootfs_identity, _ = _read(rootfs)
        if (kernel_hash != row['kernel']['sha256'] or rootfs_hash != row['rootfs']['sha256']
                or rootfs_identity[2] != shape['disk_mib'] * MIB):
            raise InvalidInput('CI fixture artifact digest or total disk size differs')
        selected[name] = CIFixture(name, row['files_sha256'], image['reference'], image['digest'],
            kernel, kernel_hash, rootfs, rootfs_hash, expected_sha256, Path(path), manifest_identity,
            kernel_identity, rootfs_identity, canonical(shape), startup)
    if set(selected_names) != set(selected):
        raise InvalidInput('selected CI task lacks an approved fixture')
    return {name: selected[name] for name in selected_names}
