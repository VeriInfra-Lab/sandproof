"""Private, content-addressed inputs for validator algorithm tests.

These are historical reference inputs, never current production admission.
Unchanged LLBC/Lean artifacts are reused byte-for-byte; the archive only supplies
Git bytes no longer present in the checkout. No Git history is needed at runtime.
"""
import atexit
import hashlib
import json
from pathlib import Path
import tarfile
import tempfile

CHECKOUT = Path(__file__).resolve().parents[2]
DATA = Path(__file__).with_name('fixtures') / 'proof-inputs'
_temporary = None
_root = None


def reference_root():
    global _temporary, _root
    if _root is not None:
        return _root
    manifest = json.loads((DATA / 'manifest.json').read_text())
    archive = DATA / 'historical-delta.tar.gz'
    if hashlib.sha256(archive.read_bytes()).hexdigest() != manifest['archive_sha256']:
        raise ValueError('historical fixture archive changed')
    _temporary = tempfile.TemporaryDirectory(prefix='sandproof-proof-reference-')
    atexit.register(_temporary.cleanup)
    root = Path(_temporary.name)
    with tarfile.open(archive) as source:
        members = {member.name: member for member in source.getmembers()}
        if set(members) != set(manifest['archived_paths']):
            raise ValueError('historical fixture archive inventory changed')
        for name, record in manifest['files'].items():
            path = Path(name)
            if path.is_absolute() or '..' in path.parts:
                raise ValueError('unsafe historical fixture path')
            if name in members:
                member = members[name]
                if not member.isfile():
                    raise ValueError('historical fixture is not a regular file')
                data = source.extractfile(member).read()
            else:
                data = (CHECKOUT / name).read_bytes()
            if hashlib.sha256(data).hexdigest() != record['sha256']:
                raise ValueError('historical fixture source changed: ' + name)
            destination = root / path
            destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            destination.write_bytes(data)
            destination.chmod(0o600)
    _root = root
    return root
