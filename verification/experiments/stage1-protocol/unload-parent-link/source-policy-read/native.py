"""Prepare original SQL/JSON policy reads and identical count assertions for ning."""
import argparse
from hashlib import sha256
import json
import os
from pathlib import Path
import subprocess
import tarfile

import extract

CODEC = Path('controller-service/src/controller/lifecycle_inbox/source_work/codec.rs')


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
    with tarfile.open(archive) as inputs:
        inputs.extractall(source, filter='data')
    config = source / extract.CONFIG
    if variant == 'skip-maintenance-validation':
        config.write_text(extract.skip_validation(config.read_text()))
    elif variant != 'original':
        raise ValueError('unknown variant')
    fixture = (extract.HERE / 'native.rs.in').read_bytes()
    codec = source / CODEC
    before = codec.read_bytes()
    codec.write_bytes(before + b'\n' + fixture)
    record = dict(prepared=True, tests_executed=False, source=extract.SOURCE, tree=extract.TREE,
        variant=variant, archive=extract.ARCHIVE, config_sha256=sha256(config.read_bytes()).hexdigest(),
        codec_before_fixture=sha256(before).hexdigest(), fixture=sha256(fixture).hexdigest(),
        installed=sha256(codec.read_bytes()).hexdigest())
    (output / 'input.json').write_text(json.dumps(record, indent=2) + '\n')
    print(variant, 'original SQL/JSON/count assertions prepared; tests not executed')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=extract.VARIANTS, required=True)
    cli.add_argument('--output', type=Path, required=True)
    args = cli.parse_args()
    prepare(args.output.resolve(), args.variant)
