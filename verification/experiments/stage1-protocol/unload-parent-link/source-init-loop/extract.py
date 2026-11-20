"""Capture the real original row loop with a last-row omission."""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent


def load_parent():
    directory = HERE.parent / 'source-init-capacity'
    sys.path.insert(0, str(directory))
    try:
        spec = importlib.util.spec_from_file_location('init_capacity_capture', directory / 'extract.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module
    finally:
        sys.path.remove(str(directory))


parent = load_parent()
SOURCE, TREE, ARCHIVE = parent.SOURCE, parent.TREE, parent.ARCHIVE
CODEC, LIB = parent.CODEC, parent.LIB
OPTIONS, NAMESPACE = parent.OPTIONS, parent.NAMESPACE
VARIANTS = ('original', 'skip-last-row')


def mutate(text, variant):
    raw = '    for id in 1..=rows {\n'
    isolated = '            Some(id) => match insert_initial_source_row(connection, id, empty, sha, reserve) {\n'
    if text.count(raw) + text.count(isolated) != 1:
        raise ValueError('one exact original or isolated initialization row loop is required')
    if variant == 'original':
        return text
    if variant != 'skip-last-row':
        raise ValueError('unknown variant')
    if raw in text:
        return text.replace(raw, raw + '        if id == rows { break; }\n', 1)
    return text.replace(isolated,
        '            Some(id) if id == rows => break Ok(()),\n' + isolated, 1)


def reconstruct(archive, output, variant):
    parent.reconstruct(archive, output, 'original')
    path = output / CODEC
    path.write_text(mutate(path.read_text(), variant))


def capture(output, tools, lock, variant):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or shutil.disk_usage(output.parent).free < 80 * parent.GIB):
        raise ValueError('the formal role and original capture budget are required')
    os.umask(0o077)
    os.sched_setaffinity(0, set(sorted(os.sched_getaffinity(0))[:4]))
    fixed = json.loads((parent.sql.ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    if any(parent.sql.digest(binary) != fixed[name]['binary_sha256'] for name,binary in (
            ('charon', charon), ('aeneas', aeneas))):
        raise ValueError('the fixed original tools are required')
    if subprocess.check_output(['git','rev-parse',SOURCE+'^{tree}'],cwd=parent.sql.ROOT,text=True).strip() != TREE:
        raise ValueError('the fixed production tree is required')
    output.mkdir(mode=0o700)
    report = dict(source=SOURCE,tree=TREE,variant=variant,options=OPTIONS,
        driver=parent.sql.digest(Path(__file__)),parent_driver=parent.sql.digest(Path(parent.__file__)),
        commands=[],complete=False)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git','archive',SOURCE],cwd=parent.sql.ROOT,stdout=stream,check=True)
        if parent.sql.digest(archive) != ARCHIVE:
            raise ValueError('the exact original production archive is required')
        source = output / 'source'
        source.mkdir(mode=0o700)
        reconstruct(archive,source,variant)
        report['staged'] = {str(path):parent.sql.digest(source/path) for path in (CODEC,LIB)}
        selectors = [part for flag,group in (('--start-from','roots'),('--start-from-if-exists','retained_roots'),
                    ('--include','include'),('--opaque','opaque')) for pattern in OPTIONS[group] for part in (flag,pattern)]
        ir,generated = output/'actual.llbc',output/'generated'
        env = dict(os.environ,CARGO_NET_OFFLINE='true',CARGO_BUILD_JOBS='4',CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(output/'target'))
        for name in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS'):
            env.pop(name,None)
        commands = [
            ('charon',[charon,'cargo','--preset=aeneas','--error-on-warnings','--sysroot','default',
                *selectors,'--dest-file',ir,'--','--manifest-path','controller-service/Cargo.toml',
                '--lib','--locked','--offline','--target',fixed['target']],240),
            ('aeneas',[aeneas,'-backend','lean','-abort-on-error','-warnings-as-errors','-split-files',
                '-emit-json','-namespace',NAMESPACE,'-subdir',NAMESPACE,'-dest',generated,ir],120),
        ]
        with lock.open('r+') as guard:
            fcntl.flock(guard,fcntl.LOCK_EX|fcntl.LOCK_NB)
            for name,argv,seconds in commands:
                reply = parent.run(name,argv,seconds,source,output,env)
                report['commands'].append(reply)
                if reply['exit_code'] or reply['bounded_stop']:
                    raise ValueError('the complete fixed '+name+' result is required')
        report['ir'] = parent.sql.digest(ir)
        report['generated'] = {str(p.relative_to(generated)):parent.sql.digest(p)
            for p in sorted(generated.rglob('*')) if p.is_file()}
        report['admission'] = parent.admit(ir,generated,fixed['target'])
        report['complete'] = True
    finally:
        (output/'result.json').write_text(json.dumps(report,indent=2)+'\n')
    print(variant,'complete actual initialization row-loop capture')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant',choices=VARIANTS,required=True)
    for name in ('output','tools','lock'):
        cli.add_argument('--'+name,type=Path,required=True)
    args = cli.parse_args()
    capture(args.output.resolve(),args.tools.resolve(strict=True),args.lock.resolve(strict=True),args.variant)
