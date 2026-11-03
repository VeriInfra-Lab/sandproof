"""The fixed V3 convergence batch; original drivers remain the proof producers.

This table adds one finite delivery scope. It does not replace the full V3
obligations, implement a proof evaluator, or let an operator select a subset.
"""
from pathlib import Path
import hashlib
import importlib.util
import json
import re

HERE = Path('verification/experiments/stage1-protocol')
PROFILE = 'protocol-convergence'
SCOPE = 'stage1-protocol-convergence'

# Every original correct and deliberately wrong implementation is retained.
CASES = {
    'restore-work': ('correct', 'drop-material-use', 'mask-generation'),
    'restore-lifetime': ('correct', 'omit-maximum-field', 'allow-expired-end'),
    'exit-observations': ('correct', 'direct-drop-start-ticks', 'before-pid-only', 'continued-boot-only'),
    'source-preservation': ('correct', 'drop-producer', 'drop-consumer', 'plan-binding-only', 'forget-settled'),
    'source-work-match': ('correct', 'key-only', 'anchor-only'),
    'source-observation': ('correct', 'skip-binding', 'pid-only', 'oversize-config'),
    'native-readiness': ('correct', 'ignore-status'),
    'pause-request': ('correct', 'origin-binding', 'native-incarnation', 'options-binding'),
    'source-question': ('correct', 'first-resource', 'options-binding', 'disk-as-material'),
    'source-origin': ('correct', 'skip-session', 'skip-generation', 'skip-vm'),
    'standard-blob': ('actual', 'repeat-first', 'second-error-first'),
    'standard-digest': ('actual', 'input-empty', 'nonempty-tail-zero'),
    'standard-save-digest': ('actual', 'save-input-empty', 'save-tail-zero', 'delegate-input-empty'),
    'digest-uses': ('actual', 'input-empty', 'tail-zero'),
    'lifetime-row': ('correct', 'skip-owner', 'skip-startup', 'lost-start'),
    'create-standard-identity': ('actual', 'namespace-omitted', 'number-truncated'),
    'create-standard-sql': ('actual', 'zero-deadline', 'empty-sandbox'),
    'record-reply': ('actual', 'skip-match', 'clear-terminal'),
    'protocol-record': ('correct', 'skip-independent-context', 'replay-falls-through', 'stale-local-operation'),
    'create-attachment-flow': ('actual', 'skip-save', 'eager-world'),
}
SPECIAL_LABELS = {
    **{name: 'stage1-'+name for name in (
        'standard-blob', 'standard-digest', 'standard-save-digest', 'digest-uses',
        'create-standard-identity', 'create-standard-sql', 'record-reply',
        'create-attachment-flow')},
    'protocol-record': 'stage1-protocol-record',
    'accepted-json-parser': 'stage1-accepted-json-parser',
    'sqlite-commit': 'stage1-sqlite-commit',
}
NATIVE_DRIVERS = (*tuple((SPECIAL_LABELS.get(name, 'stage1-protocol-'+name),
                        str(HERE/name/'native.py'), False) for name in CASES),
    ('stage1-accepted-json-parser', str(HERE/'accepted-json-parser/native.py'), False),
    ('stage1-sqlite-commit', str(HERE/'sqlite_convergence.py'), False))
LOCKED_NATIVE = frozenset(('standard-blob', 'standard-digest', 'standard-save-digest',
                            'digest-uses', 'record-reply'))
LOCKED_FORMAL = frozenset(CASES)-{'native-readiness', 'protocol-record'}
EXTRA_DRIVERS = {
    'production-json': (str(HERE/'production-json/formal.py'), (), 'stage1-accepted-json-parser'),
    'sqlite-commit': (str(HERE/'sqlite_convergence.py'), ('formal',), 'stage1-sqlite-commit'),
    'standard-record-read': (str(HERE/'protocol-record/standard-read/formal.py'), (),
                             'stage1-protocol-record'),
}

# One reviewed batch, not a general cross-version cache. Original receipts stay
# at their original identities; the caller records a separate current admission.
NATIVE_REUSE_MANIFEST = HERE/'native-reuse.json'
NATIVE_REUSE_COMPONENTS = (*tuple(CASES), 'accepted-json-parser', 'sqlite-commit')
NATIVE_REUSE_ADMISSION_FILES = (
    'tools/acceptance_receipts.py', 'tools/proof_native_suite.py',
    str(HERE/'convergence.py'), str(NATIVE_REUSE_MANIFEST),
    str(HERE/'registered.py'),
    'tools/tests/test_protocol_convergence_native_reuse.py',
    'tools/tests/test_native_scopes.py',
)


def native_reuse_definition(root):
    data = json.loads((root/NATIVE_REUSE_MANIFEST).read_text())
    expected = {SPECIAL_LABELS.get(name, 'stage1-protocol-'+name)
                for name in NATIVE_REUSE_COMPONENTS}
    if (data['schema'] != 1 or data['scope'] != SCOPE
            or data['kind'] != 'fixed-batch-original-native-receipts'
            or set(data['components']) != expected
            or set(data['admission_rules']) != set(NATIVE_REUSE_ADMISSION_FILES)-{str(NATIVE_REUSE_MANIFEST)}):
        raise ValueError('unknown fixed convergence native reuse definition')
    for name in NATIVE_REUSE_COMPONENTS:
        label = SPECIAL_LABELS.get(name, 'stage1-protocol-'+name)
        row = data['components'][label]
        prefix = ('verification/experiments/sqlite-commit/' if name == 'sqlite-commit'
                  else str(HERE/name)+'/')
        if (row['component_prefixes'] != [prefix]
                or any(not re.fullmatch('[0-9a-f]{64}', row[k])
                       for k in ('identity', 'receipt_sha256', 'current_component_rules_sha256'))
                or not re.fullmatch('[0-9a-f]{40}', row['execution_commit'])
                or row['dependency_files'] != sorted(set(row['dependency_files']))
                or any(Path(n).is_absolute() or '..' in Path(n).parts
                       for n in row['dependency_files'])):
            raise ValueError('invalid fixed original native component: '+label)
    return data


def native_reuse_shared_rules(rules):
    prefixes = tuple(str(HERE/name)+'/' for name in (*CASES, 'accepted-json-parser'))
    prefixes += ('verification/experiments/sqlite-commit/',)
    excluded = {*NATIVE_REUSE_ADMISSION_FILES, str(HERE/'fixed-originals.json'),
                str(HERE/'sqlite_convergence.py')}
    return {n:h for n,h in rules.items() if n not in excluded and not n.startswith(prefixes)}


def native_reuse_matches(root, label, original, current, shared):
    """Only the reviewed unchanged execution inputs may use these receipts."""
    definition = native_reuse_definition(root)
    if label not in definition['components']:
        return False
    for name, expected in definition['admission_rules'].items():
        if current['rules'].get(name) != expected:
            raise ValueError('fixed native reuse admission code changed: '+name)
    if shared.digest(native_reuse_shared_rules(current['rules'])) != definition['shared_rules_sha256']:
        raise ValueError('fixed native reuse shared execution rules changed')
    # The bounded batch retains the complete reviewed 8fc Rust/Cargo closure.
    # No claim of a general package dependency analyser is made here.
    if any(original[name] != current[name] for name in
           ('schema', 'label', 'command', 'rust', 'options', 'runtime', 'contracts', 'source_archives')):
        raise ValueError('fixed native reuse execution context changed: '+label)
    plan = definition['components'][label]
    selected = {n:h for n,h in current['rules'].items()
                if n.startswith(tuple(plan['component_prefixes'])) or n in plan['dependency_files']}
    if shared.digest(selected) != plan['current_component_rules_sha256']:
        return False
    if 'presence_plan' in plan:
        inventory = json.loads((root/plan['presence_plan']).read_text())
        for variant, row in inventory['native_variants'].items():
            missing = set(plan['required_plan_fields'])-row.keys()
            if missing:
                raise ValueError(f'fixed native reuse variant {variant} missing fields: {sorted(missing)}')
    return True


def read(root, name):
    path = root/HERE/name/'inventory.json'
    if path.is_symlink() or not path.is_file():
        raise ValueError('missing finite convergence inventory: '+name)
    return json.loads(path.read_text())


def definition(root):
    packages = []
    for name, cases in CASES.items():
        data = read(root, name)
        declared = data.get('views', data.get('cases', data.get('variants')))
        if name != 'native-readiness' and tuple(declared) != cases:
            raise ValueError('missing or extra convergence view: '+name)
        packages.append(dict(name=name, cases=list(cases),
                             native=SPECIAL_LABELS.get(name, 'stage1-protocol-'+name),
                             inventory_sha256=hashlib.sha256(
                                 (root/HERE/name/'inventory.json').read_bytes()).hexdigest()))
    return dict(profile=PROFILE, scope=SCOPE, packages=packages,
                native_labels=[row[0] for row in NATIVE_DRIVERS],
                additional_entries=tuple(EXTRA_DRIVERS),
                whole_V3_acceptance=False, full_profile_obligations_replaced=False)


def component(root, name):
    """Use the retained component's own strict original-output validator."""
    if name not in ('standard-blob', 'standard-digest', 'standard-save-digest'):
        raise ValueError('not a registered component reader')
    spec = importlib.util.spec_from_file_location(
        'convergence_'+name.replace('-', '_'), root/HERE/name/'registration.py')
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def extra(root, name):
    """A finite adapter validates its own retained source and original evidence."""
    path, _, _ = EXTRA_DRIVERS[name]
    spec = importlib.util.spec_from_file_location('convergence_extra_'+name.replace('-', '_'), root/path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def view_recipes(root, name, cases=None):
    """Derive exact original audits from the retained inventory, never a run."""
    data = read(root, name)
    result = {}
    for case in CASES[name] if cases is None else cases:
        if name == 'native-readiness':
            stems = ('NativeCompletionModel', 'NativeCompletionValues',
                     'NativeCompletionProofs' if case == 'correct' else 'NativeCompletionWrongStatus')
            audits = [audit for stem in stems for audit in data['providers'][stem]['audits']]
            files = None
            file_count = data['correct_files' if case == 'correct' else 'wrong_files']
        elif name == 'protocol-record':
            row = data['views'][case]
            files = row['files']
            by_file = row['audits']
            if set(by_file) != set(files) or any(not isinstance(by_file[n], list) for n in files):
                raise ValueError('original caller audit pages differ: '+case)
            audits = [audit for file in files for audit in by_file[file]]
            file_count = len(files)
        elif name == 'record-reply':
            row = data['variants'][case]
            index = json.loads((root/HERE/name/'generated-index.json').read_text())['variants'][case]
            audits, files = index['expected_audits'], index['expected_files']
            file_count = len(files)
            if file_count != row['expected_files'] or len(audits) != row['expected_audits']:
                raise ValueError('original reply counts differ from its exact generated index: '+case)
        else:
            rows = data.get('cases', data.get('variants', data.get('views')))
            row = rows[case] if isinstance(rows, dict) and isinstance(rows[case], dict) else {}
            audits = row.get('expected_audits', row.get('audits', data.get('audits', {}).get(case)))
            files = row.get('expected_files', row.get('files', data.get('files', {}).get(case)))
            file_count = len(files) if files is not None else None
        if (not isinstance(audits, list) or not audits or len(audits) != len(set(audits))
                or any(not isinstance(a, str) or not re.fullmatch(r'\S+', a) for a in audits)):
            raise ValueError('missing or duplicate original convergence audits: '+name+':'+case)
        result[case] = dict(audits=audits, files=files, file_count=file_count,
                            property_rejected=case not in ('actual', 'correct'))
    return result


def append_jobs(jobs, output, root, *, profile):
    definition(root)
    for name, cases in CASES.items():
        if name == 'create-attachment-flow':
            # The existing Create plan remains its sole registration.
            continue
        label = SPECIAL_LABELS.get(name, 'stage1-protocol-'+name)
        variants = cases if name in ('native-readiness', 'protocol-record') else (None,)
        for variant in variants:
            args = []
            if variant is not None:
                args = ['--require-native', '--variant', variant, '--native-scope',
                        SCOPE if profile == PROFILE else 'stage1-protocol']
            job_name = name if variant is None else name+'-'+variant
            selected = cases if variant is None else (variant,)
            jobs.append(dict(name=job_name, driver=str(HERE/name/'formal.py'), args=args,
                output=str(Path(output)/job_name), native=[label],
                cases=list(selected), actual_files=True,
                convergence_package=name, profile=profile,
                inventory_sha256=hashlib.sha256(
                    (root/HERE/name/'inventory.json').read_bytes()).hexdigest(),
                views=view_recipes(root, name, selected)))
    return jobs


def append_extra_jobs(jobs, output, root, *, profile):
    for name, (driver, args, label) in EXTRA_DRIVERS.items():
        specification = extra(root, name).plan(root)
        views = specification['views']
        if (not isinstance(views, dict) or not views or any(
                not row.get('files') or not row.get('audits')
                or len(row['audits']) != len(set(row['audits'])) for row in views.values())):
            raise ValueError('missing fixed adapter physical file and audit inventory: '+name)
        jobs.append(dict(name=name, driver=driver, args=list(args),
            output=str(Path(output)/name), native=[label], cases=list(views),
            views=views, actual_files=True, convergence_extra=name, profile=profile))
    return jobs


# These are the original producers' completion fields, not substitutes for
# their physical-file, kernel, service, source and counterexample checks.
CASE_BATCHES = {
    'restore-work': ('actual-restore-work-development-batch', 'all_three_views_complete',
                     'all_source_mode_tool_library_log_postguards_complete'),
    'restore-lifetime': ('actual-restore-lifetime-development-batch', 'all_three_views_complete',
                         'all_source_mode_tool_library_log_postguards_complete'),
    'exit-observations': ('exit-observation-formal-development', 'all_four_views_complete',
                          'source_tools_library_and_log_postguards_complete'),
    'source-preservation': ('source-preservation-formal-development', 'all_five_views_complete',
                            'source_tools_library_and_log_postguards_complete'),
    'source-work-match': ('source-work-match-formal-development', 'all_three_views_complete',
                          'source_tools_library_and_log_postguards_complete'),
    'source-observation': ('source-observation-formal-development', 'all_four_views_complete',
                           'source_tools_library_and_log_postguards_complete'),
    'pause-request': ('pause-request-formal-development', 'all_four_views_complete',
                      'source_tools_library_and_log_postguards_complete'),
    'source-question': ('source-question-formal-development', 'all_four_views_complete',
                        'source_tools_library_and_log_postguards_complete'),
    'source-origin': ('source-origin-formal-development', 'all_four_views_complete',
                      'source_tools_library_and_log_postguards_complete'),
    'lifetime-row': ('lifetime-row-formal-development', 'all_four_views_complete',
                     'source_tools_library_and_log_postguards_complete'),
}
VARIANT_BATCHES = {
    'create-standard-identity': 'actual-standard-identity-delegate-development-batch',
    'create-standard-sql': 'actual-standard-sql-delegate-development-batch',
    'digest-uses': 'current-adopted-digest-uses-complete-development-batch',
    'record-reply': 'actual-adopted-record-reply-complete-development-batch',
}


def actuals(result):
    if result.get('convergence_entry') in EXTRA_DRIVERS:
        rows = result.get('actuals')
        if (not isinstance(rows, list) or not rows
                or any(not row.get('files') or not row.get('audits')
                       or not row.get('service_log_sha256') for row in rows)):
            raise ValueError('missing original fixed-adapter actual views')
        return rows
    if result.get('kind') in {row[0] for row in CASE_BATCHES.values()}:
        return [row['actual'] for row in result.get('cases', {}).values()]
    if result.get('kind') in set(VARIANT_BATCHES.values()):
        return [row['actual'] for row in result.get('variants', {}).values()]
    if result.get('kind') in {
        'current-standard-blob-and-original-load-domain-development',
        'current-standard-digest-and-original-load-development',
        'current-standard-save-digest-and-original-save-development'}:
        return [row['actual'] for row in result.get('views', {}).values()]
    return None


def check_actual(result, job, root):
    name = job['convergence_package']
    if result.get('complete') is not True or result.get('prepare_only'):
        raise ValueError('incomplete or preparation-only convergence result: '+name)
    if name in ('standard-blob', 'standard-digest', 'standard-save-digest'):
        component(root, name).check(result, root)
        return
    data = read(root, name)
    if name in CASE_BATCHES:
        kind, all_views, postguards = CASE_BATCHES[name]
        if (result.get('kind') != kind or result.get(all_views) is not True
                or result.get(postguards) is not True
                or result.get('selected_variant') is not None
                or tuple(result.get('cases', {})) != CASES[name]):
            raise ValueError('incomplete fixed convergence cases: '+name)
        for case in CASES[name]:
            row = result['cases'][case]
            actual = row.get('actual', {})
            recipe = data.get('cases', {}).get(case, {})
            files = recipe.get('files', data.get('files', {}).get(case))
            audits = recipe.get('audits', data.get('audits', {}).get(case))
            if (row.get('complete') is not True
                    or row.get('property_rejected') is not (case != 'correct')
                    or not files or not audits
                    or set(actual.get('files', {})) != set(files)
                    or actual.get('files') != row.get('files')
                    or sorted(actual.get('audits', [])) != sorted(audits)
                    or sorted(row.get('audits', [])) != sorted(audits)
                    or not actual.get('service_log_sha256')
                    or row.get('service_log_sha256') != actual['service_log_sha256']):
                raise ValueError('incomplete convergence physical view: '+name+':'+case)
            if name not in ('restore-work', 'restore-lifetime') and (
                    row.get('seeded_from_public_correct_entry') is not True
                    or not row.get('source_inputs') or not row.get('source_input_modes')):
                raise ValueError('missing original source/mode admission: '+name+':'+case)
        return
    if name in VARIANT_BATCHES:
        rows = result.get('variants', {})
        if (result.get('kind') != VARIANT_BATCHES[name]
                or result.get('prepare_only') is not False
                or tuple(rows) != CASES[name]):
            raise ValueError('incomplete fixed convergence variants: '+name)
        if name.startswith('create-standard-') and result.get('all_variants_complete') is not True:
            raise ValueError('missing standard delegate completion: '+name)
        for case, row in rows.items():
            recipe = job['views'][case]
            if (row.get('complete') is not True
                    or row.get('property_rejected') is not (case != 'actual')
                    or not row.get('files') or not row.get('audits')
                    or row.get('actual', {}).get('files') != row['files']
                    or row.get('actual', {}).get('audits') != row['audits']
                    or set(row['files']) != set(recipe['files'])
                    or sorted(row['audits']) != sorted(recipe['audits'])
                    or not row.get('actual', {}).get('service_log_sha256')):
                raise ValueError('incomplete original variant files/audits: '+name+':'+case)
        if name in ('digest-uses', 'record-reply'):
            prefix = ('common_contract_sha256', 'common_values_sha256',
                      'common_library_sha256')
            fourth = 'common_carriers_sha256' if name == 'digest-uses' else 'common_recorded_sha256'
            if (any(None in {row.get(k) for row in rows.values()}
                    or len({row.get(k) for row in rows.values()}) != 1 for k in (*prefix, fourth))
                    or any(row.get('capture_complete') is not True
                           or row.get('translation_complete') is not True for row in rows.values())):
                raise ValueError('different correct/wrong full contracts or incomplete capture: '+name)
            if name == 'digest-uses' and (
                    result.get('same_complete_contract_and_value_library_carriers') is not True
                    or result.get('source_binding', {}).get('complete') is not True):
                raise ValueError('missing actual six-use binding')
            if name == 'record-reply' and (
                    result.get('registry_source_admitted_before_capture') is not True
                    or result.get('same_complete_contract_values_recorded_and_library') is not True
                    or result.get('source_worktree_inputs_and_modes_unchanged') is not True
                    or result.get('fixed_runtime_before') != result.get('fixed_runtime_after')
                    or any(row.get('original_kernel_service_dependencies_read_back') is not True
                           for row in rows.values())):
                raise ValueError('missing complete actual reply source/runtime binding')
        return
    if name in ('native-readiness', 'protocol-record'):
        variant, = job['cases']
        kind = name+'-formal-development'
        actual = result.get('actual', {})
        view = data['views'][variant] if name == 'protocol-record' else None
        files = view['files'] if view else data['correct_files' if variant == 'correct' else 'wrong_files']
        file_inventory_matches = (set(actual.get('files', {})) == set(files) if view
                                  else len(actual.get('files', {})) == files)
        audit_inventory_matches = (sorted(actual.get('audits', [])) == sorted(job['views'][variant]['audits']) if view
                                   else len(actual.get('audits', [])) == data[
                                       'correct_audits' if variant == 'correct' else 'wrong_audits'])
        rejection_matches = result.get('property_rejected') is (variant != 'correct')
        if name == 'native-readiness' and 'property_rejected' not in result:
            # The original producer reports its exact positive counterexample
            # proofs, not this aggregate Boolean. Keep that result unchanged.
            rejection_matches = sorted(actual.get('audits', [])) == sorted(
                job['views'][variant]['audits'])
            if variant != 'correct':
                rejection_matches = rejection_matches and (
                    'SandboxModel.Stage1Protocol.NativeReadiness.wrong_status_refutes_full_contract'
                    in actual.get('audits', []))
        original_files_match = actual.get('files') == result.get('files')
        original_audits_match = actual.get('audits') == result.get('audits')
        if name == 'native-readiness':
            original_files_match = (len(result.get('prepared_files', [])) == len(actual.get('files', {}))
                                    and set(result.get('prepared_files', [])) == set(actual.get('files', {})))
            original_audits_match = sorted(result.get('prepared_audits', [])) == sorted(actual.get('audits', []))
        if (result.get('kind') != kind or result.get('variant') != variant
                or not rejection_matches
                or not file_inventory_matches or not audit_inventory_matches
                or not original_files_match
                or not actual.get('service_log_sha256')
                or not original_audits_match
                or (name == 'native-readiness' and result.get(
                    'source_modes_tools_libraries_logs_postguards_complete') is not True)):
            raise ValueError('incomplete actual caller/readiness view: '+name+':'+variant)
        return
    raise ValueError('missing convergence result validator: '+name)
