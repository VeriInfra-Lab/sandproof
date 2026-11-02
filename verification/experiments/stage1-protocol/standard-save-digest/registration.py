"""Main coordinator candidate; structure checks supplement original readback."""
from pathlib import Path
import hashlib
import json

HERE = Path('verification/experiments/stage1-protocol/standard-save-digest')
KIND = 'current-standard-save-digest-and-original-save-development'
CASES = ('actual','save-input-empty','save-tail-zero','delegate-input-empty')


def check(result, root):
    data = json.loads((root/HERE/'inventory.json').read_text())
    if (result.get('kind') != KIND or result.get('complete') is not True
            or result.get('prepared') is not False
            or result.get('all_postguards_complete') is not True
            or result.get('all_published_generated_comparisons_complete') is not True
            or tuple(result.get('views', {})) != CASES
            or result.get('source_adopted') is not False
            or result.get('library_provider_adopted') is not False
            or result.get('original_direct_method_machine_refined') is not False):
        raise ValueError('missing, extra, incomplete or misrepresented standard Save SHA view')
    load = result.get('generic_save', {}).get('captures', {})
    if tuple(load) != ('original', 'canonical', 'primitive'):
        raise ValueError('original/canonical/primitive Save captures are required')
    published = list(load.values())
    original = result.get('original', {}).get('capture', {}).get('source_binding', {})
    leaf = result['views']['actual'].get('leaf_binding', {}).get('source_binding', {})
    for binding in (original, leaf):
        if (binding.get('complete') is not True or binding.get('complete_input') is not True
                or binding.get('complete_array32') is not True
                or binding.get('extra_object_access_or_business_branch') is not False
                or binding.get('library_conformance_proved') is not False
                or binding.get('kernel_implementation_refinement') is not False):
            raise ValueError('complete fixed-library source binding is missing')
    if (original.get('original_receiver_unused') is not True
            or original.get('exact_fixed_interfaces') != leaf.get('exact_fixed_interfaces')):
        raise ValueError('original/candidate complete library interface differs')
    audits = set()
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
            raise ValueError('incomplete actual SHA files/audits/counterexample: '+case)
        audits.update(actual['audits'])
        published.append(row.get('capture', {}))
    if sorted(audits) != result.get('union_audits'):
        raise ValueError('complete SHA audit union differs')
    if any(row.get('published_comparison', {}).get('complete') is not True for row in published):
        raise ValueError('fresh original generated-body comparison is missing')
    return [result['views'][case]['actual'] for case in CASES]


def check_native(result, root, folder):
    plans = json.loads((root/HERE/'plan.json').read_text())['native_variants']
    if (result.get('kind') != 'current-standard-save-digest-candidate-native-development'
            or result.get('complete') is not True
            or tuple(result.get('variants', {})) != tuple(plans)
            or not result.get('inputs') or not result.get('registry')):
        raise ValueError('incomplete standard digest native source/views')
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
