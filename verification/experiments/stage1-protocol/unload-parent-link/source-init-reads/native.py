"""Prepare identical actual SQLite reader assertions; never execute tests."""
import argparse
from hashlib import sha256
import json
import os
from pathlib import Path
import subprocess
import tarfile

import extract

VARIANTS = ('original','adapted-original','drop-current-capacity','adapted-drop-current-capacity')


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def prepare(output,variant):
    os.umask(0o077)
    output.mkdir(mode=0o700)
    archive=output/'source.tar'
    with archive.open('xb') as stream:
        subprocess.run(['git','archive',extract.SOURCE],cwd=extract.parent.sql.ROOT,stdout=stream,check=True)
    if digest(archive)!=extract.ARCHIVE:
        raise ValueError('the exact original production archive is required')
    source=output/'source'
    source.mkdir(mode=0o700)
    actual_variant=variant.removeprefix('adapted-')
    if actual_variant not in extract.VARIANTS:
        raise ValueError('unknown variant')
    if variant.startswith('adapted-'):
        extract.reconstruct(archive,source,actual_variant)
        with tarfile.open(archive) as inputs:
            (source/extract.LIB).write_bytes(inputs.extractfile(str(extract.LIB)).read())
    else:
        with tarfile.open(archive) as inputs:
            inputs.extractall(source,filter='data')
        path=source/extract.CAPACITY
        path.write_text(extract.mutate(path.read_text(),actual_variant))
    targets=(extract.CODEC,extract.CAPACITY)
    pre_format={str(p):digest(source/p) for p in targets}
    for path in targets:
        subprocess.run(['rustfmt','--edition','2024',str(source/path)],cwd=source,check=True)
    formatted={str(p):digest(source/p) for p in targets}
    fixture=(extract.HERE/'native.rs.in').read_bytes()
    path=source/extract.CAPACITY
    path.write_bytes(path.read_bytes()+b'\n'+fixture)
    record=dict(prepared=True,tests_executed=False,source=extract.SOURCE,tree=extract.TREE,
        variant=variant,archive=extract.ARCHIVE,pre_format=pre_format,formatted=formatted,
        fixture=sha256(fixture).hexdigest(),installed={str(p):digest(source/p) for p in targets})
    (output/'input.json').write_text(json.dumps(record,indent=2)+'\n')
    print(variant,'four complete capacity-reader/first-error assertions prepared; tests not executed')


if __name__ == '__main__':
    cli=argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant',choices=VARIANTS,required=True)
    cli.add_argument('--output',type=Path,required=True)
    args=cli.parse_args()
    prepare(args.output.resolve(),args.variant)
