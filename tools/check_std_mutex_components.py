"""Original standard Mutex/SerialGuard regressions in private source copies.

Run on the native machine only. Mutants must freshly compile and then fail
one of the correct version's exact semantic assertions. No timing sleeps,
production edits, compiler failures or timeouts count as a counterexample.
"""
import argparse
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import time
from check_owned_message_components import ROOT, run, artifact, sha

def reproduce(output):
    from check_std_mutex_contract import check
    contract = check()
    for name in contract['inputs'] + ['verification/std-mutex-inputs.sha256']:
        committed = subprocess.check_output(['git', 'show', 'HEAD:' + name], cwd=ROOT)
        if committed != (ROOT / name).read_bytes():
            raise ValueError('Commit frozen standard Mutex inputs before reproduction: ' + name)
    os.umask(0o077)
    output.mkdir(parents=True, exist_ok=False)
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    archive = subprocess.check_output(['git', 'archive', 'HEAD'], cwd=ROOT)
    target = Path(os.environ.get('CARGO_TARGET_DIR', str(output / 'target'))).resolve()
    env = os.environ | {'CARGO_TARGET_DIR': str(target), 'RUST_TEST_THREADS': '1'}
    start = time.monotonic()
    record = dict(kind='conditional-model-and-original-source-component', head=revision, contract_id=contract['contract_id'], variants={}, rustc=subprocess.check_output(['rustc', '-vV'], text=True))

    def build(directory, package, name, features, tag):
        log = output / (tag + '-build.log')
        cmd = ['cargo', 'test', '--locked', '--offline', '-p', package, '--lib', *features, '--no-run', '--message-format=json']
        code = run(cmd, directory, env, log)
        if code:
            raise RuntimeError(('compile failure is not a semantic rejection', tag, code))
        built = artifact(log, name)
        assert built['fresh'] is False, (tag, 'not fresh')
        assert Path(built['executable']).resolve().is_relative_to(target), (tag, 'binary escaped registered target')
        binary = output / (tag + '-tests')
        shutil.copy2(built['executable'], binary)
        return (binary, dict(sha256=sha(binary), fresh=True, build_log_sha256=sha(log)))
    for variant in ('correct', 'late-unlock', 'swallow-poison'):
        d = output / variant
        d.mkdir()
        with tarfile.open(fileobj=io.BytesIO(archive)) as t:
            t.extractall(d, filter='data')
        edits = {}
        for (source, appendix) in [('controller-service/src/application/diagnostics.rs', 'diagnostics-tests.rs'), ('sandbox-guest-service/src/task_accounting.rs', 'meter-tests.rs'), ('sandbox-guest-runtime/src/pty.rs', 'pty-tests.rs')]:
            path = d / source
            assert path.read_bytes() == (ROOT / source).read_bytes()
            text = path.read_text()
            data = dict(production_sha256=sha(path), appendix_sha256=sha(ROOT / 'verification/experiments/std-mutex' / appendix))
            if variant == 'late-unlock' and source.startswith('controller-service/'):
                lo = text.index('impl<T> Drop for SerialGuard')
                hi = text.index('\n#[cfg(test)]', lo)
                old = text[lo:hi]
                line = '        drop(self.guard.take());\n'
                assert old.count(line) == 1
                new = old.replace(line, '')
                suffix = '\n    }\n}'
                (left, right) = new.rsplit(suffix, 1)
                new = left + '\n        drop(self.guard.take());' + suffix + right
                text = text[:lo] + new + text[hi:]
                data['mutation'] = dict(before=old, after=new)
            if variant == 'swallow-poison' and source.startswith('controller-service/'):
                before = '        self.0.get_mut()\n'
                after = '        match self.0.get_mut() {\n            Ok(value) => Ok(value),\n            Err(error) => Ok(error.into_inner()),\n        }\n'
                assert text.count(before) == 1
                text = text.replace(before, after)
                data['mutation'] = dict(before=before, after=after)
            text += '\n' + (ROOT / 'verification/experiments/std-mutex' / appendix).read_text()
            path.write_text(text)
            data['tested_sha256'] = sha(path)
            edits[source] = data
        v = dict(sources=edits, checks=[], binaries={})
        record['variants'][variant] = v
        entries = [('sandproof-controller-service', 'sandproof_controller_service', ['--features', 'contention-diagnostics'], 'std_mutex_contract_trial', 9)]
        if variant == 'correct':
            entries += [('sandproof-guest-service', 'sandproof_guest_service', [], 'std_mutex_meter_trial', 1), ('sandproof-guest-runtime', 'sandproof_guest_runtime', [], 'std_mutex_pty_trial', 1)]
        for (package, name, features, marker, count) in entries:
            tag = variant + '-' + name
            (binary, meta) = build(d, package, name, features, tag)
            v['binaries'][name] = meta
            if variant != 'correct':
                marker = 'original_guard_released_before_diagnostic_failure' if variant == 'late-unlock' else 'original_unique_get_mut_keeps_the_borrow_and_existing_poison'
                count = 1
            log = output / (tag + '.log')
            code = run([str(binary), marker, '--nocapture', '--test-threads=1'], d, env, log)
            text = log.read_text()
            if variant == 'correct':
                assert code == 0 and f'test result: ok. {count} passed; 0 failed; 0 ignored;' in text, (tag, code)
            else:
                assert code == 101 and 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' in text and (('original mutex not released cleanly before diagnostic failure' if variant == 'late-unlock' else 'original poisoned mutable borrow was silently accepted') in text), (tag, code)
            v['checks'].append(dict(filter=marker, count=count, test_exit=code, log_sha256=sha(log)))
            (output / 'progress.json').write_text(json.dumps(record, indent=2))
        if variant == 'correct':
            for (package, _, features, _, _) in entries:
                log = output / ('clippy-' + package + '.log')
                code = run(['cargo', 'clippy', '--locked', '--offline', '-p', package, *features, '--all-targets', '--', '-D', 'warnings'], d, env, log)
                assert code == 0, (package, 'clippy', code)
                record.setdefault('clippy', []).append(dict(package=package, exit_code=code, log_sha256=sha(log)))
    record.update(complete=True, seconds=round(time.monotonic() - start, 3))
    (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps(dict(complete=True, seconds=record['seconds'], correct=11, wrong=2)))
if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    reproduce(parser.parse_args().output.resolve())
