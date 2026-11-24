"""Pinned source inputs shared by the native and formal composition checks."""

import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
SOURCE = '60aad8af23e317f5b1ba913518603d1fda015f4c'
SOURCE_ARCHIVE = '0d04c3d18fd78248f7068412bca199437eae3351e9fca3973506bd3f56c4f358'
SAVE_PARENT = '0b00572ed94fe72ae8cb94b9919f081c9ab1e892'
PROVIDERS = '5f84dbf199c19ffb4505ca657d1c9fb11905e65d'
STAGE1 = Path('verification/experiments/stage1-protocol')
OLD_PACKAGE = STAGE1 / 'unload-parent-link'
SAVE_PACKAGE = Path('verification/experiments/source-save-order/unload-parent-link')
CODEC = Path('controller-service/src/controller/lifecycle_inbox/source_work/codec.rs')
LIB = Path('controller-service/src/lib.rs')
NAMESPACE = 'SandproofSaveRetention'
MODES = ('correct', 'binding-only', 'parent')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def pinned(commit, path):
    return subprocess.check_output(['git', 'show', f'{commit}:{path}'], cwd=ROOT)


def provider(path):
    return pinned(PROVIDERS, path)


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def snapshot(output):
    archive = output / 'source.tar'
    with archive.open('xb') as stream:
        subprocess.run(['git', 'archive', SOURCE], cwd=ROOT, stdout=stream, check=True)
    if sha(archive) != SOURCE_ARCHIVE:
        raise ValueError('the exact current integration archive is required')
    source = output / 'source'
    source.mkdir(mode=0o700)
    with tarfile.open(archive) as contents:
        contents.extractall(source, filter='data')
    return source


def helpers():
    # Keep the original extraction/admission helpers. A later integration head
    # must be reviewed explicitly rather than silently changing this input.
    for directory in (SAVE_PACKAGE, OLD_PACKAGE):
        names = subprocess.check_output(
            ['git', 'ls-tree', '-r', '--name-only', SAVE_PARENT, str(directory)],
            cwd=ROOT, text=True).splitlines()
        for name in names:
            path = ROOT / name
            if not path.is_file() or path.is_symlink() or path.read_bytes() != pinned(SAVE_PARENT, name):
                raise ValueError('original source-save helper differs: ' + name)
    sys.path.extend((str(ROOT / SAVE_PACKAGE), str(ROOT / OLD_PACKAGE), str(ROOT)))
    import source_save_order
    return source_save_order


def stage(source, mode):
    save = helpers()
    if mode not in MODES:
        raise ValueError('unknown composition view')
    save.stage(source, 'factored')
    path = source / CODEC
    text = path.read_text()
    start = text.index('fn retains_saved_records(')
    end = text.index('\nfn write_candidate_records(', start)
    scan = provider(OLD_PACKAGE / 'first-match/retains.rs.in').decode()
    loops = scan[scan.index('fn retains_inventory_using<'):]
    if mode == 'binding-only':
        previous = 'if Check::same_work(next, record) {'
        if loops.count(previous) != 1:
            raise ValueError('the actual first complete comparison must occur once')
        loops = loops.replace(previous, 'if Check::same_binding(next, record) {')
    key_port = provider(OLD_PACKAGE / 'retention-providers/key-port.rs.in').decode()
    text = text[:start] + key_port + '\n\n' + loops + '\n' + (HERE / 'bridge.rs.in').read_text() + text[end:]
    old = 'save_using::<_, _, _, _, _, _, JournalSourceSave>(connection, sources, max_sessions)'
    new = 'save_retaining_using::<_, _, _, _, _, JournalSourceSave, JournalSavedKeys>(connection, sources, max_sessions)'
    if text.count(old) != 1:
        raise ValueError('the original parent must delegate the same complete arguments')
    path.write_text(text.replace(old, new))
    with (source / LIB).open('ab') as stream:
        stream.write(provider(STAGE1 / 'source-work-match/probe.rs.in'))
    return {str(name): sha(source / name) for name in (CODEC, LIB)}
