"""Materialize the pinned SQL and retention proofs around unchanged joint output."""

import json
from pathlib import Path
import re
import tarfile
import tempfile

from joint_capture import admit
from joint_inputs import HERE, MODES, NAMESPACE, ROOT, ROW_PARENT, configuration, retention, stage
import prepare as parent

ALIGNED = ('Types.lean', 'Funs.lean')
ROOTS = {'correct': 'JointComposition', 'binding-only': 'JointBindingOnly'}
OLD_ROW_NS = 'SandproofSourceRowSqlSelect'


def checked(folder, mode, row):
    receipt = json.loads((folder / 'result.json').read_bytes())
    drivers = {name: retention.sha(HERE / name) for name in ('joint_capture.py', 'joint_inputs.py', 'bridge.rs.in')}
    if (receipt['complete'] is not True or receipt['public_acceptance'] is not False
            or (receipt['mode'], receipt['source'], receipt['configuration'], receipt['driver'])
            != (mode, retention.SOURCE, configuration(row, mode), drivers)
            or [(item['label'], item['exit_code'], item['bounded_stop']) for item in receipt['commands']]
            != [('charon', 0, False), ('aeneas', 0, False)]):
        raise ValueError('complete original capture is required')
    for name, digest in [('source.tar', retention.SOURCE_ARCHIVE), ('actual.llbc', receipt['ir']), *[
            (entry['label'] + '.log', entry['log_sha256']) for entry in receipt['commands']]]:
        if retention.sha(folder / name) != digest:
            raise ValueError('original capture input or evidence changed: ' + name)
    with tempfile.TemporaryDirectory(dir=folder.parent, prefix='joint-verify-') as temp:
        expected = Path(temp)
        with tarfile.open(folder / 'source.tar') as archive:
            archive.extractall(expected, filter='data')
        stage(row, expected, mode)
        source = folder / 'source'
        names = {path.relative_to(expected) for path in expected.rglob('*') if path.is_file()}
        if names != {path.relative_to(source) for path in source.rglob('*') if path.is_file()}:
            raise ValueError('actual Rust inventory changed')
        for name in names:
            if (source / name).is_symlink() or (source / name).read_bytes() != (expected / name).read_bytes():
                raise ValueError('actual complete Rust input changed: ' + str(name))
    generated = folder / 'generated'
    names = {'translation.json', *(f'{NAMESPACE}/{name}' for name in (*ALIGNED, 'TypesExternal_Template.lean', 'FunsExternal_Template.lean'))}
    found = {str(path.relative_to(generated)): retention.sha(path) for path in generated.rglob('*') if path.is_file()}
    if found.keys() != names or found != receipt['generated']:
        raise ValueError('actual generated files changed')
    target = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())['target']
    if admit(row, folder / 'actual.llbc', generated, mode, target) != receipt['admission']:
        raise ValueError('actual root admission changed')
    return {name: (generated / NAMESPACE / name).read_bytes() for name in ALIGNED}


def declarations(data, name):
    # This is a byte comparison of declarations. Only the module namespace
    # is normalized, and these normalized bytes are never supplied to Lean.
    return {key: value.removesuffix(b'\n\nend ' + name.encode() + b'\n') for key, value in parent.declarations(data).items()}


def correspondence(views, reference):
    matches = {}
    for filename in ALIGNED:
        good, wrong, whole = (declarations(views[mode][filename], NAMESPACE) for mode in MODES)
        reference_defs = declarations(reference[filename], OLD_ROW_NS)
        if not reference_defs or any(good.get(name) != body for name, body in reference_defs.items()):
            raise ValueError('original SQL provider definitions differ: ' + filename)
        if not good or good.keys() != wrong.keys() or any(whole.get(name) != body for name, body in good.items()):
            raise ValueError('joint definitions must occur unchanged in the actual parent')
        differences = [key for key in good if good[key] != wrong[key]]
        changed = 'controller.lifecycle_inbox.source_work.codec.retains_record_using_loop.body'
        old = b'SavedRecordChecksInst.same_work next record'
        new = b'SavedRecordChecksInst.same_binding next record'
        if filename == 'Types.lean':
            if differences:
                raise ValueError('binding-only view must preserve all actual generated types')
        elif differences != [changed] or good[changed].count(old) != 1 or good[changed].replace(old, new) != wrong[changed]:
            raise ValueError('binding-only view must change exactly the original comparison')
        matches[filename] = {'original_row': len(reference_defs), 'shared_parent': len(good)}
    return matches


def materialize(project, mode, row_model, generated, packages, service, matches):
    from prepare_world_begin import closure
    project.mkdir(mode=0o700)

    def put(name, data):
        destination = project / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        with destination.open('xb') as stream:
            stream.write(data if isinstance(data, bytes) else data.encode())

    for local, target in (('.lake/packages', packages), ('.tools/lean-mcp', service)):
        link = project / local
        link.parent.mkdir()
        link.symlink_to(target, target_is_directory=True)
    plan = json.loads((row_model / 'plan.json').read_bytes())
    for name, digest in plan['files'].items():
        original = row_model / name
        if retention.sha(original) != digest:
            raise ValueError('original SQL proof provider changed')
        if name in {OLD_ROW_NS + '/' + file for file in (*ALIGNED, 'FunsExternal.lean')}:
            continue
        put(name.replace(OLD_ROW_NS, NAMESPACE), original.read_bytes().replace(OLD_ROW_NS.encode(), NAMESPACE.encode()))
    for name, data in generated.items():
        put(f'{NAMESPACE}/{name}', data)
    work = parent.template('source-work-match', 'FunsExternal')
    facts = parent.template('source-preservation', 'FunsExternal')
    option_eq = 'def core.option.Option.Insts.CoreCmpPartialEqOption.eq '
    callback = 'def core.option.Option.is_some_and '
    end = 'end ' + retention.NAMESPACE
    if not work[work.index(option_eq):].startswith(facts[facts.index(option_eq):facts.index(end)]):
        raise ValueError('the ordinary Option provider must agree')
    work = work.replace(option_eq, facts[facts.index(callback):facts.index(option_eq)] + option_eq)
    a6 = work.split('namespace ' + retention.NAMESPACE + '\n', 1)[1].rsplit(end, 1)[0]
    ext = (row_model / OLD_ROW_NS / 'FunsExternal.lean').read_text().replace(OLD_ROW_NS, NAMESPACE)
    put(f'{NAMESPACE}/FunsExternal.lean', ext[:ext.rindex('end ' + NAMESPACE)] + a6 + '\nend ' + NAMESPACE + '\n')
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        put(name, (row_model / name).read_bytes())
    for component, namespace, modules in (
        ('source-work-match', 'SourceWorkMatch', ('Spec', 'RequestEquality', 'ScalarEquality', 'Containers', 'Values',
            'ContainerInstances', 'CoreData', 'CoreEffects', 'AnchorData', 'AnchorEquality', 'SourceData', 'Core')),
        ('source-preservation', 'SourcePreservation', ('Spec', 'Bytes', 'Equality', 'StateEquality', 'Core')),
    ):
        for module in modules:
            text = parent.template(component, module).replace(retention.NAMESPACE, NAMESPACE)
            put(f'SandboxModel/Lifecycle/Protocol/{namespace}/{module}.lean', text)
    put('SandboxModel/Lifecycle/Protocol/ReplayContract.lean', (ROOT / 'SandboxModel/Lifecycle/Protocol/ReplayContract.lean').read_bytes())
    imports = [('FirstMatchInputs', 'first-match'), ('RetentionProviderCases', 'retention-providers')]
    imports += [('SourceFirstMatch', 'first-match'), ('RetentionProviders', 'retention-providers')] if mode == 'correct' else [
        ('RetentionProvidersBindingOnly', 'retention-providers')]
    for name, folder in imports:
        text = retention.provider(retention.OLD_PACKAGE / folder / (name + '.lean.in')).decode()
        put(name + '.lean', text.replace('SandproofSourceFirstMatch', NAMESPACE).replace('SandproofSourceRetentionProviders', NAMESPACE))

    def saved(name):
        return retention.pinned(retention.SAVE_PARENT, retention.SAVE_PACKAGE / 'save-order' / (name + '.lean.in')).decode().replace(
            'SandproofSourceSaveOrder', NAMESPACE)

    inputs = saved('SaveOrderInputs')
    put('SaveOrderInputs.lean', inputs[:inputs.index('-- The source-save view')] + inputs[inputs.index('-- A legal empty'):inputs.index('#audit_controller_proof')])
    put('SourceSaveOrder.lean', saved('SourceSaveOrder'))
    prefix = retention.pinned(ROW_PARENT, retention.OLD_PACKAGE / 'save-row-list/SaveRowsGenerated.lean.in').decode()
    aliases = prefix[prefix.index('abbrev runLoop'):prefix.index('structure ReadEffects')]
    put('SaveRowsGenerated.lean', (prefix[:prefix.index('local notation "JournalError"')] + aliases +
        'end SandboxModel.Stage1Protocol.SaveRows\n').replace(OLD_ROW_NS, NAMESPACE))
    proof = retention.pinned(ROW_PARENT, retention.OLD_PACKAGE / 'save-row-list/SaveRows.lean.in').decode()
    body = proof[:proof.index('variable {C N R Keys : Type}')].replace('open Aeneas Aeneas.Std\n', f'open Aeneas Aeneas.Std {NAMESPACE}\n', 1)
    audits = re.findall(r'^#audit_controller_proof .*$', proof, re.M)[:3]
    put('SaveRows.lean', body + 'end SandboxModel.Stage1Protocol.SaveRows\n\n' + '\n'.join(audits) + '\n')
    root = ROOTS[mode]
    for name in ('JointSpec', root):
        put(name + '.lean', (HERE / (name + '.lean.in')).read_bytes())
    lake = (row_model / 'lakefile.toml').read_text().replace(OLD_ROW_NS, NAMESPACE)
    for name in (*[name for name, _ in imports], 'SaveOrderInputs', 'SourceSaveOrder', 'SaveRowsGenerated', 'SaveRows', 'JointSpec', root):
        lake += f'\n[[lean_lib]]\nname = "{name}"\n'
    put('lakefile.toml', lake)
    files = closure(project, [root + '.lean'], active_view='lakefile.toml')
    audits = sorted({match for name in files for match in re.findall(
        r'^#audit_controller_proof (\S+)$', (project / name).read_text(), re.M)})
    report = dict(root=root, source=retention.SOURCE, row_provider=ROW_PARENT, mode=mode, matching_parent_declarations=matches,
                  files=files, strict_files=sorted(files), modules={name: {'sha256': value} for name, value in files.items()},
                  build_order=[name.removesuffix('.lean').replace('/', '.') for name in files], audits=audits, public_acceptance=False)
    retention.write_json(project / 'plan.json', report)
    return report
