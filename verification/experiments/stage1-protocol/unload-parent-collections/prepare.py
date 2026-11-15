"""Materialize an exact parent closure plus real collection outputs.

First run unload-parent-link/prepare.py into a separate parent project. The
parent is checked against its original frozen inventory, never hash-migrated.
The mutant imports the same complete contract in a separate, smaller project.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def materialize(parent, output, variant):
    upstream = HERE.parent / 'unload-parent-link'
    frozen = json.loads((upstream / 'input-closure.json').read_bytes())
    for name, expected in frozen['repository_inputs'].items():
        path = ROOT / name
        if path.is_symlink() or not path.is_file() or sha(path.read_bytes()) != expected:
            raise ValueError('original parent input changed: ' + name)
    plan = json.loads((parent / 'plan.json').read_bytes())
    actual = {name: sha((parent / name).read_bytes()) for name in plan['modules']}
    if (actual != frozen['materialized_modules'] or plan['roots'] != ['UnloadParentLink']
            or any(actual[name] != entry['sha256'] for name, entry in plan['modules'].items())):
        raise ValueError('parent project is not the exact original materialized closure')
    spec = importlib.util.spec_from_file_location('parent_capture_check', upstream / 'check_capture.py')
    checker = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(checker)
    files = {name: (parent / name).read_bytes() for name in plan['modules']}
    provenance = {name: list(entry['source']) for name, entry in plan['modules'].items()}
    raw = ROOT / 'verification/aeneas/stage1-protocol/unload-parent-collections' / variant / 'SandproofUnloadFirstCollections'
    for name in ('Types.lean', 'Funs.lean'):
        target = 'SandproofUnloadFirstCollections/' + name
        data = (raw / name).read_bytes()
        if name == 'Types.lean' and data != files[target]:
            raise ValueError('collection value types differ from the original carrier')
        if name == 'Funs.lean' and variant == 'correct' and checker.tokens(data) != checker.tokens(files[target]):
            raise ValueError('original arbitrary-list collection proofs no longer apply')
        files[target] = data
        provenance[target].append(str((raw / name).relative_to(ROOT)))
    for source in sorted(HERE.glob('*.lean.in')):
        name = source.name.removesuffix('.in')
        if name in files:
            raise ValueError('new proof shadows a parent module: ' + name)
        files[name] = source.read_bytes()
        provenance[name] = [str(source.relative_to(ROOT))]
    roots = ['UnloadCollectionLink' if variant == 'correct' else 'CollectionCounter']
    seen, active, order = set(), set(), []

    def visit(module):
        if module.split('.')[0] in {'Aeneas', 'Mathlib', 'Init', 'Lean', 'Std', 'Batteries'} or module in seen:
            return
        name = module.replace('.', '/') + '.lean'
        if module in active or name not in files:
            raise ValueError('missing or cyclic module: ' + module)
        active.add(module)
        for statement in re.findall(rb'(?m)^(?:public |private )?import[ \t]+([^\n]+)', files[name]):
            for dependency in statement.split(b'--', 1)[0].split():
                visit(dependency.decode())
        active.remove(module)
        seen.add(module)
        order.append(module)

    for root in roots:
        visit(root)
    files = {name: data for name, data in files.items() if name.removesuffix('.lean').replace('/', '.') in seen}
    own_frozen = HERE / 'input-closure.json'
    if own_frozen.exists():
        inventory = json.loads(own_frozen.read_bytes())
        if sha((upstream / 'input-closure.json').read_bytes()) != inventory['parent_freeze_sha256']:
            raise ValueError('the exact original parent freeze changed')
        for name, expected in inventory['repository_inputs'].items():
            if sha((ROOT / name).read_bytes()) != expected:
                raise ValueError('collection input closure changed: ' + name)
        if {name: sha(data) for name, data in files.items()} != inventory['variants'][variant]['materialized_modules']:
            raise ValueError('collection materialized closure changed')
    output.mkdir(mode=0o700)
    for name, data in files.items():
        target = output / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'tools/proof_services.py', 'scripts/lean-mcp-server.sh'):
        target = output / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes((parent / name).read_bytes())
    lake = 'name = "unload-parent-collections-' + variant + '"\n\n[[require]]\nname = "aeneas"\ngit = { url = "https://github.com/AeneasVerif/aeneas", subDir = "backends/lean" }\nrev = "505b6ca35217e7be5c96c3e2f8045edfbdf47291"\n'
    lake += ''.join('\n[[lean_lib]]\nname = "' + head + '"\n' for head in sorted({name.split('.')[0] for name in seen}))
    (output / 'lakefile.toml').write_text(lake)
    (output / '.lake').mkdir()
    (output / '.lake/packages').symlink_to((ROOT / '.lake/packages').resolve(strict=True), target_is_directory=True)
    (output / '.tools').mkdir()
    (output / '.tools/lean-mcp').symlink_to((ROOT / '.tools/lean-mcp').resolve(strict=True), target_is_directory=True)
    (output / 'artifacts').mkdir()
    (output / 'plan.json').write_text(json.dumps(dict(schema=1, development_only=True, public_acceptance=False,
        variant=variant, parent_plan_sha256=sha((parent / 'plan.json').read_bytes()), roots=roots, build_order=order,
        modules={name: dict(sha256=sha(data), source=provenance[name]) for name, data in files.items()}), indent=2) + '\n')
    print(f'{variant}: {len(order)} actual reachable files; development only')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--parent-project', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--variant', choices=('correct', 'attempt-zero'), required=True)
    args = parser.parse_args()
    materialize(args.parent_project.resolve(strict=True), args.output.resolve(), args.variant)
