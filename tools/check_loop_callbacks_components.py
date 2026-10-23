"""Original Store query tests in private committed source copies on ning.

Every wrong version freshly compiles and must fail the correct version's same
semantic assertion. No production source, shared cache or ordinary test runs
on the proof machine. Original test binaries/logs remain in private evidence.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import time

try:
    from .check_owned_message_components import ROOT, run, artifact, sha
except ImportError:
    from check_owned_message_components import ROOT, run, artifact, sha


SOURCE = 'controller-core/src/store.rs'
APPENDIX = 'verification/experiments/loop-callbacks/query-tests.rs'


def mutate(source, variant):
    if variant == 'skip-first':
        lower = source.index('    pub fn get(&self')
        upper = source.index('    pub fn contains_key', lower)
        original = source[lower:upper]
        if original.count('let mut index = 0;') != 1:
            raise ValueError('Ambiguous original shared query loop')
        changed = original.replace('let mut index = 0;', 'let mut index = 1;')
        return source[:lower]+changed+source[upper:], dict(before=original, after=changed)
    if variant == 'invert-presence':
        before = '        self.get(key).is_some()'
        after = '        self.get(key).is_none()'
        if source.count(before) != 1:
            raise ValueError('Ambiguous original contains_key call')
        return source.replace(before, after), dict(before=before, after=after)
    if variant != 'correct':
        raise ValueError('Unknown query variant')
    return source, None


def reproduce(output):
    from check_loop_callbacks_contract import check
    contract = check()
    if subprocess.check_output(['rustc','--version'],text=True).strip()!=contract['production_compiler']:
        raise ValueError('Native compiler differs from the fixed query contract')
    for name in contract['inputs']+['verification/loop-callbacks-inputs.sha256']:
        if subprocess.check_output(['git','show','HEAD:'+name],cwd=ROOT) != (ROOT/name).read_bytes():
            raise ValueError('Commit frozen query inputs before reproduction: '+name)
    os.umask(0o077)
    output.mkdir(parents=True,exist_ok=False)
    started = time.monotonic()
    archive = subprocess.check_output(['git','archive','HEAD'],cwd=ROOT)
    target = Path(os.environ.get('CARGO_TARGET_DIR',str(output/'target'))).resolve()
    env = os.environ | {'CARGO_TARGET_DIR':str(target),'RUST_TEST_THREADS':'1'}
    receipt = dict(kind='original-safe-shared-query-native-component',
        commit=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
        rustc=subprocess.check_output(['rustc','-vV'],text=True),variants={})
    for variant in ('correct','skip-first','invert-presence'):
        directory = output/variant
        directory.mkdir()
        with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
            tree.extractall(directory,filter='data')
        path = directory/SOURCE
        if path.read_bytes() != (ROOT/SOURCE).read_bytes():
            raise ValueError('Original Store differs from frozen source')
        original_sha = sha(path)
        text, mutation = mutate(path.read_text(),variant)
        if variant!='correct' and hashlib.sha256(text.encode()).hexdigest()!=contract['wrong_variants'][variant]['mutated_source_sha256']:
            raise ValueError('Native wrong version is not the same frozen Rust mutation')
        path.write_text(text+'\n'+(ROOT/APPENDIX).read_text())
        log = output/(variant+'-build.log')
        code = run(['cargo','test','--manifest-path','controller-core/Cargo.toml',
            '-p','sandbox-controller-core','--lib','--locked','--offline','--no-run',
            '--message-format=json'],directory,env,log)
        if code:
            raise ValueError('Compile failure is not a semantic rejection: '+str(log))
        built = artifact(log,'sandbox_controller_core')
        if built['fresh'] or not Path(built['executable']).resolve().is_relative_to(target):
            raise ValueError('Expected a fresh original test executable inside the registered target')
        binary = output/(variant+'-tests')
        shutil.copy2(built['executable'],binary)
        marker = ('loop_callbacks_query_contract' if variant=='correct' else
                  'duplicate_key_returns_original_first_matching_row' if variant=='skip-first' else
                  'contains_key_agrees_with_original_lookup_presence')
        log = output/(variant+'.log')
        code = run([str(binary),marker,'--nocapture','--test-threads=1'],directory,env,log)
        text = log.read_text()
        if variant=='correct':
            if code or 'test result: ok. 6 passed; 0 failed; 0 ignored;' not in text:
                raise ValueError('Missing six original shared-query checks')
        else:
            message = ('lookup must return original first matching row' if variant=='skip-first'
                       else 'contains_key must agree with original lookup presence')
            if (code!=101 or 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' not in text
                    or message not in text):
                raise ValueError('Wrong Rust did not fail the same semantic query assertion')
        receipt['variants'][variant] = dict(fresh=True,binary_sha256=sha(binary),
            original_sha256=original_sha,tested_sha256=sha(path),appendix_sha256=sha(ROOT/APPENDIX),
            mutation=mutation,filter=marker,test_exit_code=code,log_sha256=sha(log))
        if variant=='correct':
            delegate_checks = []
            manifest = 'verification/extract-loop-callbacks/Cargo.toml'
            commands = [
                ['cargo','fmt','--manifest-path',manifest,'--','--check'],
                ['cargo','test','--manifest-path',manifest,'--locked','--offline'],
                ['cargo','clippy','--manifest-path',manifest,'--all-targets',
                 '--locked','--offline','--','-D','warnings'],
            ]
            for index,command in enumerate(commands):
                log = output/f'delegate-{index}.log'
                code = run(command,directory,env,log)
                if code:
                    raise ValueError('Delegate crate check failed: '+str(log))
                delegate_checks.append(dict(command=command,exit_code=code,log_sha256=sha(log)))
            receipt['delegate_checks'] = delegate_checks
            log = output/'clippy-fixture.log'
            code = run(['cargo','clippy','--manifest-path','controller-core/Cargo.toml',
                '-p','sandbox-controller-core','--all-targets','--locked','--offline',
                '--','-D','warnings'],directory,env,log)
            if code:
                raise ValueError('Original shared-query fixture failed strict Clippy')
            receipt['clippy'] = dict(exit_code=code,log_sha256=sha(log))
    receipt.update(complete=True,seconds=round(time.monotonic()-started,3))
    (output/'result.json').write_text(json.dumps(receipt,indent=2)+'\n')
    print(json.dumps(dict(complete=True,correct_cases=6,wrong_variants=2,seconds=receipt['seconds'])))


if __name__=='__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    reproduce(parser.parse_args().output.resolve())
