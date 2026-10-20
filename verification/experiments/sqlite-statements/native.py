"""Actual-library component regression for this isolated SQL caller candidate.

Run on the native machine under the existing host queue and fixed Cargo cache.
This checks a real caller and an actually compiled wrong row-count branch.
Finite regression is not an original/candidate general equivalence proof.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

HERE=Path(__file__).resolve().parent


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def materializer():
    spec=importlib.util.spec_from_file_location('sqlite_statements_materializer',HERE/'prepare.py')
    module=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def command(arguments, source, environment, log):
    with log.open('xb') as stream:
        code=subprocess.run(arguments,cwd=source,env=environment,stdout=stream,
                            stderr=subprocess.STDOUT,timeout=900).returncode
    return dict(command=arguments,exit_code=code,log_sha256=sha(log))


def run(archive, output):
    if output.exists() or output.is_symlink():
        raise ValueError('component output already exists; inspect original task instead')
    for name in ('CARGO_HOME','CARGO_TARGET_DIR'):
        if not os.environ.get(name):
            raise ValueError('use the existing coordinated fixed '+name)
    output.mkdir(parents=True,mode=0o700)
    environment=dict(os.environ,CARGO_NET_OFFLINE='true',CARGO_INCREMENTAL='0',
                     CARGO_BUILD_JOBS='4',RUST_TEST_THREADS='1')
    runtime=subprocess.check_output(['rustc','-vV'],env=environment,text=True)
    if not runtime.startswith('rustc 1.97.0 '):
        raise ValueError('unreviewed production compiler')
    start=time.monotonic()
    record=dict(kind='isolated-actual-SQL-caller-components-not-full-acceptance',
                rustc=runtime,archive_sha256=sha(archive),
                fixture_sha256=sha(HERE/'journal-tests.rs'),variants={})
    prepare=materializer()
    for variant in ('correct','accept-zero-changes','omit-owner-guard'):
        source=output/variant
        receipt=prepare.prepare_archive(archive,source)
        path=source/'controller-service/src/journal/statement_core.rs'
        original=path.read_text()
        if variant=='accept-zero-changes':
            needle='if changed != 1 {'
            if original.count(needle)!=1:
                raise ValueError('row-count mutation target is ambiguous')
            path.write_text(original.replace(needle,'if changed > 1 {'))
        elif variant=='omit-owner-guard':
            needle='WHERE sandbox_id=?3 AND owner_id=?4 AND revision=?5'
            if original.count(needle)!=1:
                raise ValueError('SQL owner mutation target is ambiguous')
            path.write_text(original.replace(needle,'WHERE sandbox_id=?3 AND revision=?5'))
        journal=source/'controller-service/src/journal.rs'
        journal.write_text(journal.read_text()+'\n'+(HERE/'journal-tests.rs').read_text())
        metadata=dict(materialization=receipt,core_sha256=sha(path),
                      tested_journal_sha256=sha(journal))
        build=command(['cargo','test','--locked','--offline','-p',
                       'sandproof-controller-service','--lib','--no-run','--message-format=json'],
                      source,environment,output/(variant+'-build.log'))
        if build['exit_code']!=0:
            raise ValueError('compile failure is not a semantic counterexample: '+variant)
        rows=[]
        for line in (output/(variant+'-build.log')).read_text().splitlines():
            if line.startswith('{'):
                row=json.loads(line)
                if (row.get('reason')=='compiler-artifact' and
                        row.get('target',{}).get('name')=='sandproof_controller_service' and
                        row.get('executable')):
                    rows.append(row)
        if len(rows)!=1 or rows[0]['fresh']:
            raise ValueError('expected one freshly compiled real test binary: '+variant)
        binary=Path(rows[0]['executable']).resolve()
        if not binary.is_relative_to(Path(environment['CARGO_TARGET_DIR']).resolve()):
            raise ValueError('compiler binary escaped coordinated target')
        kept=output/(variant+'-tests')
        shutil.copy2(binary,kept)
        selected={'correct':'sqlite_statements_trial',
                  'accept-zero-changes':'sqlite_statements_trial::wrong_revision_never_succeeds_or_changes_any_row',
                  'omit-owner-guard':'sqlite_statements_trial::wrong_owner_never_succeeds_or_changes_any_row'}[variant]
        tested=command([str(kept),selected,'--nocapture','--test-threads=1'],
                       source,environment,output/(variant+'-test.log'))
        text=(output/(variant+'-test.log')).read_text()
        if variant=='correct':
            if tested['exit_code']!=0 or 'test result: ok. 8 passed; 0 failed; 0 ignored;' not in text:
                raise ValueError('correct actual SQLite components failed')
        elif (tested['exit_code']!=101 or
              'test result: FAILED. 0 passed; 1 failed; 0 ignored;' not in text or
              ({'accept-zero-changes':'original zero-row UPDATE must be a revision conflict',
                'omit-owner-guard':'original SQL owner mismatch must reject before subsequent writes'}[variant]) not in text):
            raise ValueError('wrong Rust did not fail the unchanged original assertion')
        record['variants'][variant]=dict(source=metadata,fresh=True,
            binary_sha256=sha(kept),build=build,test=tested)
        if variant=='correct':
            clippy=command(['cargo','clippy','--locked','--offline','-p',
                            'sandproof-controller-service','--lib','--tests','--','-D','warnings'],
                           source,environment,output/'clippy.log')
            if clippy['exit_code']!=0:
                raise ValueError('strict actual-library component Clippy failed')
            record['clippy']=clippy
        (output/'progress.json').write_text(json.dumps(record,indent=2)+'\n')
    record.update(complete=True,seconds=time.monotonic()-start)
    (output/'complete.json').write_text(json.dumps(record,indent=2)+'\n')
    return record


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-archive',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    print(json.dumps(run(args.source_archive.resolve(strict=True),args.output.absolute()),indent=2))
