"""Install this one finite JSON proof view; never compile or certify it.

The caller supplies a new empty private project. Library/tool caches and the
original kernel/service checks remain the coordinator's responsibility.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import tempfile

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/production-json')
ENCODING = 'SandboxModel/Boundaries/JsonRequest/Encoding.lean'


def digest(value):
    return hashlib.sha256(value).hexdigest()


def relative(name):
    path = Path(name)
    if path.is_absolute() or '..' in path.parts or not path.parts:
        raise ValueError('not a project-relative path: ' + name)
    return path


def mode(path):
    return '100755' if path.stat().st_mode & 0o111 else '100644'


def checked(name, expected, expected_mode=None):
    path = ROOT / relative(name)
    if not path.is_file() or path.resolve() != path:
        raise ValueError('not a regular source in this repository: ' + name)
    value = path.read_bytes()
    if digest(value) != expected:
        raise ValueError('reviewed JSON view input changed: ' + name)
    if expected_mode is not None and mode(path) != expected_mode:
        raise ValueError('reviewed JSON view input mode changed: ' + name)
    return value


def install(project):
    project = Path(project).resolve()
    if project == ROOT or ROOT.is_relative_to(project):
        raise ValueError('cannot install into the source repository or its parent')
    if project.exists() and (not project.is_dir() or any(project.iterdir())):
        raise ValueError('a new empty private project is required')
    manifest_bytes = (ROOT / HERE / 'view.json').read_bytes()
    data = json.loads(manifest_bytes)
    if data['schema'] != 1 or data['kind'] != 'finite-production-json-view-not-proof-result':
        raise ValueError('only the registered finite JSON view is supported')
    for name, sha in data['source_bindings'].items():
        checked(name, sha)
    provenance = checked(data['raw_provenance']['source'], data['raw_provenance']['sha256'])
    values = {}
    for name, row in data['files'].items():
        relative(name)
        if row['mode'] not in ('100644', '100755'):
            raise ValueError('unreviewed file mode: ' + name)
        values[name] = checked(row['source'], row['sha256'], row['mode'])
        if 'transform' in row and (name != ENCODING or
                row['transform'] != 'local_view.install:envelope-writer'):
            raise ValueError('unregistered private view transform: ' + name)
    transform_inputs = {name: checked(name, sha)
                        for name, sha in data['encoding_transform_inputs'].items()}
    project.parent.mkdir(parents=True, exist_ok=True)
    # Reuse the existing transformation itself. Its extraction-root and template
    # setup stays in scratch; only Encoding's exact value enters this project.
    with tempfile.TemporaryDirectory(prefix='json-encoding-', dir=project.parent) as folder:
        scratch = Path(folder)
        for name, value in transform_inputs.items():
            path = scratch / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(value)
        script = scratch / 'verification/experiments/stage1-protocol/local_view.py'
        spec = importlib.util.spec_from_file_location('original_json_local_view', script)
        view = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(view)
        view.install(scratch, 'envelope-writer')
        values[ENCODING] = (scratch / ENCODING).read_bytes()
    if digest(values[ENCODING]) != data['files'][ENCODING]['installed_sha256']:
        raise ValueError('original Encoding transformation output changed')
    # Compare the actual consumed module-to-source mapping. This does not assert
    # that the historical broader Lake file has the current template's bytes.
    libraries = {}
    for block in values['lakefile.toml'].decode().split('[[lean_lib]]')[1:]:
        block = block.split('[[')[0]
        name = re.search(r'^name = "([^"]+)"$', block, re.M)
        src = re.search(r'^srcDir = "([^"]+)"$', block, re.M)
        if name is None or name[1] in libraries:
            raise ValueError('unreviewed library declaration')
        libraries[name[1]] = src[1] if src else '.'
    if any(libraries.get(name) != source
           for name, source in data['consumed_library_sources'].items()):
        raise ValueError('consumed JSON module source mapping changed')
    project.mkdir(exist_ok=True)
    changed, installed_modes = {}, {}
    for name, value in values.items():
        path = project / name
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open('xb') as stream:
            stream.write(value)
        path.chmod(0o755 if data['files'][name]['mode'] == '100755' else 0o644)
        changed[name] = digest(value)
        installed_modes[name] = mode(path)
    record = dict(kind='production-json-view-materialization-not-proof',
                  changed=changed, view_sha256=digest(manifest_bytes),
                  installed_file_count=len(changed),
                  installed_modes=installed_modes,
                  raw_provenance_sha256=digest(provenance),
                  source_bindings=data['source_bindings'],
                  consumed_library_sources=data['consumed_library_sources'],
                  unconsumed_library_declarations=data['unconsumed_library_declarations'],
                  seed='SandboxModel.Lifecycle.Protocol.ProductionEnvelope',
                  source_admission_only=True, formal_acceptance=False)
    receipt = project / 'artifacts/production-json-materialization.json'
    receipt.parent.mkdir(parents=True, exist_ok=True)
    with receipt.open('x') as stream:
        json.dump(record, stream, indent=2)
        stream.write('\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--project', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(install(args.project), indent=2))
