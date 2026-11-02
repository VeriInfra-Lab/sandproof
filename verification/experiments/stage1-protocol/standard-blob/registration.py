"""Coordinator candidate: validate the complete original three-view result."""
from pathlib import Path
import hashlib
import json

HERE = Path('verification/experiments/stage1-protocol/standard-blob')
KIND = 'current-standard-blob-and-original-load-domain-development'
CASES = ('actual', 'repeat-first', 'second-error-first')


def check(result, root):
    data = json.loads((root/HERE/'inventory.json').read_text())
    if (result.get('kind') != KIND or result.get('complete') is not True
            or result.get('source_candidate_adopted') is not True
            or result.get('finite_library_provider_adopted') is not True
            or result.get('prepared') is not False
            or result.get('all_postguards_complete') is not True
            or result.get('all_published_generated_comparisons_complete') is not True
            or tuple(result.get('views', {})) != CASES):
        raise ValueError('missing, extra or incomplete standard BLOB view')
    audits = set()
    captures = list(result.get('load', {}).get('captures', {}).values())
    if len(captures) != 3:
        raise ValueError('missing original/canonical/primitive Load capture')
    for case in CASES:
        row, expected = result['views'][case], data['views'][case]
        actual = row.get('actual', {})
        if (row.get('complete') is not True
                or row.get('property_rejected') is not (case != 'actual')
                or row.get('correct_core_imported') is not (case == 'actual')
                or set(actual.get('files', {})) != set(expected['files'])
                or actual.get('files') != row.get('files')
                or sorted(actual.get('audits', [])) != expected['audits']
                or sorted(row.get('audits', [])) != expected['audits']
                or not actual.get('service_log_sha256')
                or row.get('all_original_kernel_service_dependencies_read_back') is not True):
            raise ValueError('incomplete original BLOB files/audits/counterexample: '+case)
        audits.update(actual['audits'])
        captures.append(row.get('captures', {}).get('standard', {}))
    if sorted(audits) != result.get('union_audits'):
        raise ValueError('complete BLOB audit union differs')
    if any(row.get('published_comparison', {}).get('complete') is not True for row in captures):
        raise ValueError('missing fresh generated-body comparison')
    return [result['views'][case]['actual'] for case in CASES]


def check_native(result, root, folder):
    plans = json.loads((root/HERE/'plan.json').read_text())['native_variants']
    if (result.get('kind') != 'current-standard-blob-candidate-native-development'
            or result.get('complete') is not True
            or result.get('production_adopted') is not True
            or result.get('finite_provider_adopted') is not True
            or tuple(result.get('variants', {})) != tuple(plans)
            or not result.get('inputs') or not result.get('registry')):
        raise ValueError('incomplete standard BLOB native source/views')
    commands = result.get('commands', [])
    if [(r.get('variant'), r.get('label')) for r in commands] != [
            (case, label) for case in plans for label in ('format', 'test', 'clippy')]:
        raise ValueError('missing or extra original native command')
    for case, plan in plans.items():
        row = result['variants'][case]
        if row.get('complete') is not True or not row.get('inputs'):
            raise ValueError('incomplete native source view: '+case)
        for command in (r for r in commands if r['variant'] == case):
            label = command['label']
            log = folder/case/(label+'.log')
            if (log.is_symlink() or not log.is_file()
                    or hashlib.sha256(log.read_bytes()).hexdigest() != command.get('log_sha256')):
                raise ValueError('missing or changed original native log')
            failures = plan['expected_failures'] if label == 'test' else []
            if command.get('exit_code') != (101 if failures else 0):
                raise ValueError('native compile/lint/timeout is not a property rejection')
            if bool(command.get('property_rejected')) != bool(failures):
                raise ValueError('incorrect native property rejection flag')
            if label == 'test':
                text = log.read_text()
                block = text.rsplit('\nfailures:\n', 1)[-1].split('\ntest result:', 1)[0]
                observed = [line.strip().rsplit('::', 1)[-1] for line in block.splitlines()
                            if line.startswith('    controller::')]
                tally = f"{plan['test_count']-len(failures)} passed; {len(failures)} failed; 0 ignored;"
                if sorted(observed) != sorted(failures) or tally not in text:
                    raise ValueError('wrong native source did not reach the exact assertions')
