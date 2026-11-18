"""Use one unchanged generated view for the scan and both existing provider proofs."""
import argparse
import json
from pathlib import Path
import re
import shutil

from prepare_first_match import captured, steps
from prepare_world_begin import closure, read_exact
from source_retention_providers import (
    ARCHIVE, CODEC, CODEC_DIGESTS, HERE, NAMESPACE, PROBE_DIGEST, PROBE_SOURCE, PROVIDERS, ROOT, SOURCE, TREE, admit, digest,
)


CONFIGURATION = {
    'source-work-match': ('SourceWorkMatch', ('Spec', 'RequestEquality', 'Containers', 'ScalarEquality', 'Values')),
    'source-preservation': ('SourcePreservation', ('Spec', 'Bytes', 'Equality', 'StateEquality', 'Core')),
}
SUPPORT = ('SandboxModel/ProofAuditCommand.lean', 'SandboxModel/Lifecycle/Protocol/ReplayContract.lean')
FILES = ('Types.lean', 'Funs.lean')
ROOTS = {'providers': 'RetentionProviders', 'binding-only': 'RetentionProvidersBindingOnly'}
FULL_WORK_MODULES = ('ContainerInstances', 'CoreData', 'CoreEffects', 'AnchorData', 'AnchorEquality', 'SourceData', 'Core')
CASES = 'RetentionProviderCases'
PREFIX = Path('SandboxModel/Lifecycle/Protocol')


def generated_view(capture, mode):
    report = json.loads((capture / 'result.json').read_bytes())
    driver = HERE.parent / 'source_retention_providers.py'
    if (report['complete'] is not True or report['public_acceptance'] is not False
            or (report['mode'], report['source_commit'], report['source_tree'], report['driver_sha256'])
            != (mode, SOURCE, TREE, digest(driver))):
        raise ValueError('completed exact capture receipt required')
    commands = report['commands']
    if [(r['label'], r['exit_code'], r['bounded_stop']) for r in commands] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both fixed extraction invocations must complete')
    read_exact(capture / 'source.tar', ARCHIVE)
    for command in commands:
        read_exact(capture / (command['label'] + '.log'), command['log_sha256'])
    expected = CODEC_DIGESTS[mode if mode == 'binding-only' else 'providers']
    if (report['codec_sha256'], report['comparison_probe_sha256']) != (expected, PROBE_DIGEST):
        raise ValueError('fixed Rust inputs differ')
    read_exact(capture / 'source' / CODEC, expected)
    read_exact(capture / 'source' / PROBE_SOURCE, PROBE_DIGEST)
    ir = capture / 'actual.llbc'
    read_exact(ir, report['ir_sha256'])
    generated = capture / 'generated'
    targets = {'translation.json', *(f'{NAMESPACE}/{name}' for name in (*FILES, 'FunsExternal_Template.lean'))}
    if mode == 'parent':
        targets.add(f'{NAMESPACE}/TypesExternal_Template.lean')
    found = {str(file.relative_to(generated)) for file in generated.rglob('*') if file.is_file()}
    if found != targets or found != report['generated_sha256'].keys():
        raise ValueError('actual generated source inventory changed')
    for name in found:
        read_exact(generated / name, report['generated_sha256'][name])
    target = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())['target']
    if admit(ir, generated, target, mode) != report['admission']:
        raise ValueError('actual caller or provider admission changed')
    return report, {name: (generated / NAMESPACE / name).read_bytes() for name in FILES}


def declarations(data):
    bodies = {}
    for block in data.split(b'\n\n/--'):
        match = re.search(rb'^(?:def|impl_def|structure|inductive)\s+([^\s(:]+)', block, re.M)
        if match is not None:
            name = match[1].decode()
            if name in bodies:
                raise ValueError('duplicate generated declaration: ' + name)
            bodies[name] = block[match.start():].removesuffix(b'\n\nend ' + NAMESPACE.encode() + b'\n')
    return bodies


def checked_template(component, module, inventory):
    original = PROVIDERS / component / (module + '.lean.in')
    recorded = inventory['templates'][str(original.relative_to(ROOT))]
    return read_exact(original, recorded).decode()


def materialize(mode, generic, parent, baseline, output, packages, mcp):
    root = ROOTS[mode]
    report, bodies = generated_view(generic, mode)
    parent_report, parent_bodies = generated_view(parent, 'parent')
    first_match, reference = captured(baseline, 'binding-only' if mode == 'binding-only' else 'factored')
    if steps(bodies['Funs.lean']) != steps(first_match['Funs.lean']):
        raise ValueError('the same six original first-match bodies are required')
    matched = {}
    for name in FILES:
        inner = declarations(bodies[name])
        actual = declarations(parent_bodies[name])
        if mode == 'binding-only' and name == 'Funs.lean':
            loop, = (key for key in inner if key.endswith('.retains_record_using_loop.body'))
            original = b'SavedRecordChecksInst.same_work next record'
            if actual[loop].count(original) != 1:
                raise ValueError('expected one full comparison in the actual parent')
            actual[loop] = actual[loop].replace(original, b'SavedRecordChecksInst.same_binding next record')
        different = [key for key, value in inner.items() if actual.get(key) != value]
        if not inner or different:
            raise ValueError('provider bodies differ in original parent: ' + ', '.join(different))
        matched[name] = len(inner)

    output.mkdir(mode=0o700)
    for name, dependency in (('.lake/packages', packages), ('.tools/lean-mcp', mcp)):
        destination = output / name
        destination.parent.mkdir()
        destination.symlink_to(dependency, target_is_directory=True)
    for name in (*SUPPORT, 'tools/lean_session.py', 'scripts/lean-mcp-server.sh', 'lean-toolchain', 'lake-manifest.json'):
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / name, destination)
    with (output / 'lakefile.toml').open('x') as file:
        dependency, = re.findall(r'\[\[require\]\]\nname = "aeneas"\n[^\n]+\n[^\n]+\n',
                                  (ROOT / 'lakefile.toml').read_text())
        file.write('name = "source-retention-providers"\n' + dependency)
        libraries = ('SandboxModel', NAMESPACE, 'FirstMatchInputs', CASES, root)
        if mode == 'providers':
            libraries += ('SourceFirstMatch',)
        for library in libraries:
            file.write(f'\n[[lean_lib]]\nname = "{library}"\n')
    (output / NAMESPACE).mkdir()
    for name, data in bodies.items():
        (output / NAMESPACE / name).write_bytes(data)
    external = {}
    installed = set()
    for component, (namespace, modules) in CONFIGURATION.items():
        if component == 'source-work-match' and mode == 'providers':
            modules += FULL_WORK_MODULES
        inventory = json.loads((PROVIDERS / component / 'inventory.json').read_bytes())
        old = 'Sandproof' + namespace
        def bind_generated(text):
            return text.replace(old + 'Rust', NAMESPACE).replace(old + '.', NAMESPACE + '.')
        for module in modules:
            name = PREFIX / namespace / (module + '.lean')
            destination = output / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(bind_generated(checked_template(component, module, inventory)))
            installed.add(str(name))
        value = bind_generated(checked_template(component, 'FunsExternal', inventory))
        external[component] = value
    work = external['source-work-match']
    facts = external['source-preservation']
    callback = 'def core.option.Option.is_some_and '
    option_eq = 'def core.option.Option.Insts.CoreCmpPartialEqOption.eq '
    common = facts[facts.index(option_eq):facts.index('end ' + NAMESPACE)]
    if not work[work.index(option_eq):].startswith(common):
        raise ValueError('previous ordinary Option equality models must agree')
    fragment = facts[facts.index(callback):facts.index(option_eq)]
    (output / NAMESPACE / 'FunsExternal.lean').write_text(work.replace(option_eq, fragment + option_eq))
    scans = ('FirstMatchInputs', 'SourceFirstMatch') if mode == 'providers' else ('FirstMatchInputs',)
    for name in scans:
        template = (HERE.parent / 'first-match' / (name + '.lean.in')).read_text()
        (output / (name + '.lean')).write_text(template.replace('SandproofSourceFirstMatch', NAMESPACE))
    for name in (CASES, root):
        shutil.copyfile(HERE / (name + '.lean.in'), output / (name + '.lean'))
    expected = installed | set(SUPPORT) | {f'{NAMESPACE}/{name}' for name in (*FILES, 'FunsExternal.lean')}
    expected |= {name + '.lean' for name in (*scans, CASES, root)}
    files = closure(output, [root + '.lean'], active_view='lakefile.toml')
    if files.keys() != expected:
        raise ValueError('unexpected imports or missing physical files in provider closure')
    plan = dict(root=root, mode=mode, source=SOURCE, matching_parent_declarations=matched, first_match=reference,
                generated=report['generated_sha256'], parent=parent_report['generated_sha256'], files=files,
                strict_files=sorted(expected), modules={name: {'sha256': sha} for name, sha in files.items()},
                build_order=[name.removesuffix('.lean') for name in files])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(root, len(files), 'exact Lean inputs;', matched, 'parent declarations matched')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--mode', choices=ROOTS, required=True)
    for name in ('generic', 'parent', 'baseline', 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    materialize(args.mode, args.generic.resolve(strict=True), args.parent.resolve(strict=True), args.baseline.resolve(strict=True),
                args.output.resolve(), args.lake_packages.resolve(strict=True), args.lean_mcp.resolve(strict=True))
