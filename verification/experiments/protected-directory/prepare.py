"""Materialize frozen real sources; only the explicit mutant changes a business bitmask."""
import hashlib
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent
SOURCE = Path('sandbox-internal/src/protected_files.rs')
BASE = '139f4db6990f67c1ed9cd28c4ce28af707d469f9'
ARCHIVE_SHA = '4ef9f178a2039ad8effd3fc97414d5b464d940fce7e6514a0d8f8e39ef14d687'
ORIGINAL_SHA = '7c3db03d1d04b89d641208776bf12e890f43299e8325cb15aa87e031a926b801'
VARIANTS = ('correct', 'bypass-writes')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare(archive, target, variant):
    if variant not in VARIANTS or target.exists():
        raise ValueError('known variant and new private source directory required')
    if sha(archive) != ARCHIVE_SHA:
        raise ValueError('original source archive differs')
    target.mkdir(parents=True, mode=0o700)
    with tarfile.open(archive) as stream:
        for member in stream.getmembers():
            name = Path(member.name)
            if name.is_absolute() or '..' in name.parts or not (member.isfile() or member.isdir()):
                raise ValueError('unexpected archive entry')
        stream.extractall(target, filter='data')
    path = target / SOURCE
    if sha(path) != ORIGINAL_SHA:
        raise ValueError('the original protected-file source differs')
    if variant == 'bypass-writes':
        before = 'metadata.mode() & 0o022 == 0'
        after = 'metadata.mode() & 0o000 == 0'
        text = path.read_text()
        if text.count(before) != 1:
            raise ValueError('original guard missing or ambiguous')
        path.write_text(text.replace(before, after))
    return {'base': BASE, 'variant': variant, 'business_source_sha256': sha(path),
            'archive_sha256': ARCHIVE_SHA}


def add_tests(source):
    path = source / SOURCE
    prefix = path.read_bytes()
    path.write_bytes(prefix + b'\n#[cfg(test)]\nmod protected_directory_tests;\n')
    destination = source / 'sandbox-internal/src/protected_files/protected_directory_tests.rs'
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes((HERE / 'native.rs').read_bytes())
    if not path.read_bytes().startswith(prefix):
        raise ValueError('test overlay changed the real source prefix')
    return {'business_prefix_sha256': hashlib.sha256(prefix).hexdigest(),
            'test_fixture_sha256': sha(destination)}
