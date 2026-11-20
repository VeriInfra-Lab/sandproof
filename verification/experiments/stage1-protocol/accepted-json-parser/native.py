"""Same twelve real-parser assertions in correct/wrong/restored development views.

Local development is authorized by the user; ning remains the acceptance
provider. Native assertion rejection is not a Lean Rust counterexample.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import traceback

import capture

ROOT, HERE = capture.ROOT, capture.HERE
sha = capture.sha
TESTS = {
    'earlier::' + name for name in (
        'buffered_operation_leaf_stays_unpositioned_without_an_outer_json_container',
        'nearest_real_container_positions_once_through_both_envelopes',
        'operation_buffering_can_reject_syntax_before_a_custom_leaf_runs',
        'expected_session_tag_content_order_selects_the_actual_reader',
        'content_error_beats_later_container_and_document_errors',
        'same_original_schema_rejects_unknown_duplicate_and_missing_members',
        'successful_values_accept_both_orders_and_sequences_before_document_end')
} | {'fields::normal_' + variant for variant in ('create', 'unload', 'resume', 'renew', 'destroy')}
FAILURES = {'correct': set(), 'swap-resource-keys': {'fields::normal_create'},
            'swap-resume-keys': {'fields::normal_resume'}, 'restored': set()}


def run(output):
    output.mkdir(parents=True, mode=0o700)
    record = dict(kind='current-Accepted-parser-native-development-not-ning-acceptance',
                  development_complete=False, ning_acceptance=False, shared_acceptance=False,
                  full_v3=False, Lean_Rust_counterexamples=False, commands=[], cases={})
    started = time.monotonic()
    try:
        capture.source.require(1 <= len(os.sched_getaffinity(0)) <= 4, 'native CPU allocation differs')
        record['source_head'] = capture.guard_baseline()
        record['source_base'] = capture.BASE
        inputs = capture.base.snapshot(ROOT)
        modes = {name: capture.mode(ROOT / name) for name in inputs}
        registry = capture.registry.registry()
        library = capture.module('accepted_native_fixed_parser_library', HERE.parent / 'parser-binding/library.py')
        library.MANIFESTS['native_parser'] = str((HERE / 'Cargo.toml').relative_to(ROOT))
        library_inputs = library.capture(ROOT)
        reviewed = json.loads((HERE.parent / 'parser-binding/candidate.json').read_text())['library_inputs']
        capture.source.require(library_inputs == reviewed, 'actual parser source/features differ from reviewed use')
        record.update(inputs=inputs, input_modes=modes, registry=registry,
                      library_inputs=library_inputs, expected_tests=sorted(TESTS), tools={})
        for name in ('rustc', 'cargo', 'clippy-driver', 'rustfmt'):
            path = Path(subprocess.check_output(['rustup', 'which', name], text=True).strip())
            record['tools'][name] = {'path': str(path), 'sha256': sha(path)}
        project = output / 'source'
        project.mkdir(mode=0o700)
        for name, wanted in inputs.items():
            path = project / name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, path)
            capture.source.require(sha(path) == wanted, 'native source changed while copying')
        restorations = []
        manifest = str((HERE / 'Cargo.toml').relative_to(ROOT))
        for case, failures in FAILURES.items():
            folder = output / case
            folder.mkdir(mode=0o700)
            target = folder / 'target'
            capture.source.require(not target.exists(), 'variant target was already populated')
            expected = dict(inputs)
            mutation = capture.MUTATIONS.get(case)
            if mutation:
                name, before, after = mutation
                path = project / name
                text = path.read_text()
                capture.source.require(text.count(before) == 1, 'native mutation site changed')
                path.write_text(text.replace(before, after))
                expected[name] = sha(path)
            env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
                       CARGO_TARGET_DIR=str(target), CARGO_TERM_COLOR='never', PYTHONDONTWRITEBYTECODE='1')
            case_record = {'source_inputs': expected, 'fresh_target_initially_absent': True}
            for label, argv in (
                ('test', ['cargo', 'test', '--manifest-path', manifest, '--locked', '--offline',
                          '--lib', '--message-format=json', '--', '--test-threads=1']),
                ('clippy', ['cargo', 'clippy', '--manifest-path', manifest, '--locked', '--offline',
                            '--all-targets', '--', '-D', 'warnings']),
                ('fmt', ['cargo', 'fmt', '--manifest-path', manifest, '--', '--check']),
            ):
                log = folder / (label + '.log')
                begin = time.monotonic()
                with log.open('xb') as stream:
                    result = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                            stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
                row = dict(case=case, label=label, argv=argv, exit_code=result.returncode,
                           seconds=time.monotonic()-begin, log=str(log), log_sha256=sha(log))
                record['commands'].append(row)
                if label != 'test':
                    capture.source.require(result.returncode == 0, 'native ' + label + ' failed')
                    continue
                text = log.read_text()
                outcomes = re.findall(r'^test ([\w:]+) \.\.\. (ok|FAILED)$', text, re.M)
                capture.source.require(len(outcomes) == len(TESTS) and
                                       {name for name, _ in outcomes} == TESTS and
                                       {name for name, status in outcomes if status == 'FAILED'} == failures,
                                       'native full assertion set or exact failures differ')
                capture.source.require(result.returncode == (101 if failures else 0) and
                                       f'{len(TESTS)-len(failures)} passed; {len(failures)} failed; 0 ignored;' in text,
                                       'native process did not reach the expected assertions')
                if failures:
                    capture.source.require('assertion `left == right` failed' in text,
                                           'wrong parser did not reach complete value equality')
                artifacts = []
                for line in text.splitlines():
                    try:
                        message = json.loads(line)
                    except ValueError:
                        continue
                    if message.get('reason') == 'compiler-artifact' and message['target']['name'] in (
                            'sandproof_protocol', 'sandproof_accepted_parser_check'):
                        artifacts.append(message)
                capture.source.require(len(artifacts) == 2 and not any(a['fresh'] for a in artifacts),
                                       'actual protocol/test artifacts were not freshly compiled')
                for artifact in artifacts:
                    wanted = project / ('sandbox-protocol/Cargo.toml' if artifact['target']['name'] ==
                                       'sandproof_protocol' else manifest)
                    capture.source.require(Path(artifact['manifest_path']) == wanted,
                                           'native artifact comes from a different source manifest')
                compiled_protocol = next(a for a in artifacts if a['target']['name'] == 'sandproof_protocol')
                protocol_libraries = list(target.glob('debug/deps/libsandproof_protocol-*.rlib'))
                capture.source.require(len(protocol_libraries) == 1 and str(protocol_libraries[0]) in
                                       compiled_protocol['filenames'], 'native target contains a different protocol library')
                case_record.update(native_assertions_checked=True, failed_assertions=sorted(failures),
                    property_rejected=bool(failures), artifacts=[{
                        'message': artifact,
                        'files': {name: sha(Path(name)) for name in artifact['filenames'] if Path(name).is_file()}
                    } for artifact in artifacts])
            for name, wanted in expected.items():
                capture.source.require(not (project / name).is_symlink() and sha(project / name) == wanted and
                                       capture.mode(project / name) == modes[name], 'native source/mode changed')
            if mutation:
                name, before, after = mutation
                path = project / name
                text = path.read_text()
                capture.source.require(text.count(after) == 1, 'native mutant cannot restore exact bytes')
                path.write_text(text.replace(after, before))
                capture.source.require(path.read_bytes() == (ROOT / name).read_bytes(), 'native Rust restoration differs')
                restorations.append({'case': case, 'source': name, 'restored_sha256': sha(path)})
            record['cases'][case] = case_record
            (output / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')
        capture.source.require(capture.base.snapshot(ROOT) == inputs and
                               capture.registry.registry() == registry and library.capture(ROOT) == library_inputs,
                               'native source or library inputs changed')
        capture.source.require(all(capture.mode(ROOT / name) == value for name, value in modes.items()),
                               'current source modes changed')
        capture.source.require(all(sha(Path(value['path'])) == value['sha256'] for value in record['tools'].values()),
                               'native tool changed')
        for name, wanted in inputs.items():
            capture.source.require(sha(project / name) == wanted, 'final restored native source differs')
        record.update(development_complete=True, exact_byte_restorations=restorations)
    except BaseException as error:
        record.update(development_complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    with (ROOT.parent / '.proof-heavy.lock').open('r+') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        result = run(args.output.absolute())
    print(json.dumps({name: result.get(name) for name in ('development_complete', 'seconds', 'error')}))
    raise SystemExit(0 if result['development_complete'] else 1)
