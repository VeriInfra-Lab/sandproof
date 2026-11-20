"""Stage exact production sources and the shared complete reservation assertions."""
import argparse
from hashlib import sha256
import json
import os
from pathlib import Path
import subprocess

import extract


def prepare(output, variant):
    os.umask(0o077)
    output.mkdir(mode=0o700)
    archive = output / 'source.tar'
    with archive.open('xb') as stream:
        subprocess.run(['git', 'archive', extract.SOURCE], cwd=extract.sql.ROOT, stdout=stream, check=True)
    if sha256(archive.read_bytes()).hexdigest() != extract.ARCHIVE:
        raise ValueError('the exact fixed source archive is required')
    source = output / 'source'
    source.mkdir(mode=0o700)
    before = extract.reconstruct(archive, source, variant)
    fixture = (extract.HERE / 'native.rs.in').read_bytes()
    installed = before + b'\n' + fixture
    (source / extract.CONFIG).write_bytes(installed)
    record = dict(prepared=True, tests_executed=False, source=extract.SOURCE, tree=extract.TREE,
                  variant=variant, archive=extract.ARCHIVE, source_file=str(extract.CONFIG),
                  before_fixture=sha256(before).hexdigest(), fixture=sha256(fixture).hexdigest(),
                  installed=sha256(installed).hexdigest())
    (output / 'input.json').write_text(json.dumps(record, indent=2) + '\n')
    print(variant, 'original reservation assertions prepared; tests not executed')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--output', type=Path, required=True)
    cli.add_argument('--variant', choices=extract.VARIANTS, required=True)
    args = cli.parse_args()
    prepare(args.output.resolve(), args.variant)
