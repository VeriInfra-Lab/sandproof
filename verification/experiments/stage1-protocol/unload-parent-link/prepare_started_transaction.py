"""Materialize the exact existing V3 transaction view and its new local proof."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
PROVIDER = ROOT / 'verification/experiments/create-transaction-link/current'
SOURCE = 'f26fc26a3fb6987a18ad69015c56bb99c0748e12'
RUST_FILES = (
    'Cargo.lock',
    'controller-core/src/model.rs',
    'controller-core/src/maintenance.rs',
    'controller-service/src/controller/transaction.rs',
    'controller-service/src/controller/transaction/memo.rs',
)
MODULE = 'UnloadStartedTransaction'
sys.path.insert(0, str(ROOT))
from tools.proof_batch import closure


def digest(data):
    return hashlib.sha256(data).hexdigest()


def materialize(output, packages):
    manifest = json.loads((PROVIDER / 'materialization.json').read_bytes())
    binding = json.loads((PROVIDER / 'rust-source-binding.json').read_bytes())['files']
    hashes = {}
    for name in RUST_FILES:
        source = subprocess.check_output(['git', 'show', f'{SOURCE}:{name}'], cwd=ROOT)
        expected = binding[name]
        if digest(source) != expected or (ROOT / name).read_bytes() != source:
            raise ValueError(f'original transaction input differs: {name}')
        hashes[name] = expected

    output.mkdir(mode=0o700)
    (output / '.lake').mkdir()
    (output / '.lake/packages').symlink_to(packages, target_is_directory=True)
    for name in ('lean-toolchain', 'lake-manifest.json'):
        shutil.copyfile(ROOT / name, output / name)
    view = (PROVIDER / 'lakefile.toml.in').read_text()
    (output / 'lakefile.toml').write_text(view + f'\n[[lean_lib]]\nname = "{MODULE}"\n')

    original = {}
    for entry in manifest['modules'].values():
        relative = entry['path']
        data = (ROOT / entry['source']).read_bytes()
        if digest(data) != entry['sha256']:
            raise ValueError(f'original generated file or provider differs: {relative}')
        path = output / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        original[relative] = entry['sha256']
    proof = MODULE + '.lean'
    data = (HERE / 'started-transaction' / (proof + '.in')).read_bytes()
    (output / proof).write_bytes(data)
    reachable = closure(output, [proof], active_view='lakefile.toml')
    if any(value != original.get(name) for name, value in reachable.items() if name != proof):
        raise ValueError('unexpected reachable Lean provider')
    plan = dict(source=SOURCE, rust=hashes, root=MODULE, files=reachable)
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(f'{MODULE}: {len(reachable)} exact Lean inputs')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lake-packages', type=Path, required=True)
    args = parser.parse_args()
    materialize(args.output.resolve(), args.lake_packages.resolve(strict=True))
