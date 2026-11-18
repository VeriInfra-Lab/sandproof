"""Validate the candidate registration against the original V3 proof closures."""
import argparse
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[4]
STAGE = Path('verification/experiments/stage1-protocol')
PARENT = STAGE / 'unload-parent-link'
COLLECTION = STAGE / 'unload-parent-collections'
SUPPORT = ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py',
           'tools/proof_services.py', 'scripts/lean-mcp-server.sh')
AUDIT = re.compile(rb'^#audit_controller_proof (\S+)$', re.M)


def read_regular(root, name):
    relative = Path(name)
    if relative.is_absolute() or '..' in relative.parts or str(relative) != str(name):
        raise ValueError(f'noncanonical proof input: {name}')
    path = root / relative
    if path.resolve() != path or not path.is_file():
        raise ValueError(f'missing or linked proof input: {name}')
    return path.read_bytes()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def exact_file(root, name, expected):
    data = read_regular(root, name)
    if sha(data) != expected:
        raise ValueError(f'changed proof input: {name}')
    return data


def audits(data):
    return [m.decode() for m in AUDIT.findall(data)]


def candidate_view(root, package, formal_root, recipe, modules):
    own = recipe['own_modules']
    declared = recipe['audited_declarations']
    if (recipe['root'] != formal_root or len(modules) != recipe['physical_files']
            or formal_root + '.lean' not in modules or len(own) != len(set(own))
            or not set(own) <= set(modules) or len(declared) != len(set(declared))):
        raise ValueError(f'wrong proof root, file count, or duplicate registration: {formal_root}')
    by_file = {}
    for name in own:
        data = exact_file(root, package / (name + '.in'), modules[name])
        by_file[name] = audits(data)
    if {name for entries in by_file.values() for name in entries} != set(declared):
        raise ValueError(f'candidate declarations differ from frozen templates: {formal_root}')
    return dict(root=formal_root, modules=modules, own_audits_by_file=by_file)


def project_lean_files(project):
    files = set()
    for directory, subdirectories, names in project.walk():
        if directory == project:
            subdirectories[:] = [name for name in subdirectories if name not in {'.lake', '.tools', 'artifacts'}]
        files.update((directory / name).relative_to(project).as_posix()
                     for name in names if name.endswith('.lean'))
    return files


def check_project(project, variant, view, support):
    plan = json.loads(read_regular(project, 'plan.json'))
    modules = view['modules']
    if (plan.get('schema') != 1 or plan.get('development_only') is not True
            or plan.get('public_acceptance') is not False or plan.get('variant') != variant
            or plan.get('roots') != [view['root']]
            or {name: row['sha256'] for name, row in plan['modules'].items()} != modules):
        raise ValueError(f'project plan differs from the frozen view: {view["root"]}')
    order = [name.replace('.', '/') + '.lean' for name in plan['build_order']]
    if len(order) != len(modules) or set(order) != set(modules):
        raise ValueError(f'project build order does not cover the exact files once: {view["root"]}')
    if project_lean_files(project) != set(modules):
        raise ValueError(f'physical Lean file set differs from the frozen view: {view["root"]}')
    for name, expected in support.items():
        exact_file(project, name, expected)
    return {name: entries for name, expected in modules.items()
            if (entries := audits(exact_file(project, name, expected)))}


def collect(root, projects):
    parent_file = PARENT / 'input-closure.json'
    collection_file = COLLECTION / 'input-closure.json'
    parent_bytes = read_regular(root, parent_file)
    collection_bytes = read_regular(root, collection_file)
    parent = json.loads(parent_bytes)
    collection = json.loads(collection_bytes)
    if collection['parent_freeze'] != str(parent_file) or collection['parent_freeze_sha256'] != sha(parent_bytes):
        raise ValueError('collection does not name the exact original parent freeze')
    for freeze in (parent, collection):
        for name, expected in freeze['repository_inputs'].items():
            exact_file(root, name, expected)

    parent_recipe = json.loads(read_regular(root, PARENT / 'registration-candidate.json'))
    collection_recipe = json.loads(read_regular(root, COLLECTION / 'registration-candidate.json'))['variants']
    views = {
        'parent': candidate_view(root, PARENT, 'UnloadParentLink',
                                 dict(parent_recipe, root=parent_recipe['formal_root']),
                                 parent['materialized_modules']),
    }
    for variant, formal_root in (('correct', 'UnloadCollectionLink'), ('attempt-zero', 'CollectionCounter')):
        views[variant] = candidate_view(root, COLLECTION, formal_root, collection_recipe[variant],
                                        collection['variants'][variant]['materialized_modules'])
    for package, labels in ((PARENT, ('parent',)), (COLLECTION, ('correct', 'attempt-zero'))):
        own = {name + '.in' for label in labels for name in views[label]['own_audits_by_file']}
        actual = {path.name for path in (root / package).glob('*.lean.in')}
        if own != actual:
            raise ValueError(f'unregistered or missing candidate proof template: {package}')

    support = {name: parent['repository_inputs'][name] for name in SUPPORT}
    for label, project in projects.items():
        if project is not None:
            views[label]['physical_audits_by_file'] = check_project(
                project.resolve(strict=True), None if label == 'parent' else label, views[label], support)
    return dict(kind='unload-registration-preflight', public_acceptance=False, proof_executed=False,
                parent_freeze_sha256=sha(parent_bytes), collection_freeze_sha256=sha(collection_bytes),
                views=views)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    for label in ('parent', 'correct', 'attempt-zero'):
        parser.add_argument('--' + label + '-project', type=Path)
    args = parser.parse_args()
    report = collect(ROOT, {'parent': args.parent_project, 'correct': args.correct_project,
                            'attempt-zero': args.attempt_zero_project})
    with args.output.open('x') as stream:
        json.dump(report, stream, indent=2)
        stream.write('\n')
