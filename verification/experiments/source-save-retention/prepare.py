"""Materialize unchanged generated files and the existing proof providers."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import tarfile
import tempfile

import inputs as v
import capture

ROOTS = {'correct': 'SaveRetentionComposition', 'binding-only': 'SaveRetentionBindingOnly'}
FILES = ('Types.lean', 'Funs.lean')


def checked(folder, mode):
    result = json.loads((folder / 'result.json').read_bytes())
    drivers = {p.name: v.sha(p) for p in (v.HERE / 'capture.py', v.HERE / 'inputs.py', v.HERE / 'bridge.rs.in')}
    if (result.get('complete') is not True or result.get('public_acceptance') is not False
            or (result.get('mode'), result.get('source'), result.get('driver')) != (mode, v.SOURCE, drivers)
            or [(c['label'], c['exit_code'], c['bounded_stop']) for c in result['commands']]
            != [('charon', 0, False), ('aeneas', 0, False)]):
        raise ValueError('a complete current capture receipt is required')
    if v.sha(folder / 'source.tar') != v.SOURCE_ARCHIVE or v.sha(folder / 'actual.llbc') != result['ir']:
        raise ValueError('the actual source archive or MIR changed')
    for command in result['commands']:
        if v.sha(folder / (command['label'] + '.log')) != command['log_sha256']:
            raise ValueError('the original extraction log changed')
    with tempfile.TemporaryDirectory(prefix='save-retention-stage-', dir=folder.parent) as temporary:
        expected = Path(temporary)
        for name in (v.CODEC, v.LIB):
            destination = expected / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(v.pinned(v.SOURCE, name))
        rust = v.stage(expected, mode)
        if rust != result['rust']:
            raise ValueError('the actual staged Rust differs from the current adapter')
        with tarfile.open(folder / 'source.tar') as archive:
            original = {member.name: member for member in archive if member.isfile()}
            physical = {str(p.relative_to(folder / 'source')) for p in (folder / 'source').rglob('*') if p.is_file()}
            if physical != original.keys():
                raise ValueError('the complete captured Rust source inventory changed')
            for name, member in original.items():
                path = folder / 'source' / name
                data = (expected / name).read_bytes() if name in rust else archive.extractfile(member).read()
                if path.is_symlink() or path.read_bytes() != data:
                    raise ValueError('captured source changed: ' + name)
    generated = folder / 'generated'
    physical = {str(p.relative_to(generated)): v.sha(p) for p in generated.rglob('*') if p.is_file()}
    expected_names = {'translation.json', *(str(Path(v.NAMESPACE) / name) for name in
                      (*FILES, 'TypesExternal_Template.lean', 'FunsExternal_Template.lean'))}
    if physical.keys() != expected_names or physical != result['generated']:
        raise ValueError('the full generated output inventory changed')
    if capture.admit(folder / 'actual.llbc', generated, mode) != result['admission']:
        raise ValueError('the actual source admission changed')
    return {name: (generated / v.NAMESPACE / name).read_bytes() for name in FILES}


def declarations(data):
    result = {}
    for block in data.split(b'\n\n/--'):
        match = re.search(rb'^(?:def|impl_def|structure|inductive)\s+([^\s(:]+)', block, re.M)
        if match:
            name = match[1].decode()
            if name in result:
                raise ValueError('duplicate generated declaration: ' + name)
            result[name] = block[match.start():].removesuffix(b'\n\nend ' + v.NAMESPACE.encode() + b'\n')
    return result


def correspondence(positive, wrong, parent):
    matches = {}
    for name in FILES:
        good, bad, whole = (declarations(view[name]) for view in (positive, wrong, parent))
        if not good or good.keys() != bad.keys() or any(whole.get(key) != value for key, value in good.items()):
            raise ValueError('all shared positive generated declarations must occur unchanged in the parent')
        changed = [key for key in good if good[key] != bad[key]]
        expected = [] if name == 'Types.lean' else [
            'controller.lifecycle_inbox.source_work.codec.retains_record_using_loop.body']
        if changed != expected:
            raise ValueError('the real mutant must change only the inner full-match selection')
        if changed:
            old = b'SavedRecordChecksInst.same_work next record'
            new = b'SavedRecordChecksInst.same_binding next record'
            body = good[changed[0]]
            if body.count(old) != 1 or body.replace(old, new) != bad[changed[0]]:
                raise ValueError('the actual mutant must replace exactly the complete comparison')
        matches[name] = len(good)
    return matches


def template(component, module):
    name = v.STAGE1 / component / (module + '.lean.in')
    data = v.provider(name)
    inventory = json.loads(v.provider(v.STAGE1 / component / 'inventory.json'))
    if hashlib.sha256(data).hexdigest() != inventory['templates'][str(name)]:
        raise ValueError('the original provider template differs: ' + str(name))
    namespace = {'source-work-match': 'SourceWorkMatch', 'source-preservation': 'SourcePreservation'}[component]
    return data.decode().replace('Sandproof' + namespace + 'Rust', v.NAMESPACE).replace(
        'Sandproof' + namespace + '.', v.NAMESPACE + '.')


def materialize(args):
    positive = checked(args.captures / 'correct', 'correct')
    wrong = checked(args.captures / 'binding-only', 'binding-only')
    parent = checked(args.captures / 'parent', 'parent')
    matched = correspondence(positive, wrong, parent)
    v.helpers()
    from prepare_started_transaction import materialize as base
    from prepare_world_begin import closure
    base(args.output, args.lake_packages)
    project = args.output
    (project / '.tools').mkdir()
    (project / '.tools/lean-mcp').symlink_to(args.lean_mcp, target_is_directory=True)
    for name in ('tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        destination = project / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(v.ROOT / name, destination)
    destination = project / v.NAMESPACE
    destination.mkdir()
    for name, data in (positive if args.mode == 'correct' else wrong).items():
        (destination / name).write_bytes(data)
    (destination / 'TypesExternal.lean').write_text(
        'import SandproofStage1Request.Types\n\nnamespace ' + v.NAMESPACE + '\n\n' +
        'abbrev core.io.error.Error := SandproofSqliteRust.core.io.error.Error\n' +
        'abbrev rusqlite.error.Error := SandproofSqliteRust.rusqlite.error.Error\n' +
        'abbrev serde_json.error.Error := SandproofSqliteRust.serde_json.error.Error\n\nend ' + v.NAMESPACE + '\n')
    components = {
        'source-work-match': ('SourceWorkMatch', ('Spec', 'RequestEquality', 'Containers', 'ScalarEquality', 'Values',
            'ContainerInstances', 'CoreData', 'CoreEffects', 'AnchorData', 'AnchorEquality', 'SourceData', 'Core')),
        'source-preservation': ('SourcePreservation', ('Spec', 'Bytes', 'Equality', 'StateEquality', 'Core')),
    }
    external = {}
    for component, (namespace, modules) in components.items():
        for module in modules:
            path = project / 'SandboxModel/Lifecycle/Protocol' / namespace / (module + '.lean')
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(template(component, module))
        external[component] = template(component, 'FunsExternal')
    work, facts = external['source-work-match'], external['source-preservation']
    option = 'def core.option.Option.Insts.CoreCmpPartialEqOption.eq '
    callback = 'def core.option.Option.is_some_and '
    common = facts[facts.index(option):facts.index('end ' + v.NAMESPACE)]
    if not work[work.index(option):].startswith(common):
        raise ValueError('the original ordinary Option equality models must agree')
    fragment = facts[facts.index(callback):facts.index(option)]
    (destination / 'FunsExternal.lean').write_text(work.replace(option, fragment + option))
    modules = ['FirstMatchInputs', 'RetentionProviderCases']
    modules += ['SourceFirstMatch', 'RetentionProviders'] if args.mode == 'correct' else ['RetentionProvidersBindingOnly']
    for module in modules:
        folder = 'first-match' if module in ('FirstMatchInputs', 'SourceFirstMatch') else 'retention-providers'
        text = v.provider(v.OLD_PACKAGE / folder / (module + '.lean.in')).decode()
        text = text.replace('SandproofSourceFirstMatch', v.NAMESPACE).replace('SandproofSourceRetentionProviders', v.NAMESPACE)
        (project / (module + '.lean')).write_text(text)
    original = (v.ROOT / v.SAVE_PACKAGE / 'save-order/SaveOrderInputs.lean.in').read_text()
    # The cross-view RequestError transport remains in #1032's original project.
    # This view uses one concrete JournalError and needs only its generic inputs.
    prefix = original[:original.index('-- The source-save view')]
    witness = original[original.index('-- A legal empty'):original.index('#audit_controller_proof')]
    (project / 'SaveOrderInputs.lean').write_text((prefix + witness).replace('SandproofSourceSaveOrder', v.NAMESPACE))
    original = (v.ROOT / v.SAVE_PACKAGE / 'save-order/SourceSaveOrder.lean.in').read_text()
    (project / 'SourceSaveOrder.lean').write_text(original.replace('SandproofSourceSaveOrder', v.NAMESPACE))
    root = ROOTS[args.mode]
    for module in ('SaveRetentionSpec', root):
        shutil.copyfile(v.HERE / (module + '.lean.in'), project / (module + '.lean'))
    libraries = (v.NAMESPACE, *modules, 'SaveOrderInputs', 'SourceSaveOrder', 'SaveRetentionSpec', root)
    with (project / 'lakefile.toml').open('a') as file:
        for library in libraries:
            file.write('\n[[lean_lib]]\nname = "' + library + '"\n')
    files = closure(project, [root + '.lean'], active_view='lakefile.toml')
    audits = sorted({symbol for name in files for symbol in re.findall(
        r'^#audit_controller_proof (\S+)$', (project / name).read_text(), re.M)})
    plan = dict(root=root, mode=args.mode, source=v.SOURCE, save_parent=v.SAVE_PARENT, providers=v.PROVIDERS,
                matching_parent_declarations=matched, files=files, strict_files=sorted(files),
                modules={name: {'sha256': value} for name, value in files.items()},
                build_order=[name.removesuffix('.lean').replace('/', '.') for name in files], audits=audits,
                public_acceptance=False)
    v.write_json(project / 'plan.json', plan)
    print(root, len(files), 'actual Lean files;', len(audits), 'unique audits')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=ROOTS, required=True)
    for name in ('captures', 'output', 'lake-packages', 'lean-mcp'):
        parser.add_argument('--' + name, type=Path, required=True)
    materialize(parser.parse_args())
