"""Compare already-read Cargo evidence by locked content, retaining raw locations.

Callers still read and validate every archive and unpacked file on each machine.
This projection changes only storage-location identity. It is not a cache reader,
an archive validator, or permission to reuse a result with different inputs.
"""
from pathlib import PurePosixPath
import json
import re


def _digest(value):
    if not isinstance(value, str) or re.fullmatch(r'[0-9a-f]{64}', value) is None:
        raise ValueError('invalid registry content digest')
    return value


def _location(value, kind):
    if not isinstance(value, str) or '\x00' in value or '\\' in value:
        raise ValueError('invalid registry location')
    path = PurePosixPath(value)
    if not path.is_absolute() or '..' in path.parts or str(path) != value:
        raise ValueError('non-canonical registry location')
    marker = '/registry/' + kind + '/'
    if marker not in value:
        raise ValueError('wrong registry location kind')
    root, relative = value.rsplit(marker, 1)
    parts = relative.split('/')
    if not root or len(parts) < 2 or any(not p for p in parts):
        raise ValueError('incomplete registry location')
    return root, parts


def content_identity(record, lock):
    """Preserve package/source, archive bytes, every source byte and multiplicity."""
    if not isinstance(record, dict) or set(record) != {'archives', 'files'}:
        raise ValueError('unexpected registry evidence fields')
    if not all(isinstance(record[k], dict) for k in record):
        raise ValueError('invalid registry evidence maps')
    locked = {}
    for package in lock['package']:
        if 'checksum' not in package:
            continue
        source = package.get('source')
        if not isinstance(source, str) or not source.startswith('registry+'):
            raise ValueError('unsupported locked package source')
        stem = package['name'] + '-' + package['version']
        if stem in locked:
            raise ValueError('ambiguous locked crate archive name')
        locked[stem] = dict(name=package['name'], version=package['version'],
                            source=source, archive_sha256=_digest(package['checksum']))
    providers = {}
    for physical, digest in record['archives'].items():
        root, parts = _location(physical, 'cache')
        if len(parts) != 2 or not parts[1].endswith('.crate'):
            raise ValueError('invalid crate archive location')
        stem = parts[1][:-len('.crate')]
        if stem not in locked or _digest(digest) != locked[stem]['archive_sha256']:
            raise ValueError('archive differs from locked package')
        key = (root, parts[0], stem)
        if key in providers:
            raise ValueError('duplicate physical crate provider')
        providers[key] = dict(locked[stem], files={})
    for physical, digest in record['files'].items():
        root, parts = _location(physical, 'src')
        if len(parts) < 3:
            raise ValueError('incomplete crate source location')
        key = (root, parts[0], parts[1])
        if key not in providers:
            raise ValueError('source has no locked archive provider')
        relative = '/'.join(parts[2:])
        if relative in providers[key]['files']:
            raise ValueError('duplicate relative crate source')
        providers[key]['files'][relative] = _digest(digest)
    if {key[2] for key in providers} != set(locked):
        raise ValueError('missing locked crate provider')
    if any(not provider['files'] for provider in providers.values()):
        raise ValueError('missing unpacked crate source evidence')
    # Keep duplicate providers as duplicate list entries, even if their bytes
    # are identical. Relocation must not hide a changed provider count.
    return sorted(providers.values(), key=lambda p: json.dumps(p, sort_keys=True))


def require_same_content(left, right, lock):
    if content_identity(left, lock) != content_identity(right, lock):
        raise ValueError('native and formal locked registry contents differ')

