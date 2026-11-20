"""Stage original SQL policy reads and the same record-count assertions."""
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
        subprocess.run(['git', 'archive', extract.SOURCE], cwd=extract.bounds.sql.ROOT, stdout=stream, check=True)
    if sha256(archive.read_bytes()).hexdigest() != extract.ARCHIVE:
        raise ValueError('the exact original production archive is required')
    source = output / 'source'
    source.mkdir(mode=0o700)
    with tarfile.open(archive) as inputs:
        inputs.extractall(source, filter='data')
    codec = source / extract.CODEC
    before = extract.mutate(codec.read_text(), variant).encode()
    fixture = (extract.HERE / 'native.rs.in').read_bytes()
    installed = before + b'\n' + fixture
    codec.write_bytes(installed)
    record = dict(prepared=True, tests_executed=False, source=extract.SOURCE, tree=extract.TREE,
                  variant=variant, archive=extract.ARCHIVE, source_file=str(extract.CODEC),
                  before_fixture=sha256(before).hexdigest(), fixture=sha256(fixture).hexdigest(),
                  installed=sha256(installed).hexdigest())
    (output / 'input.json').write_text(json.dumps(record, indent=2) + '\n')
    print(variant, 'original SQL/count assertions prepared; tests not executed')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=extract.VARIANTS, required=True)
    cli.add_argument('--output', type=Path, required=True)
    args = cli.parse_args()
    prepare(args.output.resolve(), args.variant)
