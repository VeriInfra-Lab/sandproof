"""Small one-time prepared-CI material helpers; no host container execution.

Only the reviewed platform recipe, trusted uv bytes and fixed requirements enter
the build context. Original oracle/test files are never packaged. A guest builder
still owns all OCI handling; this module copies opaque ready artifacts only.
"""
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import tarfile

from benchmarks.ci_fixtures import FORBIDDEN_PATHS, MIB, _hash, _identity, _keys, _path, _read, _startup
from benchmarks.ci_verifier import PYTHON, REQUIREMENTS_SHA256, REQUIREMENTS_TEXT
from tools.benchmark_tasks import InvalidInput, canonical, pin_for
from tools.template_stage import copy_sparse, sync_directory

RECIPE = Path(__file__).with_name('ci-fixture.Dockerfile')
MAX_RECIPE_BYTES = 16 * 1024
MAX_UV_BYTES = 128 * MIB


def _directory(path):
    path = Path(path)
    if not path.is_absolute() or path.resolve(strict=True) != path:
        raise InvalidInput('canonical private CI preparation directory required')
    info = path.lstat()
    if (not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o077):
        raise InvalidInput('private operator-owned CI preparation directory required')
    for parent in path.parents:
        value = parent.lstat()
        sticky = value.st_uid == 0 and value.st_mode & stat.S_ISVTX
        if (not stat.S_ISDIR(value.st_mode) or stat.S_ISLNK(value.st_mode)
                or value.st_uid not in (0, os.geteuid()) or (value.st_mode & 0o022 and not sticky)):
            raise InvalidInput('unsafe CI preparation directory ancestry')
    return path, _identity(info)


def _entry(entry):
    name = entry.get('name') if isinstance(entry, dict) else None
    if not isinstance(name, str) or re.fullmatch(r'[a-z0-9][a-z0-9-]{0,51}', name) is None:
        raise InvalidInput('invalid frozen CI preparation task identity')
    files = entry.get('files')
    if (not isinstance(files, list) or not files or _hash(entry.get('files_sha256'), 'task files')
            != hashlib.sha256(canonical(files)).hexdigest()):
        raise InvalidInput('CI preparation frozen inventory digest differs')
    paths = [row.get('path') for row in files if isinstance(row, dict)]
    if len(paths) != len(files) or any(not isinstance(p, str) for p in paths) or len(set(paths)) != len(paths):
        raise InvalidInput('invalid frozen CI preparation file inventory')
    requirements = entry.get('requirements')
    if not isinstance(requirements, dict):
        raise InvalidInput('invalid frozen CI preparation requirements')
    shape = {key: requirements.get(key) for key in ('cpus', 'memory_mib', 'disk_mib')}
    if (any(type(v) is not int for v in shape.values()) or not 1 <= shape['cpus'] <= 255
            or not 128 <= shape['memory_mib'] <= 16384 or not 1 <= shape['disk_mib'] <= 32768):
        raise InvalidInput('invalid frozen CI preparation resource shape')
    reference = entry.get('upstream_image_reference')
    if (not isinstance(reference, str) or len(reference) > 512
            or re.fullmatch(r'[a-z0-9][a-z0-9._/:@-]*', reference) is None):
        raise InvalidInput('invalid frozen public OCI reference')
    return name, shape, reference


def _digest_reference(reference, digest):
    if not isinstance(digest, str) or re.fullmatch(r'sha256:[0-9a-f]{64}', digest) is None:
        raise InvalidInput('immutable original OCI digest required for CI preparation')
    if '@' in reference and reference.rsplit('@', 1)[1] != digest:
        raise InvalidInput('original OCI reference digest conflicts with its pin')
    # Drop a tag only from the last path component; preserve registry host ports.
    repository = reference.split('@', 1)[0]
    head, separator, tail = repository.rpartition('/')
    tail = tail.split(':', 1)[0]
    return (head + separator + tail) + '@' + digest


def archive_ci_context(entry, source, output, uv_path, base_digest):
    """Create a deterministic three-file context and return its byte SHA-256."""
    try:
        _, _, reference = _entry(entry)
        immutable = _digest_reference(reference, base_digest)
        relative = entry.get('source_path')
        if (not isinstance(relative, str) or not relative or PurePosixPath(relative).is_absolute()
                or PurePosixPath(relative).as_posix() != relative
                or '..' in PurePosixPath(relative).parts or '\\' in relative or '\0' in relative):
            raise InvalidInput('invalid original task source path')
        original = Path(source) / relative / 'environment/Dockerfile'
        declared = [row for row in entry['files'] if row['path'] == 'environment/Dockerfile']
        if len(declared) != 1 or declared[0].get('role') != 'environment':
            raise InvalidInput('original Dockerfile identity is missing')
        original_hash, ident, _ = _read(original, maximum=MAX_RECIPE_BYTES)
        if (original_hash != declared[0].get('sha256') or ident[2] != declared[0].get('bytes')
                or declared[0].get('mode') not in ('100644', '100755')
                or bool(ident[6] & 0o111) != (declared[0]['mode'] == '100755')):
            raise InvalidInput('original Dockerfile differs from the frozen task')
        _, _, recipe = _read(RECIPE, maximum=MAX_RECIPE_BYTES)
        lines = recipe.decode('utf-8').splitlines(keepends=True)
        if lines.count('ARG BASE_IMAGE\n') != 1 or lines.count('FROM ${BASE_IMAGE}\n') != 1:
            raise InvalidInput('reviewed CI fixture recipe base placeholders differ')
        recipe = ''.join('FROM ' + immutable + '\n' if line == 'FROM ${BASE_IMAGE}\n' else line
                         for line in lines if line != 'ARG BASE_IMAGE\n').encode()
        _, uv_identity, uv = _read(uv_path, maximum=MAX_UV_BYTES)
        if (not uv_identity[6] & 0o111 or len(uv) < 20 or uv[:7] != b'\x7fELF\x02\x01\x01'
                or int.from_bytes(uv[18:20], 'little') != 62):
            raise InvalidInput('trusted Linux amd64 uv executable required')
        output = Path(output)
        if not output.is_absolute() or '..' in output.parts or output.exists() or output.is_symlink():
            raise InvalidInput('CI context requires a new absolute output file')
        parent, parent_identity = _directory(output.parent)
        fd = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, 'wb') as incoming:
            with tarfile.open(fileobj=incoming, mode='w', format=tarfile.PAX_FORMAT) as archive:
                for name, data, mode in (('Dockerfile', recipe, 0o644), ('ci-uv', uv, 0o755),
                                         ('verifier-requirements.txt', REQUIREMENTS_TEXT.encode(), 0o644)):
                    member = tarfile.TarInfo(name)
                    member.size, member.mode, member.mtime = len(data), mode, 0
                    archive.addfile(member, io.BytesIO(data))
            incoming.flush()
            os.fsync(incoming.fileno())
        # Creating this one file changes directory mtime/ctime, not its identity.
        after = _directory(parent)[1]
        if after[:2] != parent_identity[:2]:
            raise InvalidInput('CI context output directory was replaced')
        sync_directory(parent)
        return _read(output, private=True)[0]
    except (OSError, UnicodeError) as error:
        raise InvalidInput('CI context material is unavailable: ' + type(error).__name__) from error


def publish_ready_fixture(entry, template, ready_rootfs, kernel, dest, original_digest, *, ready_owner_uid):
    """Copy ready opaque bytes into a new private fixture; never share source inodes.

    Failure preserves the precise partial directory for the caller's journaled
    cleanup; it does not delete, adopt, or overwrite an existing destination.
    """
    try:
        if type(ready_owner_uid) is not int or not 0 <= ready_owner_uid <= 2**32 - 2:
            raise InvalidInput('explicit approved Ready registry owner required')
        name, shape, reference = _entry(entry)
        _digest_reference(reference, original_digest)
        if (not isinstance(template, dict)
                or (template.get('cpuCount'), template.get('memoryMib'), template.get('diskBytes'))
                   != (shape['cpus'], shape['memory_mib'], shape['disk_mib'] * MIB)
                or not isinstance(template.get('imageDigest'), str)
                or re.fullmatch(r'sha256:[0-9a-f]{64}', template['imageDigest']) is None):
            raise InvalidInput('ready CI template differs from the task shape or immutable image')
        _startup(template.get('startup'))
        # The Ready registry belongs to the independently running center.
        # Only this source may use its explicit operator-approved UID; the
        # kernel, destination and final manifest remain operator-owned.
        root_hash, root_identity, _ = _read(ready_rootfs, owner_uid=ready_owner_uid)
        kernel_hash, kernel_identity, _ = _read(kernel)
        if (root_hash != _hash(template.get('rootfsSha256'), 'ready rootfs')
                or kernel_hash != _hash(template.get('kernelSha256'), 'ready kernel')
                or root_identity[2] != shape['disk_mib'] * MIB):
            raise InvalidInput('ready CI artifact fingerprint or logical disk size differs')
        ready_rootfs, kernel = Path(ready_rootfs), Path(kernel)
        dest = Path(dest)
        if not dest.is_absolute() or '..' in dest.parts or dest.exists() or dest.is_symlink():
            raise InvalidInput('CI fixture publication requires a new absolute destination')
        _directory(dest.parent)
        dest.mkdir(mode=0o700)
        destination_identity = _directory(dest)[1][:2]
        for source, target, expected, original, owner in (
                (ready_rootfs, dest / 'rootfs.ext4', root_hash, root_identity, ready_owner_uid),
                (kernel, dest / 'kernel', kernel_hash, kernel_identity, None)):
            if _identity(_path(source, private=False, owner_uid=owner)[1]) != original:
                raise InvalidInput('ready CI source identity changed before copying')
            copy_sparse(source, target)
            target.chmod(0o444)
            with target.open('rb') as copied:
                os.fsync(copied.fileno())
            observed, copied_identity, _ = _read(target)
            if (observed != expected or copied_identity[2] != original[2]
                    or copied_identity[:2] == original[:2]
                    or _identity(_path(source, private=False, owner_uid=owner)[1]) != original):
                raise InvalidInput('CI artifact copy content or source identity changed')
        if _directory(dest)[1][:2] != destination_identity:
            raise InvalidInput('CI fixture destination identity changed')
        sync_directory(dest)
        sync_directory(dest.parent)
        return dict(name=name, files_sha256=entry['files_sha256'],
                    original_oci=dict(reference=reference, digest=original_digest),
                    kernel=dict(path=str(dest / 'kernel'), sha256=kernel_hash),
                    rootfs=dict(path=str(dest / 'rootfs.ext4'), sha256=root_hash),
                    resources=shape, startup=json.loads(canonical(template['startup'])),
                    absent_paths=list(FORBIDDEN_PATHS))
    except OSError as error:
        raise InvalidInput('ready CI publication failed: ' + type(error).__name__) from error


def make_ci_fixture_manifest(items, lock):
    """Return the strict private envelope; the harness owns atomic publication."""
    pin = pin_for(lock)
    if lock.get('source') != dict(repository=pin.repository, commit=pin.commit):
        raise InvalidInput('CI fixture envelope requires the fixed source identity')
    if not isinstance(items, list) or len(items) != 2:
        raise InvalidInput('CI fixture envelope requires exactly two ready fixtures')
    entries = {entry['name']: entry for entry in lock['tasks']}
    seen = set()
    for item in items:
        _keys(item, ('name', 'files_sha256', 'original_oci', 'kernel', 'rootfs', 'resources',
                     'startup', 'absent_paths'), 'published entry')
        name = item['name']
        if not isinstance(name, str) or name in seen or name not in entries:
            raise InvalidInput('duplicate or unlocked prepared CI fixture')
        seen.add(name)
        _, shape, reference = _entry(entries[name])
        _keys(item['original_oci'], ('reference', 'digest'), 'original OCI')
        _digest_reference(reference, item['original_oci']['digest'])
        if (item['files_sha256'] != entries[name]['files_sha256'] or item['resources'] != shape
                or item['original_oci']['reference'] != reference or item['absent_paths'] != list(FORBIDDEN_PATHS)):
            raise InvalidInput('published CI fixture differs from its frozen task')
        _startup(item['startup'])
        for label in ('rootfs', 'kernel'):
            _keys(item[label], ('path', 'sha256'), label)
            actual, identity, _ = _read(item[label]['path'])
            if (actual != _hash(item[label]['sha256'], label)
                    or (label == 'rootfs' and identity[2] != shape['disk_mib'] * MIB)):
                raise InvalidInput('published CI material fingerprint or size changed')
    return dict(schema_version=1, dataset=lock['dataset'], source_commit=pin.commit,
                verifier_python=PYTHON, verifier_requirements_sha256=REQUIREMENTS_SHA256,
                fixtures=json.loads(canonical(items)))
