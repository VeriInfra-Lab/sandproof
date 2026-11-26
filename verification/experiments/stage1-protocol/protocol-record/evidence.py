"""Registered admission for the current caller's physical proof and refutations."""
import json
import re
import subprocess
from pathlib import Path

NAMESPACE = 'SandboxModel.Stage1Protocol.ProtocolRecord.'
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
HERE = Path('verification/experiments/stage1-protocol/protocol-record')
GENERATED_VIEWS = {
    'request': ('request', 'SandproofStage1Request'),
    'primitive': ('graphic', 'SandproofStage1Graphic'),
}
CONTRACTS = {
    'correct': ('InboxProtocolRecord', 'full_flow_contract', ()),
    'skip-independent-context': (
        'InboxProtocolWrongContext', 'WrongContext.violates_full_contract',
        ('WrongContext.foreign_node', 'WrongContext.model_refuses', 'WrongContext.actual_records')),
    'replay-falls-through': (
        'InboxProtocolWrongReplay', 'WrongReplay.violates_full_contract',
        ('WrongReplay.model_replay', 'WrongReplay.actual_not_replay')),
    'stale-local-operation': (
        'InboxProtocolWrongOperation', 'WrongOperation.violates_full_contract',
        ('WrongOperation.actual_attached', 'WrongOperation.actual_not_current')),
}


def certificate(result, job, inventory):
    case, = job['cases']
    page, declaration, witnesses = CONTRACTS[case]
    theorem = NAMESPACE + declaration
    if (result.get('complete') is not True or result.get('prepare_only')
            or result.get('kind') != 'protocol-record-formal-development'
            or result.get('variant') != case):
        raise ValueError('incomplete current ProtocolRecord view: ' + case)
    actual = result.get('actual', {})
    recipe = job['views'][case]
    files, audits = actual.get('files', {}), actual.get('audits', [])
    if (set(files) != set(recipe['files']) or files != result.get('prepared_files')
            or not actual.get('service_log_sha256')):
        raise ValueError('current ProtocolRecord physical page inventory differs: ' + case)
    if result.get('contract_theorem') != theorem or theorem not in audits:
        raise ValueError('missing current ProtocolRecord contract theorem: ' + theorem)
    for witness in witnesses:
        if NAMESPACE + witness not in audits:
            raise ValueError('missing current ProtocolRecord refutation witness: ' + witness)
    if (sorted(audits) != sorted(recipe['audits']) or sorted(audits) != sorted(result.get('prepared_audits', []))
            or len(set(audits)) != len(audits)):
        raise ValueError('current ProtocolRecord audits differ: ' + case)
    models = inventory['same_models']
    if len(models) != 2 or any(files.get(name) != digest for name, digest in models.items()):
        raise ValueError('current ProtocolRecord uses a different load or flow contract: ' + case)
    if 'property_rejected' in result and result['property_rejected'] is not (case != 'correct'):
        raise ValueError('current ProtocolRecord outcome contradicts its theorem: ' + case)
    return page, declaration.rsplit('.', 1)[-1], theorem


def check_theorem_page(text, original, case, declaration, theorem):
    proposition = ('¬ ' if case != 'correct' else '') + 'FullFlowContract record_using'
    statement = r'^theorem\s+' + re.escape(declaration) + r'\s*:\s*' + re.escape(proposition) + r'\s*:=\s*by'
    if re.search(statement, text, re.M) is None:
        raise ValueError('missing current ProtocolRecord theorem statement: ' + theorem)
    if text != original or f'#audit_controller_proof {theorem}' not in text.splitlines():
        raise ValueError('current ProtocolRecord theorem page differs from its provider: ' + theorem)


def check_native_assertion(native, result, case, mutations):
    variant = 'original' if case == 'correct' else case
    if (native.get('kind') != 'protocol-record-native-development' or native.get('complete') is not True
            or native.get('source_commit') != result['source_commit']
            or native.get('inputs') != result['inputs'] or native.get('registry') != result['registry']):
        raise ValueError('current ProtocolRecord native source differs: ' + case)
    source = native['variants'][variant]
    changes = result['business_inputs']
    if (source.get('complete') is not True or not changes
            or any(source['inputs'].get(name) != digest for name, digest in changes.items())
            or native.get('mutations', {}).get(variant) != result['mutation']):
        raise ValueError('current ProtocolRecord native mutation differs: ' + case)
    tests = [row for row in native['commands'] if row['variant'] == variant and row['label'] == 'test']
    if len(tests) != 1:
        raise ValueError('missing current ProtocolRecord native assertion: ' + case)
    test, = tests
    wrong = case != 'correct'
    expected = sorted(mutations[variant]['expected_failures']) if wrong else []
    if (test['expected_failures'] != expected or test['observed_failures'] != expected
            or bool(expected) is not wrong or test['exit_code'] != (101 if wrong else 0)
            or test.get('property_rejected', False) is not wrong):
        raise ValueError('current ProtocolRecord native assertion differs: ' + case)
    return variant, test['log_sha256']


def check_generated_capture(label, capture, actual_files):
    view, namespace = GENERATED_VIEWS[label]
    generated = ('Types.lean', 'Funs.lean')
    expected = {'translation.json', *(f'{namespace}/{name}' for name in (
        *generated, 'TypesExternal_Template.lean', 'FunsExternal_Template.lean'))}
    if set(capture) != expected:
        raise ValueError('incomplete raw ProtocolRecord generated files: '+label)
    base = f'verification/aeneas/stage1-protocol/{view}/{namespace}'
    if any(actual_files.get(f'{base}/{name}') != capture[f'{namespace}/{name}']
           for name in generated):
        raise ValueError('ProtocolRecord physical generated source differs from raw capture: '+label)


def check_physical(root, job, result, folder, evidence, shared, sealed_file):
    inventory = json.loads((root/HERE/'inventory.json').read_bytes())
    page, declaration, theorem = certificate(result, job, inventory)
    case, = job['cases']
    project = folder/'source'
    current = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
    if current != result.get('source_commit'):
        raise ValueError('current ProtocolRecord is bound to another commit: ' + case)

    def checked(path, digest):
        relative = path.resolve()
        if (path.is_symlink() or not relative.is_relative_to(folder)
                or evidence.get(str(relative)) != digest):
            raise ValueError('missing or changed current ProtocolRecord artifact: ' + str(path))

    actual = result['actual']
    relative_page = PROOFS/(page+'.lean')
    checked(project/relative_page, actual['files'][str(relative_page)])
    check_theorem_page((project/relative_page).read_text(), (root/HERE/(page+'.lean.in')).read_text(),
                       case, declaration, theorem)
    for name, digest in result['business_inputs'].items():
        checked(project/name, digest)
    labels = set()
    for command in result['commands']:
        label = command['label']
        if (re.fullmatch(r'[a-z0-9-]+', label) is None or label in labels
                or command['cwd'] != str(project) or command['exit_code'] != 0):
            raise ValueError('current ProtocolRecord command differs: ' + label)
        labels.add(label)
        checked(folder/(label+'.log'), command['log_sha256'])
    required = {'build', 'request-charon', 'request-aeneas', 'request-admission-regressions',
                'primitive-charon', 'primitive-aeneas', 'primitive-admission-regressions'}
    if not required <= labels:
        raise ValueError('missing current ProtocolRecord extraction or build command')
    if set(result['llbc']) != set(GENERATED_VIEWS) or set(result['raw_generated']) != set(GENERATED_VIEWS):
        raise ValueError('missing current ProtocolRecord extraction layers')
    for label, ir in result['llbc'].items():
        if ir['path'] != f'artifacts/{label}.llbc':
            raise ValueError('current ProtocolRecord extraction path differs: ' + label)
        checked(project/ir['path'], ir['sha256'])
        captured = result['raw_generated'][label]
        check_generated_capture(label, captured, actual['files'])
        for name, digest in captured.items():
            checked(folder/('generated-'+label)/name, digest)

    args = job['args']
    scope = args[args.index('--native-scope')+1]
    receipt = shared.native_case('stage1-protocol-record', root, None, scope=scope)
    if result['native_receipt'] != receipt:
        raise ValueError('current ProtocolRecord refers to a different native receipt')
    native_path = sealed_file(receipt, 'result.json')
    native = json.loads(native_path.read_bytes())
    mutations = json.loads((root/HERE/'mutations.json').read_bytes())['variants']
    variant, digest = check_native_assertion(native, result, case, mutations)
    test_log = sealed_file(receipt, variant+'/test.log')
    if shared.sha(test_log) != digest:
        raise ValueError('current ProtocolRecord native assertion log differs: ' + case)
    return {str(native_path): shared.sha(native_path), str(test_log): digest}
