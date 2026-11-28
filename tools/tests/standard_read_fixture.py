"""Original StandardRecord source for scheduling tests, never proof admission.

The current validator executes unchanged against the captured original Rust.
A controlled private mutation tests its source rejection; this fixture cannot
make a new production source satisfy an old capture.
"""
import atexit
import hashlib
import json
from pathlib import Path
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[2]
HERE = Path('verification/experiments/stage1-protocol/protocol-record/standard-read')
DATA = ROOT / 'tools/tests/fixtures/standard-record-read'
_temporary = None
_root = None


def reference_root():
    global _temporary, _root
    if _root is not None:
        return _root
    manifest = json.loads((DATA / 'manifest.json').read_text())
    archive = DATA / 'source.tar.gz'
    if (manifest['source_commit'] != 'd3004a46449c0a11982fb21a6853f56a43f3f689'
            or hashlib.sha256(archive.read_bytes()).hexdigest() != manifest['archive_sha256']):
        raise ValueError('original scheduling source fixture changed')
    _temporary = tempfile.TemporaryDirectory(prefix='sandproof-standard-read-reference-')
    atexit.register(_temporary.cleanup)
    root = Path(_temporary.name)
    with tarfile.open(archive) as tar:
        entries = tar.getmembers()
        if len(entries) != len(manifest['files']) or {m.name for m in entries} != set(manifest['files']):
            raise ValueError('missing, extra or duplicate original scheduling source')
        for member in entries:
            name = member.name
            path = Path(name)
            expected = manifest['files'][name]
            mode = 0o755 if expected['mode'] == '100755' else 0o644
            if (path.is_absolute() or '..' in path.parts or path.as_posix() != name
                    or not member.isfile() or member.mode != mode):
                raise ValueError('invalid original scheduling source member')
            data = tar.extractfile(member).read()
            if hashlib.sha256(data).hexdigest() != expected['sha256']:
                raise ValueError('original scheduling source bytes/mode changed: ' + name)
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            target.chmod(mode)
    capture = json.loads((root / HERE / 'capture.json').read_text())
    view = json.loads((root / HERE / 'view.json').read_text())
    names = {str(HERE / name) for name in ('formal.py', 'view.json', 'capture.json', 'root.rs.in', 'support.py')}
    names.update(row['source'] for row in view['files'].values())
    names.update(capture['source_inputs'])
    if (set(manifest['source_inputs']) != set(capture['source_inputs'])
            or names != set(manifest['files'])):
        raise ValueError('original scheduling capture/provider closure changed')
    for name, expected in capture['source_inputs'].items():
        record = manifest['files'][name]
        if record['sha256'] != expected or record['mode'] != capture['source_modes'][name]:
            raise ValueError('original scheduling capture source binding changed: ' + name)
    _root = root
    return root
