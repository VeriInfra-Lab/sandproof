"""Real TLS source/candidate regressions on the ordinary test machine only."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]
REL = Path('verification/experiments/tls-certificate-binding')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != 'linux' or not os.environ.get('CARGO_TARGET_DIR'):
        raise ValueError('use the registered ordinary Linux environment and a private Cargo target')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False, mode=0o700)
    started = time.monotonic()
    record = {'kind': 'native-source-connection-not-machine-refinement', 'complete': False,
              'steps': [], 'variants': {}, 'new_theorems': 0}
    env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_INCREMENTAL='0', RUST_TEST_THREADS='1')

    def save():
        record['seconds'] = time.monotonic()-started
        (output/'result.json').write_text(json.dumps(record, indent=2)+'\n')

    def run(label, command, source=ROOT, timeout=900, expected=0):
        log = output/(label+'.log')
        start = time.monotonic()
        with log.open('xb') as stream:
            code = subprocess.run(command, cwd=source, env=env, stdout=stream,
                                  stderr=subprocess.STDOUT, timeout=timeout).returncode
        record['steps'].append({'label': label, 'command': command, 'exit_code': code,
                                'seconds': time.monotonic()-start, 'log_sha256': sha(log)})
        save()
        if code != expected:
            raise ValueError(label+': expected '+str(expected)+', got '+str(code))
        return log

    def artifacts(log, source, target, require_library=False):
        rows = []
        library = []
        manifest = (source/'sandbox-internal/Cargo.toml').resolve()
        library_source = (source/'sandbox-internal/src/lib.rs').resolve()
        for line in log.read_text().splitlines():
            try:
                data = json.loads(line)
            except ValueError:
                continue
            if data.get('reason') != 'compiler-artifact':
                continue
            if data.get('target', {}).get('name') in {'sandproof_internal', target}:
                if Path(data['manifest_path']).resolve() != manifest:
                    raise ValueError('artifact manifest is not this original source package')
                if not data['package_id'].startswith('path+file://'+str(manifest.parent)):
                    raise ValueError('artifact package_id does not bind this source')
            if data.get('target', {}).get('name') == target and data.get('executable'):
                rows.append(data)
            if (data.get('target', {}).get('name') == 'sandproof_internal'
                    and data['target']['kind'] == ['lib'] and not data.get('executable')):
                library.append(data)
        if len(rows) != 1 or rows[0]['fresh'] is not False:
            raise ValueError('expected exactly one newly compiled test binary')
        expected_source = library_source if target == 'sandproof_internal' else source/'sandbox-internal/tests/secure_probe.rs'
        if Path(rows[0]['target']['src_path']).resolve() != expected_source.resolve():
            raise ValueError('test target source differs')
        binary = Path(rows[0]['executable'])
        saved = output/(source.name+'-'+target+'-tests')
        shutil.copy2(binary, saved)
        result = {'binary_sha256': sha(saved), 'fresh': False,
                  'package_id': rows[0]['package_id'], 'manifest_path': rows[0]['manifest_path'],
                  'src_path': rows[0]['target']['src_path']}
        if require_library:
            if len(library) != 1 or library[0]['fresh'] is not False:
                raise ValueError('the integration test business library must also be newly compiled')
            if Path(library[0]['target']['src_path']).resolve() != library_source:
                raise ValueError('business library source differs')
            rlib = Path(next(name for name in library[0]['filenames'] if name.endswith('.rlib')))
            copied = output/(source.name+'-library.rlib')
            shutil.copy2(rlib, copied)
            result['library'] = {'fresh': False, 'sha256': sha(copied),
                                 'package_id': library[0]['package_id'],
                                 'manifest_path': library[0]['manifest_path'],
                                 'src_path': library[0]['target']['src_path']}
        return saved, result

    try:
        sys.path.insert(0, str(ROOT))
        from tools.check_tls_certificate_binding import check
        record['admission'] = check(ROOT)
        audit = json.loads((ROOT/REL/'primitive-audit.json').read_text())
        registry = Path(os.environ['CARGO_HOME'])/'registry'
        import tomllib
        packages = tomllib.loads((ROOT/'Cargo.lock').read_text())['package']
        locked = {p['name']+'-'+p['version']: p.get('checksum') for p in packages}
        record['library_sources'] = {}
        for name, digest in audit['library_sources'].items():
            package, relative = name.split('/', 1)
            candidates = list((registry/'src').glob('*/'+name))
            archives = list((registry/'cache').glob('*/'+package+'.crate'))
            if len(candidates) != 1 or len(archives) != 1 or sha(candidates[0]) != digest:
                raise ValueError('primitive library source differs: '+name)
            if sha(archives[0]) != locked[package]:
                raise ValueError('primitive crate archive checksum differs: '+package)
            checksums = json.loads((candidates[0].parents[len(Path(relative).parts)-1]/'.cargo-checksum.json').read_text())
            if checksums['files'].get(relative) != digest:
                raise ValueError('primitive source checksum differs from crate manifest: '+name)
            record['library_sources'][name] = digest
        record['head'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        record['rustc'] = subprocess.check_output(['rustc', '--version'], env=env, text=True).strip()
        record['python'] = sys.version
        if record['rustc'] != 'rustc 1.97.0 (2d8144b78 2026-07-07)':
            raise ValueError('ordinary compiler differs from the fixed production version')
        run('admission-tests', ['bash', 'scripts/python-entry.sh', '-m', 'unittest',
                                'tools.tests.test_tls_certificate_binding'])
        archive = subprocess.check_output(['git', 'archive', '--format=tar', 'HEAD'], cwd=ROOT)
        record['source_archive_sha256'] = hashlib.sha256(archive).hexdigest()
        (output/'source.tar').write_bytes(archive)
        support = (ROOT/'sandbox-internal/tests/support/tls.rs').read_text()
        support = support[support.index('use rcgen'):]
        before = 'use sandproof_internal::{CertificatePin, TlsIdentity};'
        if support.count(before) != 1:
            raise ValueError('frozen existing certificate fixture import changed')
        support = support.replace(before, 'use crate::{CertificatePin, TlsIdentity};')
        appendix = (ROOT/REL/'native.rs').read_text()
        appendix = appendix.replace('// @CERTIFICATE_FIXTURE@',
                                     '#[allow(dead_code)]\nmod fixtures {\n'+support+'\n}')
        appendix_file = output/'native-generated.rs'
        appendix_file.write_text(appendix)
        run('format-fixture', ['rustfmt', '--edition', '2024', str(appendix_file)])
        record['native_generated_sha256'] = sha(appendix_file)
        appendix = appendix_file.read_text()
        for variant in ['original', 'owned-candidate', 'wrong-first-leaf', 'wrong-return-leaf']:
            source = output/variant
            source.mkdir(mode=0o700)
            with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
                tar.extractall(source, filter='data')
            path = source/'sandbox-internal/src/tls.rs'
            original = path.read_text()
            if variant == 'owned-candidate':
                run(variant+'-patch', ['patch', '--batch', '--forward', '-p1', '--input',
                    str(source/'verification/fixtures/tls-pin-candidate.patch')], source)
            elif variant == 'wrong-first-leaf':
                start = original.index('\nfn verify_pin(')
                part = original[start:]
                if part.count('chain.first()') != 1:
                    raise ValueError('original first-leaf guard source changed')
                path.write_text(original[:start]+part.replace('chain.first()', 'chain.last()'))
            elif variant == 'wrong-return-leaf':
                start = original.index('    pub(crate) fn peer_pin(')
                end = original.index('\n#[derive(Clone)]\npub struct ClientTls', start)
                part = original[start:end]
                if part.count('chain.first()') != 1:
                    raise ValueError('original returned leaf source changed')
                path.write_text(original[:start]+part.replace('chain.first()', 'chain.last()')+original[end:])
            prefix = path.read_text()
            path.write_text(prefix+'\n'+appendix)
            if path.read_text().split('\n#[cfg(test)]\nmod tls_certificate_binding_tests', 1)[0].rstrip() != prefix.rstrip():
                raise ValueError('test overlay changed a business statement')
            data = record['variants'][variant] = {'business_prefix_sha256': hashlib.sha256(prefix.encode()).hexdigest()}
            run(variant+'-fmt', ['cargo', 'fmt', '--all', '--check'], source)
            run(variant+'-clean-package', ['cargo', 'clean', '-p', 'sandproof-internal'], source)
            log = run(variant+'-build', ['cargo', 'test', '--locked', '--offline', '-p', 'sandproof-internal',
                '--lib', '--no-run', '--message-format=json'], source)
            binary, data['unit_artifact'] = artifacts(log, source, 'sandproof_internal')
            if variant in {'original', 'owned-candidate'}:
                log = run(variant+'-tests', [str(binary), 'tls_certificate_binding_tests', '--nocapture'], source)
                if 'test result: ok. 7 passed; 0 failed; 0 ignored;' not in log.read_text():
                    raise ValueError('expected every new direct original/candidate case')
                data['new_unit_passed'] = 7
                run(variant+'-probe-clean-package', ['cargo', 'clean', '-p', 'sandproof-internal'], source)
                log = run(variant+'-probe-build', ['cargo', 'test', '--locked', '--offline', '-p',
                    'sandproof-internal', '--test', 'secure_probe', '--no-run', '--message-format=json'], source)
                probe, data['tls_artifact'] = artifacts(log, source, 'secure_probe', require_library=True)
                log = run(variant+'-tls', [str(probe), '--nocapture', '--test-threads=1'], source)
                if 'test result: ok. 16 passed; 0 failed; 0 ignored;' not in log.read_text():
                    raise ValueError('actual original TLS loopback cases missing')
                data['tls_passed'] = 16
                run(variant+'-clippy', ['cargo', 'clippy', '--locked', '--offline', '-p',
                    'sandproof-internal', '--all-targets', '--', '-D', 'warnings'], source)
            else:
                name, message = (
                    ('a_registered_tail_cannot_authorize_the_original_first_leaf',
                     'a registered tail must not authorize the original first leaf')
                    if variant == 'wrong-first-leaf' else
                    ('peer_pin_returns_the_authorized_original_first_leaf',
                     'peer_pin must return the authorized original first leaf'))
                log = run(variant+'-native-rejection', [str(binary),
                    'tls::tls_certificate_binding_tests::'+name, '--exact', '--nocapture'], source, expected=101)
                text = log.read_text()
                if message not in text or 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' not in text:
                    raise ValueError('wrong version failed for another reason')
                data['negative'] = 'actual Rust native assertion only; no Lean rejection claimed'
            save()
        run('remove-last-mutant-package-cache', ['cargo', 'clean', '-p', 'sandproof-internal'], source)
        record.update(complete=True, exit_code=0)
    except BaseException as error:
        record.update(exit_code=1, error=repr(error), traceback=traceback.format_exc())
    save()
    print(json.dumps({k:v for k,v in record.items() if k not in ['steps','variants']}))
    return record['exit_code']


if __name__ == '__main__':
    raise SystemExit(main())
