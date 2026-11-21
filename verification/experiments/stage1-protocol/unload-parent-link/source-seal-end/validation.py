"""Bind complete current captures and the unchanged complete parent evidence."""
import importlib.util
import json
from pathlib import Path
import re
import sys
import tarfile
from tempfile import TemporaryDirectory

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent / 'source-remove-retired'
sys.path.insert(0, str(PARENT))


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


capture = module('source_seal_end_capture', HERE / 'capture.py')
original_inputs = module('source_seal_end_parent_inputs', PARENT / 'inputs.py')
digest = original_inputs.digest
SUPPORT = original_inputs.SUPPORT
REPO = capture.retire.REPO
ROOT = 'controller.lifecycle_inbox.source_work.full.end.seal_end'
SEAL_END = 'sandbox_controller_core.capture.continuation.CaptureSealEnd'
ACCESSOR = 'sandbox_controller_core.model.EffectId.command'
LIBRARIES = {'core.option.Option.Insts.CoreCmpPartialEqOption.eq'}


def parsed(data):
    return original_inputs.parsed(data, capture.NAMESPACE)


def current(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    expected = dict(source=capture.retire.SOURCE, tree=capture.retire.TREE, variant=variant,
        options=capture.OPTIONS, driver=digest(HERE / 'capture.py'),
        base_driver=digest(PARENT / 'extract.py'), ports_driver=digest(PARENT / 'ports.py'),
        capture_root=capture.ROOT, direct_original_root=True, whole_receipt_proved=False,
        mutation=None if variant == 'original' else 'omit only original source-generation comparison',
        complete=True, public_acceptance=False)
    if any(result.get(name) != wanted for name, wanted in expected.items()):
        raise ValueError('the exact completed original seal-end capture is required')
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
        raise ValueError('all six exact staged files are mandatory')
    with TemporaryDirectory(prefix='source-seal-end-', dir=directory.parent) as temporary:
        source = Path(temporary)
        capture.reconstruct(directory / 'source.tar', source, variant)
        with tarfile.open(directory / 'source.tar') as archive:
            members = {m.name: m for m in archive.getmembers() if m.isfile()}
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
        raise ValueError('only the existing exact typed Option equality interface is permitted')
    return result, sources


def same_interfaces(correct, wrong):
    for name in ('Types', 'TypesExternal', 'External'):
        if correct[name] != wrong[name]:
            raise ValueError('all complete original type/library files must remain byte-identical: ' + name)
    types = parsed(correct['Types'])
    left, right = parsed(correct['Funs']), parsed(wrong['Funs'])
    if len(types) != 110 or len(left) != 7 or set(left) != set(right):
        raise ValueError('all 110 types and seven actual function declarations are required')
    if {name for name in left if left[name] != right[name]} != {ROOT}:
        raise ValueError('only the complete original seal_end body may change')
    omitted = ('      if generation != record.work.anchor.native.generation\n'
               '      then ok (core.result.Result.Err JournalError.CorruptStore)\n'
               '      else\n').encode()
    end = b'  | sandbox_controller_core.model.Command.CommitSnapshot '
    body = left[ROOT]
    if body.count(omitted) != 1 or body.count(end) != 1:
        raise ValueError('the exact emitted source-generation guard and following command branch are required')
    before, tail = body.split(omitted)
    nested, following = tail.split(end)
    if any(line and not line.startswith(b'  ') for line in nested.splitlines()):
        raise ValueError('only one emitted nested conditional level may be removed')
    reduced = b''.join(line[2:] if line.strip() else line for line in nested.splitlines(True))
    if before + reduced + end + following != right[ROOT]:
        raise ValueError('the actual wrong root must omit exactly source-generation equality')
    return dict(root=right[ROOT])


def parent(project, audit, retention, retention_audit, reference):
    plan, evidence = original_inputs.prior(project, audit, 'SourceRetirementQuestionProof', 136, 761)
    retained, retained_evidence = original_inputs.prior(retention, retention_audit, 'RetentionProviders', 26, 127)
    if (plan['source'], plan['tree']) != (capture.retire.SOURCE, capture.retire.TREE):
        raise ValueError('the fixed production identity must agree with the entire parent')
    if any(plan['files'].get(name) != wanted for name, wanted in retained['files'].items()):
        raise ValueError('the entire retention provider project must remain in its parent')
    pools = {
        'Types': ('SandproofSourceRetentionProviders/Types.lean', 'SourceRetireTypes.lean'),
        'Funs': ('SandproofSourceRetentionProviders/Funs.lean', 'SourceRetireEqualityView.lean', 'SourceRetireView.lean'),
    }
    expected_new = {'Types': {SEAL_END}, 'Funs': {ACCESSOR, ROOT}}
    matched, additional = {}, {}
    for group, files in pools.items():
        data = parsed(reference[group])
        sources = {name: (project / name).read_bytes() for name in files}
        matched[group] = {}
        additional[group] = {}
        for name, body in data.items():
            # The namespace views preserve each complete raw declaration
            # verbatim. Their exact enclosing modules and original whole-file
            # service evidence have already been checked above.
            found = [file for file, source in sources.items() if source.count(body) == 1]
            if found:
                matched[group][name] = found
            else:
                additional[group][name] = body
        if (len(matched[group]) != (109 if group == 'Types' else 5)
                or set(additional[group]) != expected_new[group]):
            raise ValueError('all common complete generated declarations must be unchanged: ' + group)
    return plan, dict(parent=evidence, retention=retained_evidence, shared=matched), additional


def declaration(reference, group, name, body):
    raw = reference[group]
    if raw.count(body) != 1:
        raise ValueError('one complete original generated declaration is required: ' + name)
    at = raw.index(body)
    prefix = raw[raw.rfind(b'\n\n/--', 0, at):at]
    return (b'@[reducible]\n' if re.search(rb'@\[reducible\]\s*$', prefix) else b'') + body
