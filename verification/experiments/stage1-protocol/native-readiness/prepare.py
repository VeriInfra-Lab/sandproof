"""Add one transparent root and an exact mutation to private source only."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/native-readiness')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/readiness.rs'
TEST_ROOT = 'controller-service/src/controller/lifecycle_inbox/readiness/tests.rs'
VARIANTS = ('original', 'ignore-status', 'restored')
OLD = '&& operation.status == core::OperationStatus::Succeeded)'
NEW = '&& { let _ = operation.status; true })'
FAILURES = {'ignore-status': {'proof_native_completion_checks_original_fields'}}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inventory(root):
    data = json.loads((root/HERE/'inventory.json').read_text())
    for group in ('production_inputs', 'dependency_inputs'):
        for name, digest in data[group].items():
            if sha(root/name) != digest:
                raise ValueError('reviewed native completion input differs: '+name)
    for name, digest in data['auxiliary_inputs'].items():
        if sha(root/name) != digest:
            raise ValueError('reviewed root/primitive/native fixture differs: '+name)
    for name, provider in data['providers'].items():
        if sha(root/HERE/(name+'.lean.in')) != provider['sha256']:
            raise ValueError('reviewed proof provider differs: '+name)
    return data


def prepare(project, variant):
    if variant not in (*VARIANTS, 'correct'):
        raise ValueError('unknown completion variant: '+variant)
    inventory(project)
    path = project/SOURCE
    original = path.read_text()
    fragment = (project/HERE/'roots.rs.in').read_text()
    if 'fn proof_native_create_snapshot_completed(' in original or original.count(OLD) != 1:
        raise ValueError('original unique checker/root differs')
    text = original
    mutation = None
    if variant == 'ignore-status':
        text = text.replace(OLD, NEW, 1)
        mutation = dict(path=SOURCE, old=OLD, new=NEW,
                        original_sha256=sha(path), wrong_unformatted_sha256=hashlib.sha256(text.encode()).hexdigest())
    path.write_text(text.rstrip()+'\n\n'+fragment.strip()+'\n')
    # Formatting only this private projection does not follow/change children.
    subprocess.run(['rustfmt', '--edition', '2024', '--config', 'skip_children=true', SOURCE],
                   cwd=project, check=True)
    return {SOURCE: sha(path)}, dict(original_source_sha256=hashlib.sha256(original.encode()).hexdigest(),
        prepared_source_sha256=sha(path), mutation=mutation, production_modified=False,
        business_refactor=False, generated_bodies_modified=False,
        original_root_fragment_sha256=sha(project/HERE/'roots.rs.in'))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--project', type=Path, required=True)
    parser.add_argument('--variant', choices=(*VARIANTS, 'correct'), default='correct')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    changes, prepared = prepare(args.project.absolute(), args.variant)
    with args.output.open('x') as stream:
        json.dump(dict(changes=changes, preparation=prepared), stream, indent=2)
        stream.write('\n')
