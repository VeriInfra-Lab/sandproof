"""Strict eight-method capture, exact29 interfaces and complete parent readback."""
import importlib.util,json,re,sys,tarfile
from pathlib import Path
from tempfile import TemporaryDirectory
HERE=Path(__file__).resolve().parent
PARENT=HERE.parent/'source-remove-retired'
sys.path.insert(0,str(PARENT))
def module(name,path):
 spec=importlib.util.spec_from_file_location(name,path)
 value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value
capture=module('source_end_receipt_capture',HERE/'capture.py')
inherited=module('source_end_receipt_parent_validation',HERE.parent/'source-end-receipt/validation.py')
original_inputs=inherited.original_inputs
digest=original_inputs.digest
SUPPORT=original_inputs.SUPPORT
REPO=capture.retire.REPO
ROOT='controller.lifecycle_inbox.source_work.full.end.receipt'
LIBRARIES=capture.LIBRARIES

def parsed(data):return original_inputs.parsed(data,capture.NAMESPACE)

def current(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    expected = dict(source=capture.retire.SOURCE, tree=capture.retire.TREE, variant=variant,
        options=capture.OPTIONS, driver=digest(HERE/'capture.py'),
        base_driver=digest(PARENT/'extract.py'), ports_driver=digest(PARENT/'ports.py'),
        parent_driver=digest(Path(capture.parent.__file__)), capture_roots=list(capture.ROOTS),
        direct_original_public_receipt=True, complete_constructor_fields=6, behavior_proved=False,
        mutation=None if variant=='original' else variant, complete=True, public_acceptance=False)
    if result.get('preflight_only') or any(result.get(name) != wanted for name, wanted in expected.items()):
        raise ValueError('the exact completed original request-end capture is required')
    if [(c['label'], c['exit_code'], c['bounded_stop']) for c in result['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both original fixed tools must actually finish successfully')
    fixed = json.loads((REPO / 'verification/foundations-environment.json').read_bytes())
    ir, generated = directory / 'actual.llbc', directory / 'generated'
    selectors = [part for flag, group in (('--start-from', 'roots'),
        ('--start-from-if-exists', 'retained_roots'), ('--include', 'include'), ('--opaque', 'opaque'))
        for pattern in capture.OPTIONS[group] for part in (flag, pattern)]
    tails = [
        ['cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default', *selectors,
         '--dest-file', str(ir), '--', '--manifest-path', 'controller-service/Cargo.toml',
         '--lib', '--locked', '--offline', '--target', fixed['target']],
        ['-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
         '-namespace', capture.NAMESPACE, '-subdir', capture.NAMESPACE, '-dest', str(generated), str(ir)],
    ]
    for command, tail in zip(result['commands'], tails, strict=True):
        binary = Path(command['argv'][0])
        if (command['argv'][1:] != tail or not binary.is_absolute()
                or digest(binary) != fixed[command['label']]['binary_sha256']):
            raise ValueError('the complete exact fixed tool command and binary are required')
    original_inputs.read_exact(directory / 'source.tar', capture.retire.ARCHIVE)
    for command in result['commands']:
        original_inputs.read_exact(directory / (command['label'] + '.log'), command['log_sha256'])
    if set(result['staged']) != {str(path) for path in capture.STAGED}:
        raise ValueError('all thirteen exact staged files are mandatory')
    with TemporaryDirectory(prefix='source-end-validation-', dir=directory.parent) as temporary:
        source = Path(temporary)
        capture.reconstruct(directory / 'source.tar', source, variant)
        with tarfile.open(directory / 'source.tar') as archive:
            members = {m.name: m for m in archive.getmembers() if m.isfile()}
        if len(members) != 4906:
            raise ValueError('all4906 complete fixed production files are mandatory')
        paths = list((directory / 'source').rglob('*'))
        if any(path.is_symlink() for path in paths):
            raise ValueError('the full captured source cannot contain symlinks')
        if {str(p.relative_to(directory / 'source')) for p in paths if p.is_file()} != set(members):
            raise ValueError('the entire fixed production source inventory must remain present')
        for name, member in members.items():
            path = directory / 'source' / name
            if (digest(path) != digest(source / name)
                    or bool(path.stat().st_mode & 0o111) != bool(member.mode & 0o111)):
                raise ValueError('captured source differs from exact reconstruction: ' + name)
        for name, wanted in result['staged'].items():
            original_inputs.read_exact(directory / 'source' / name, wanted)
    original_inputs.read_exact(directory / 'actual.llbc', result['ir'])
    generated = directory / 'generated'
    names = {'translation.json', *(capture.NAMESPACE + '/' + name + '.lean' for name in (
        'Types', 'Funs', 'TypesExternal_Template', 'FunsExternal_Template'))}
    if (set(result['generated']) != names
            or {str(p.relative_to(generated)) for p in generated.rglob('*') if p.is_file()} != names):
        raise ValueError('all actual generated files are required')
    for name, wanted in result['generated'].items():
        original_inputs.read_exact(generated / name, wanted)
    if capture.admit(directory / 'actual.llbc', generated, fixed['target']) != result['admission']:
        raise ValueError('the original transparent business/library mapping changed')
    sources = {name: (generated / capture.NAMESPACE / (name + '.lean')).read_bytes()
               for name in ('Types', 'Funs')}
    sources['External'] = (generated / capture.NAMESPACE / 'FunsExternal_Template.lean').read_bytes()
    sources['TypesExternal'] = (generated / capture.NAMESPACE / 'TypesExternal_Template.lean').read_bytes()
    libraries = set(re.findall(r'^axiom\s+([^\s(:]+)', sources['External'].decode(), re.M))
    expected_libraries = LIBRARIES
    if libraries != expected_libraries or sorted(libraries) != result['admission']['libraries']:
        raise ValueError('only the exact twenty-nine typed library interfaces are permitted')
    return result, sources


PUBLIC = 'controller.lifecycle_inbox.source_work.full.end.SourceWorkRecord.capture_work_end_receipt'
CTOR = 'capture.continuation.OriginalContinueWorkRequest.from_ended_source'
QUESTION = 'capture.continuation.OriginalContinueWorkRequest'
NEW = {'Types': {QUESTION}, 'Funs': {PUBLIC, CTOR,
    *(QUESTION+'.impl.'+name for name in ('identity','full','source','end_receipt','network_policy','file_policy'))}}


def parent(project, audit, retention, retention_audit, reference):
    plan, evidence = original_inputs.prior(project, audit, 'SourceEndReceiptValues', 207, 1175)
    kept, kept_evidence = original_inputs.prior(retention, retention_audit, 'RetentionProviders', 26, 127)
    if (plan['source'], plan['tree']) != (capture.retire.SOURCE, capture.retire.TREE):
        raise ValueError('same exact original production source/tree required')
    if any(plan['files'].get(n) != h for n, h in kept['files'].items()):
        raise ValueError('all26/127 original retained providers are required')
    sources = {n:(project/n).read_bytes() for n in plan['files']}
    shared, added = {}, {}
    for group, count, total in [('Types',116,117),('Funs',183,191)]:
        shared[group], added[group] = {}, {}
        rows = parsed(reference[group])
        if len(rows) != total:
            raise ValueError('all complete actual declarations required: '+group)
        for name, body in rows.items():
            owners = [n for n, raw in sources.items() if raw.count(body) == 1]
            (shared[group] if owners else added[group])[name] = owners if owners else body
        if len(shared[group]) != count or set(added[group]) != NEW[group]:
            raise ValueError('exact whole parent bodies and actual type/eight method additions required: '+group)
    return plan, dict(parent=evidence,retention=kept_evidence,shared=shared), added


def receipt_wrong_parent(correct_project):
    # The input is the original complete parent batch layout, not a fabricated
    # new checkpoint. Read both original wrong leaves and all209/1180 evidence.
    project = correct_project.parent/'no-request-as-returned-project'
    audit = correct_project.parent/'no-request-as-returned-audit'
    plan, evidence = original_inputs.prior(project,audit,'SourceEndReceiptNegative',209,1180)
    for name in ('SourceEndReceiptWrong.lean','SourceEndReceiptNegative.lean'):
        if name not in plan['files']:
            raise ValueError('both original wrong parent leaves are required')
    return project, plan, evidence


def same_interfaces(reference, actual, variant, wrong_parent=None):
    for group in ('Types','TypesExternal','External'):
        if reference[group] != actual[group]:
            raise ValueError('all117 raw types and exact29 interfaces must agree: '+group)
    before, after = parsed(reference['Funs']), parsed(actual['Funs'])
    expected = ROOT if variant == 'receipt-no-request-as-returned' else CTOR
    if (len(before) != 191 or before.keys() != after.keys()
            or [n for n in before if before[n] != after[n]] != [expected]):
        raise ValueError('one exact complete function may differ; all191 remain')
    if variant == 'receipt-no-request-as-returned':
        if wrong_parent is None or (wrong_parent/'SourceEndReceiptWrong.lean').read_bytes().count(after[ROOT]) != 1:
            raise ValueError('the whole actual wrong receipt must match the checked parent NoRequest mutant')
    elif variant == 'swap-receipt-bindings':
        old = b'  ok { identity, full, source, end_receipt, network_policy, file_policy }'
        new = b"""  ok
    {
      identity,
      full,
      source,
      end_receipt :=
        {
          end_receipt
            with
            work_binding := end_receipt.end_binding,
            end_binding := end_receipt.work_binding
        },
      network_policy,
      file_policy
    }"""
        if before[CTOR].count(old) != 1 or before[CTOR].replace(old,new,1) != after[CTOR]:
            raise ValueError('only the exact two receipt-binding assignments may be swapped')
    else:
        raise ValueError('both exact registered wrong kinds are required')
    return dict(changed_function=expected,unchanged_complete_functions=190,
        complete_types=117,exact_interfaces=29,actual_mutation_checked=True)


def declaration(reference, group, name, body):
    return inherited.declaration(reference,group,name,body)


def extra_inputs():
    from types import ModuleType
    pending = [capture,inherited,original_inputs,*tuple(sys.modules.values())]
    visited, files = set(), set(inherited.extra_inputs())
    for name in ('check.py','inputs.py'):
        files.add(str((PARENT/name).relative_to(REPO)))
    for name in ('SourceEndReceiptWrong.lean.in','SourceEndReceiptNegative.lean.in'):
        files.add(str((HERE.parent/'source-end-receipt'/name).relative_to(REPO)))
    while pending:
        value = pending.pop()
        if not isinstance(value,ModuleType) or id(value) in visited:
            continue
        visited.add(id(value))
        filename = getattr(value,'__file__',None)
        if not filename or str(filename).startswith('<'):
            continue
        path = Path(filename).resolve()
        if not path.is_relative_to(REPO):
            continue
        if path.suffix != '.py' or not path.is_file():
            raise ValueError('a complete repository Python source is required')
        files.add(str(path.relative_to(REPO)))
        pending.extend(x for x in vars(value).values() if isinstance(x,ModuleType))
    return sorted(files)
