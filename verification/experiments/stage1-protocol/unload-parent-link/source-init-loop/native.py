"""Prepare the complete row-sequence assertion and unchanged metadata/row/policy controls for ning."""
import argparse
from hashlib import sha256
import json
import os
from pathlib import Path
import subprocess
import tarfile

import extract

VARIANTS = ('original','adapted-original','skip-last-row','adapted-skip-last-row')


def prepare(output,variant):
    os.umask(0o077)
    output.mkdir(mode=0o700)
    archive = output/'source.tar'
    with archive.open('xb') as stream:
        subprocess.run(['git','archive',extract.SOURCE],cwd=extract.parent.sql.ROOT,stdout=stream,check=True)
    if sha256(archive.read_bytes()).hexdigest() != extract.ARCHIVE:
        raise ValueError('the exact original production archive is required')
    source = output/'source'
    source.mkdir(mode=0o700)
    actual_variant = variant.removeprefix('adapted-')
    if actual_variant not in extract.VARIANTS:
        raise ValueError('unknown variant')
    if variant.startswith('adapted-'):
        extract.reconstruct(archive,source,actual_variant)
        with tarfile.open(archive) as inputs:
            (source/extract.LIB).write_bytes(inputs.extractfile(str(extract.LIB)).read())
    else:
        with tarfile.open(archive) as inputs:
            inputs.extractall(source,filter='data')
        path = source/extract.CODEC
        path.write_text(extract.mutate(path.read_text(),actual_variant))
    controls_path = extract.HERE.parent/'source-policy-read/native.rs.in'
    controls = (controls_path.read_bytes()+b'\n'+(extract.HERE.parent/'source-init-row/native.rs.in').read_bytes()
        +b'\n'+(extract.HERE.parent/'source-init-metadata/native.rs.in').read_bytes())
    fixture = (extract.HERE/'native.rs.in').read_bytes()
    path = source/extract.CODEC
    isolated = path.read_bytes()
    # Preserve the fixed capture's original bytes separately. Native inputs use
    # the configured toolchain's formatter before appending the common tests.
    subprocess.run(['rustfmt','--edition','2024',str(path)],check=True,cwd=source)
    before = path.read_bytes()
    path.write_bytes(before+b'\n'+controls+b'\n'+fixture)
    record = dict(prepared=True,tests_executed=False,source=extract.SOURCE,tree=extract.TREE,
        variant=variant,archive=extract.ARCHIVE,isolated_codec=sha256(isolated).hexdigest(),
        codec_before_fixture=sha256(before).hexdigest(),
        controls=sha256(controls).hexdigest(),fixture=sha256(fixture).hexdigest(),installed=sha256(path.read_bytes()).hexdigest())
    (output/'input.json').write_text(json.dumps(record,indent=2)+'\n')
    print(variant,'complete row-sequence assertion and five original controls prepared; tests not executed')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant',choices=VARIANTS,required=True)
    cli.add_argument('--output',type=Path,required=True)
    args = cli.parse_args()
    prepare(args.output.resolve(),args.variant)
