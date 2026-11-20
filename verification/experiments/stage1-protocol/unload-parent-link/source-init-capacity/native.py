"""Prepare identical real SQLite assertions for original and isolated callers."""
import argparse
from hashlib import sha256
import json
import os
from pathlib import Path
import subprocess
import tarfile

import extract

VARIANTS = ('original', 'omit-row-overhead', 'adapted-original', 'adapted-omit-row-overhead')


def prepare(output, variant):
    os.umask(0o077)
    output.mkdir(mode=0o700)
    archive = output / 'source.tar'
    with archive.open('xb') as stream:
        subprocess.run(['git', 'archive', extract.SOURCE], cwd=extract.sql.ROOT, stdout=stream, check=True)
    if sha256(archive.read_bytes()).hexdigest() != extract.ARCHIVE:
        raise ValueError('the exact original production archive is required')
    source = output / 'source'
    source.mkdir(mode=0o700)
    adapted = variant.startswith('adapted-')
    actual_variant = variant.removeprefix('adapted-')
    if actual_variant not in extract.VARIANTS:
        raise ValueError('unknown variant')
    if adapted:
        extract.reconstruct(archive, source, actual_variant)
        # Charon naming metadata is never installed in the native source.
        with tarfile.open(archive) as inputs:
            (source / extract.LIB).write_bytes(inputs.extractfile(str(extract.LIB)).read())
    else:
        with tarfile.open(archive) as inputs:
            inputs.extractall(source, filter='data')
        path = source / extract.CODEC
        path.write_text(extract.mutate(path.read_text(), actual_variant))
    fixture = (extract.HERE / 'native.rs.in').read_bytes()
    codec = source / extract.CODEC
    before = codec.read_bytes()
    codec.write_bytes(before + b'\n' + fixture)
    record = dict(prepared=True, tests_executed=False, source=extract.SOURCE, tree=extract.TREE,
        variant=variant, archive=extract.ARCHIVE, codec_before_fixture=sha256(before).hexdigest(),
        fixture=sha256(fixture).hexdigest(), installed=sha256(codec.read_bytes()).hexdigest())
    (output / 'input.json').write_text(json.dumps(record, indent=2) + '\n')
    print(variant, 'original SQLite assertions prepared; tests not executed')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    cli.add_argument('--output', type=Path, required=True)
    args = cli.parse_args()
    prepare(args.output.resolve(), args.variant)
