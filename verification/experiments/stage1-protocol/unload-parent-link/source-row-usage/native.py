"""Prepare fresh exact production sources for ning's three native assertions."""
import argparse
from hashlib import sha256
import json
import os
from pathlib import Path
import subprocess
import tarfile

import extract


def prepare(output, variant):
    os.umask(0o077)
    output.mkdir(mode=0o700)
    archive = output / 'source.tar'
    with archive.open('xb') as stream:
        subprocess.run(['git', 'archive', extract.metadata.SOURCE], cwd=extract.metadata.sql.ROOT,
                       stdout=stream, check=True)
    if sha256(archive.read_bytes()).hexdigest() != extract.metadata.ARCHIVE:
        raise ValueError('the exact fixed production archive is required')
    source = output / 'source'
    source.mkdir(mode=0o700)
    with tarfile.open(archive) as tree:
        tree.extractall(source, filter='data')
    path = source / extract.CAPACITY_FILE
    original = path.read_bytes()
    changed = original.decode()
    if variant == 'drop-terminator-byte':
        changed = extract.drop_terminator_byte(changed)
    fixture = (Path(__file__).parent / 'native.rs.in').read_bytes()
    # These native cases call the original private functions. None of the
    # transparent extraction helpers or the Range adapter is installed here.
    payload = changed.encode() + b'\n' + fixture
    path.write_bytes(payload)
    record = dict(prepared=True, tests_executed=False, source=extract.metadata.SOURCE,
                  tree=extract.metadata.TREE, variant=variant, archive=extract.metadata.ARCHIVE,
                  file=str(extract.CAPACITY_FILE), original=sha256(original).hexdigest(),
                  variant_before_fixture=sha256(changed.encode()).hexdigest(),
                  fixture=sha256(fixture).hexdigest(), installed=sha256(payload).hexdigest())
    (output / 'input.json').write_text(json.dumps(record, indent=2) + '\n')
    print(variant, 'native source prepared; tests not executed')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--output', type=Path, required=True)
    cli.add_argument('--variant', choices=extract.VARIANTS, required=True)
    args = cli.parse_args()
    prepare(args.output.resolve(), args.variant)
