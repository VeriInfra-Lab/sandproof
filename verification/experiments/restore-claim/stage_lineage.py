"""Materialize existing #770 providers for one downstream check."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil


PROVIDERS = Path('verification/experiments/create-transaction-link/current')
COMPONENTS = {
    'lineage': ('RestoreLineageUniqueness', (
        'controller-service/src/controller/lifecycle_inbox/model.rs',
        'sandbox-protocol/src/lifecycle/identity.rs',
        'sandbox-protocol/src/lifecycle/request.rs',
    )),
    'parent-transaction': ('RestoreTransaction', (
        'controller-service/src/controller/transaction.rs',
        'controller-service/src/controller/transaction/memo.rs',
        'controller-service/src/lib.rs',
    )),
}
IMPORT = re.compile(r'^\s*(?:(?:public|private|meta)\s+)?import\s+(.+)$', re.MULTILINE)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def materialize(project, component):
    name, production = COMPONENTS[component]
    target_module = 'SandboxModel.Lifecycle.Protocol.RestoreClaim.' + name
    own_proof = Path('verification/experiments/restore-claim') / f'{name}.lean.in'
    rust_inputs = ('Cargo.lock', *production)
    existing = project / PROVIDERS
    rust = json.loads((existing / 'rust-source-binding.json').read_text())
    for name in rust_inputs:
        if digest(project / name) != rust['files'][name]:
            raise ValueError(f'inherited Rust source changed: {name}')

    modules = json.loads((existing / 'materialization.json').read_text())['modules']
    expected = {}
    pending = [target_module]
    seen = set()
    while pending:
        module = pending.pop()
        if module in seen:
            continue
        seen.add(module)
        if module == target_module:
            entry = dict(path=target_module.replace('.', '/') + '.lean', source=own_proof)
        elif module in modules:
            entry = modules[module]
        else:
            # Lean/Lake resolves external packages and proof_batch checks the
            # complete local closure after materialization.
            continue
        source = project / entry['source']
        target = project / entry['path']
        sha = digest(source)
        if sha != entry.get('sha256', sha):
            raise ValueError(f'inherited Lean provider changed: {source}')
        target.parent.mkdir(parents=True, exist_ok=True)
        if target != source:
            shutil.copyfile(source, target)
        expected[entry['path']] = sha
        for imports in IMPORT.findall(source.read_text()):
            pending.extend(imports.split('--', 1)[0].split())

    shutil.copyfile(existing / 'lakefile.toml.in', project / 'lakefile.toml')
    return dict(inherited_rust_source=rust['source_head'],
                rust={name: rust['files'][name] for name in rust_inputs}, modules=expected)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('project', type=Path)
    parser.add_argument('receipt', type=Path)
    parser.add_argument('--component', choices=COMPONENTS, default='lineage')
    args = parser.parse_args()
    args.receipt.write_text(json.dumps(materialize(args.project, args.component), indent=2, sort_keys=True) + '\n')
