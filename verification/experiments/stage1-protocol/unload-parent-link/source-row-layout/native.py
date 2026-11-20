"""Stage original ready/prepare fixtures and actual source-row layout assertions."""
import argparse
from hashlib import sha256
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'source-row-usage'))
_spec = importlib.util.spec_from_file_location('source_row_usage_capture', HERE.parent / 'source-row-usage/extract.py')
usage = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(usage)


def prepare(output, variant):
    os.umask(0o077)
    output.mkdir(mode=0o700)
    archive = output / 'source.tar'
    with archive.open('xb') as stream:
        subprocess.run(['git', 'archive', usage.metadata.SOURCE], cwd=usage.metadata.sql.ROOT,
                       stdout=stream, check=True)
    if sha256(archive.read_bytes()).hexdigest() != usage.metadata.ARCHIVE:
        raise ValueError('the exact fixed original production archive is required')
    source = output / 'source'
    source.mkdir(mode=0o700)
    with tarfile.open(archive) as tree:
        tree.extractall(source, filter='data')
    original_capacity = (source / usage.CAPACITY_FILE).read_text()
    if variant == 'drop-terminator-byte':
        (source / usage.CAPACITY_FILE).write_text(usage.drop_terminator_byte(original_capacity))
    fixtures = {
        'controller-service/src/controller/capacity.rs': 'native.rs.in',
        'controller-service/src/controller/lifecycle_inbox/source_work.rs': 'forward.rs.in',
        'controller-service/src/controller/lifecycle_inbox/source_work/tests.rs': 'fixture.rs.in',
    }
    installed = {}
    for name, fixture in fixtures.items():
        path = source / name
        payload = (HERE / fixture).read_bytes()
        original = path.read_bytes()
        path.write_bytes(original + b'\n' + payload)
        installed[name] = dict(before_fixture=sha256(original).hexdigest(), fixture=sha256(payload).hexdigest(),
                               installed=sha256(path.read_bytes()).hexdigest())
    record = dict(prepared=True, tests_executed=False, source=usage.metadata.SOURCE,
                  tree=usage.metadata.TREE, variant=variant, archive=usage.metadata.ARCHIVE,
                  original_capacity=sha256(original_capacity.encode()).hexdigest(), files=installed)
    (output / 'input.json').write_text(json.dumps(record, indent=2) + '\n')
    print(variant, 'original row-layout fixtures prepared; tests not executed')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--output', required=True, type=Path)
    cli.add_argument('--variant', choices=usage.VARIANTS, required=True)
    args = cli.parse_args()
    prepare(args.output.resolve(), args.variant)
