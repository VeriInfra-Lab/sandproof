"""Assemble the reviewed real-source caller view; never replace a business body."""
import argparse
import importlib.util
import json
from pathlib import Path

HERE = Path('verification/experiments/stage1-protocol/protocol-record')
ROOT = Path(__file__).resolve().parents[4]
SOURCE = 'controller-service/src/controller/lifecycle_inbox/recording.rs'
VARIANTS = ('original', 'skip-independent-context', 'replay-falls-through',
            'stale-local-operation', 'restored')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


views = module('protocol_record_source_views', ROOT/HERE.parent/'source_views.py')
sha = views.sha
MUTATIONS = json.loads((ROOT/HERE/'mutations.json').read_text())['variants']
FAILURES = {name: set(value['expected_failures']) for name, value in MUTATIONS.items()}
FRAGMENTS = (
    ('controller-service/src/controller/lifecycle_inbox/model.rs', 'inbox-validation/generic-root.rs.in'),
    ('controller-service/src/capture.rs', 'inbox-validation/primitive-roots.rs.in'),
    ('verification/experiments/stage1-protocol/inbox-record/lib.rs', 'terminal-binding/roots.rs.in'),
    ('controller-service/src/controller/lifecycle_inbox/codec.rs', 'inbox-save/generic-root.rs.in'),
    ('controller-service/src/controller/lifecycle_inbox/model.rs', 'inbox-record/roots.rs.in'),
    ('controller-service/src/controller/lifecycle_inbox/codec.rs', 'inbox-load/generic-root.rs.in'),
    ('controller-service/src/controller/lifecycle_inbox/model.rs', 'inbox-admission/roots.rs.in'),
    ('controller-service/src/controller/lifecycle_inbox/model.rs', 'inbox-state/roots.rs.in'),
    (SOURCE, 'protocol-record/roots.rs.in'),
)


def inventory(root):
    value = json.loads((root/HERE/'inventory.json').read_text())
    for name, digest in value['production_inputs'].items():
        if sha(root/name) != digest:
            raise ValueError('reviewed production input differs: '+name)
    for name, digest in value['dependency_inputs'].items():
        if sha(root/name) != digest:
            raise ValueError('reviewed dependency contract/binding differs: '+name)
    for name, provider in value['providers'].items():
        if sha(root/provider['path']) != provider['installed_sha256']:
            raise ValueError('reviewed proof provider differs: '+name)
    for name, digest in value['same_models'].items():
        if value['providers'][name]['installed_sha256'] != digest:
            raise ValueError('correct/wrong contract differs: '+name)
    return value


def prepare(project, variant):
    if variant not in (*VARIANTS, 'correct'):
        raise ValueError('unknown caller variant: '+variant)
    fixed = inventory(project)
    names = {*fixed['production_inputs'], *(name for name, _ in FRAGMENTS)}
    before = {name: sha(project/name) for name in names}
    fragments = {}
    for name, relative in FRAGMENTS:
        path = HERE.parent/relative
        views.append(project, name, (project/path).read_text())
        fragments[str(path)] = sha(project/path)
    path = HERE.parent/'inbox-renewal-session/fragments.json'
    renewal = json.loads((project/path).read_text())
    views.append(project, 'verification/experiments/stage1-protocol/inbox-record/lib.rs',
                 renewal['renewal_roots'])
    fragments[str(path)] = sha(project/path)
    # Same ordering as the original checked scripts: format the transparent
    # projection, then change one unique expression in actual recording.rs.
    changes, _ = views.finish(project, before)
    mutation = None
    if variant in MUTATIONS:
        change = MUTATIONS[variant]
        mutation = views.edit(project, change['path'], [(change['old'], change['new'])])
        mutation['name'] = variant
        changes[change['path']] = sha(project/change['path'])
    return changes, dict(original_inputs=before, thin_root_fragments=fragments,
        mutation=mutation, production_adopted=False, original_equivalence_proved=False,
        standard_provider_connection_proved=False, generated_bodies_modified=False)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--project', type=Path, required=True)
    parser.add_argument('--variant', choices=(*VARIANTS, 'correct'), default='correct')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    changes, metadata = prepare(args.project.absolute(), args.variant)
    with args.output.open('x') as stream:
        json.dump(dict(changes=changes, preparation=metadata), stream, indent=2)
        stream.write('\n')
