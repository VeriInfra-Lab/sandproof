"""Materialize exact upstream proof closures and the current source-row capture."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
STAGE = HERE.parent


def sha(data):
    return hashlib.sha256(data).hexdigest()


def checked(root, name, expected):
    path = (root / name).resolve(strict=True)
    if not path.is_relative_to(root) or not path.is_file():
        raise ValueError('unsafe proof input: ' + name)
    data = path.read_bytes()
    if sha(data) != expected:
        raise ValueError('changed proof input: ' + name)
    return data


def apply_view(data, view):
    """Apply exact, reversible edits to handwritten adapters or attributes."""
    if sha(data) != view['raw_sha256']:
        raise ValueError('adapter original differs')
    previous = 0
    for edit in view['edits']:
        offset, before = edit['offset'], edit['before'].encode()
        if offset < previous or data[offset:offset + len(before)] != before:
            raise ValueError('adapter edit differs')
        previous = offset + len(before)
    output = data
    for edit in reversed(view['edits']):
        offset, before, after = edit['offset'], edit['before'].encode(), edit['after'].encode()
        output = output[:offset] + after + output[offset + len(before):]
    restored = output
    for edit in view['edits']:
        offset, before, after = edit['offset'], edit['before'].encode(), edit['after'].encode()
        if restored[offset:offset + len(after)] != after:
            raise ValueError('adapter inverse differs')
        restored = restored[:offset] + before + restored[offset + len(after):]
    if restored != data or sha(output) != view['view_sha256']:
        raise ValueError('adapter inverse or output differs')
    return output


def original_inputs():
    files, provenance = {}, {}
    isolation = json.loads((STAGE / 'unload-base-provenance/isolation.json').read_bytes())
    spec = importlib.util.spec_from_file_location('base_materializer', STAGE / 'unload-base-provenance/materialize.py')
    tool = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(tool)

    def put(name, data, origin):
        if name in files and files[name] != data:
            raise ValueError('incompatible shared module: ' + name)
        files[name] = data
        provenance.setdefault(name, []).append(origin)

    for package in ('unload-base-provenance', 'unload-continued-observation', 'unload-stored-options'):
        directory = STAGE / package
        inventory = json.loads((directory / 'sources.json').read_bytes())
        for name, expected in inventory['tool_inputs'].items():
            put(name, checked(ROOT, name, expected), name)
        for module, entry in inventory['modules'].items():
            key, root = ('package', directory) if 'package' in entry else ('repository', ROOT)
            data = checked(root, entry[key], entry['sha256'])
            if module in isolation:
                data = tool.apply_isolation(data, isolation[module])
            put(module.replace('.', '/') + '.lean', data, str((root / entry[key]).relative_to(ROOT)))

    rows = STAGE / 'original-source-rows'
    spec = json.loads((rows / 'execution-inputs.json').read_bytes())
    for name, entry in spec['materialize'].items():
        module = Path(name)
        if 'verification/aeneas/' in name:
            module = Path(*module.parts[-2:])
        put(str(module), checked(ROOT, entry['source'], entry['sha256']), entry['source'])
    generated = ROOT / 'verification/aeneas/stage1-protocol/unload-parent-link/SandproofOriginalSourceRows'
    put('SandproofOriginalSourceRows/Funs.lean', (generated / 'Funs.lean').read_bytes(), str((generated / 'Funs.lean').relative_to(ROOT)))
    # The #771 type view has only isolated helper discriminant attributes. Check
    # all declarations against the new raw tool output before using that view.
    def declarations(data):
        text = re.sub(rb'/\-.*?\-/', b'', data, flags=re.S)
        text = text.replace(b'discriminant isize, ', b'').replace(b'@[discriminant isize]\n', b'')
        return text
    raw = (generated / 'Types.lean').read_bytes()
    public = files['SandproofOriginalSourceRows/Types.lean']
    if declarations(raw) != declarations(public):
        raise ValueError('current source-row types differ from the public shared view')

    for name, entry in json.loads((HERE / 'extra-inputs.json').read_bytes()).items():
        put(name, checked(ROOT, entry['source'], entry['sha256']), entry['source'])
    for name, view in json.loads((HERE / 'views.json').read_bytes()).items():
        if name == 'SandproofSourceLive/Types.lean':
            files[name] = tool.apply_isolation(files[name], view)
        else:
            files[name] = apply_view(files[name], view)
        provenance[name].append('unload-parent-link/views.json:' + name)
    for source in sorted(HERE.glob('*.lean.in')):
        put(source.name.removesuffix('.in'), source.read_bytes(), str(source.relative_to(ROOT)))
    return files, provenance


def write_project(output, files, provenance, packages, lean_mcp, *, frozen=None, identity=None):
    roots = ['UnloadParentLink']
    modules = {name.removesuffix('.lean').replace('/', '.'): data
               for name, data in files.items() if name.endswith('.lean')}
    order, active, seen = [], set(), set()

    def visit(name):
        if name in seen or name.split('.')[0] in {'Aeneas', 'Init', 'Lean', 'Std', 'Mathlib', 'Batteries'}:
            return
        if name in active or name not in modules:
            raise ValueError('missing or cyclic module: ' + name)
        active.add(name)
        for statement in re.findall(rb'(?m)^(?:public |private )?import[ \t]+([^\n]+)', modules[name]):
            for dependency in statement.split(b'--', 1)[0].split():
                visit(dependency.decode())
        active.remove(name)
        seen.add(name)
        order.append(name)

    for name in roots:
        visit(name)
    # Only emit the actual transitive closure. Extra historical inventory entries
    # are checked above, but cannot silently become unchecked materialized files.
    files = {name: data for name, data in files.items()
             if not name.endswith('.lean') or name.removesuffix('.lean').replace('/', '.') in seen}
    if frozen is not None:
        actual = {name: sha(data) for name, data in files.items() if name.endswith('.lean')}
        if actual != frozen['materialized_modules']:
            raise ValueError('materialized import closure differs from the checked candidate')
    heads = sorted({name.split('.')[0] for name in seen})
    lake = 'name = "unload-parent-link"\n\n[[require]]\nname = "aeneas"\ngit = { url = "https://github.com/AeneasVerif/aeneas", subDir = "backends/lean" }\nrev = "505b6ca35217e7be5c96c3e2f8045edfbdf47291"\n'
    lake += ''.join('\n[[lean_lib]]\nname = "' + head + '"\n' for head in heads)
    files['lakefile.toml'] = lake.encode()
    for name in ('tools/lean_session.py', 'tools/proof_services.py', 'scripts/lean-mcp-server.sh'):
        data = (ROOT / name).read_bytes()
        if name in files and files[name] != data:
            raise ValueError('incompatible shared tool input: ' + name)
        files[name] = data
    output.mkdir(mode=0o700)
    for name, data in files.items():
        target = output / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    (output / '.lake').mkdir()
    (output / '.lake/packages').symlink_to(packages, target_is_directory=True)
    (output / '.tools').mkdir()
    (output / '.tools/lean-mcp').symlink_to(lean_mcp, target_is_directory=True)
    (output / 'artifacts').mkdir()
    plan = dict(schema=1, development_only=True,
        public_acceptance=False, build_order=order, roots=roots,
        modules={name: dict(sha256=sha(data), source=provenance[name])
                 for name, data in files.items() if name.endswith('.lean')})
    if identity is not None:
        plan.update(source_commit=identity['head'], source_tree=identity['tree'])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2)+'\n')


def materialize(output):
    manifest = HERE / 'input-closure.json'
    frozen = json.loads(manifest.read_bytes()) if manifest.exists() else None
    if frozen is not None:
        for name, expected in frozen['repository_inputs'].items():
            checked(ROOT, name, expected)
    files, provenance = original_inputs()
    write_project(output, files, provenance, (ROOT / '.lake/packages').resolve(strict=True),
                  (ROOT / '.tools/lean-mcp').resolve(strict=True), frozen=frozen)


def materialize_current(output, revision, identity):
    """Compose the current generated reader with the original leaf proofs."""
    plan = json.loads((revision / 'plan.json').read_bytes())
    if (plan['source_commit'], plan['source_tree'], plan['roots']) != (
            identity['head'], identity['tree'], ['RevisionParentCall', 'ParentRejects']):
        raise ValueError('current revision project differs from the admitted extraction')
    files, provenance = original_inputs()
    generated_modules = ('SandproofUnloadBaseSnapshot', 'SandproofUnloadFirstObservation')
    dependencies = [f'{module}/{name}.lean' for module in generated_modules
                    for name in ('Types', 'Funs', 'TypesExternal', 'FunsExternal')]
    dependencies += ['RevisionReadErrors.lean', 'RevisionReadSuccess.lean']
    for name in dependencies:
        files[name] = checked(revision, name, plan['modules'][name]['sha256'])
        provenance[name] = ['current revision capture:' + name]
    overlay = HERE / 'current-readiness-views.json'
    for name, view in json.loads(overlay.read_bytes()).items():
        files[name] = apply_view(files[name], view)
        provenance[name].append(overlay.name + ':' + name)
    write_project(output, files, provenance, (revision / '.lake/packages').resolve(strict=True),
                  (revision / '.tools/lean-mcp').resolve(strict=True), identity=identity)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--revision-project', type=Path)
    parser.add_argument('--source-commit')
    args = parser.parse_args()
    if bool(args.revision_project) != bool(args.source_commit):
        parser.error('the current route requires the actual revision project and its exact source commit')
    if args.revision_project is None:
        materialize(args.output.resolve())
    else:
        from check_revision_pair import source_identity
        materialize_current(args.output.resolve(), args.revision_project.resolve(strict=True),
                            source_identity(args.source_commit, args.source_commit))
