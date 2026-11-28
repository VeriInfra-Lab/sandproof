"""Bind the exact fixed serializer use sites and compiled feature sets."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
MANIFESTS = {
    'production': 'Cargo.toml',
    'vec_calls': 'verification/experiments/stage1-protocol/envelope-bytes/Cargo.toml',
    'writer_calls': 'verification/experiments/stage1-protocol/envelope-writer/Cargo.toml',
    'native_full_fields': 'verification/experiments/stage1-protocol/envelope-schema/Cargo.toml',
}
SOURCES = {
    'serde': ['src/private/ser.rs'],
    'serde_core': ['src/ser/mod.rs', 'src/ser/impls.rs', 'src/crate_root.rs'],
    'serde_derive': ['src/ser.rs'],
    'serde_json': ['src/ser.rs', 'src/error.rs', 'src/lib.rs'],
    'itoa': ['src/lib.rs', 'src/udiv128.rs'],
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def capture(root=ROOT):
    output = dict(features={}, sources={}, standard_library={})
    for role, manifest in MANIFESTS.items():
        metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--format-version', '1',
            '--locked', '--offline', '--manifest-path', str(root / manifest)], text=True))
        packages = {p['name']: p for p in metadata['packages'] if p['name'] in SOURCES}
        if set(packages) != set(SOURCES):
            raise ValueError('incomplete concrete serializer dependency graph')
        nodes = {n['id']: sorted(n['features']) for n in metadata['resolve']['nodes']}
        output['features'][role] = {name: dict(version=p['version'], features=nodes[p['id']])
                                    for name, p in sorted(packages.items())}
        for name, package in packages.items():
            for relative in SOURCES[name]:
                path = Path(package['manifest_path']).parent / relative
                key = name + '-' + package['version'] + '/' + relative
                if path.is_symlink() or not path.is_file():
                    raise ValueError('missing regular actual serializer source: ' + key)
                value = sha(path)
                if key in output['sources'] and output['sources'][key] != value:
                    raise ValueError('different compiled copies of serializer dependency')
                output['sources'][key] = value
    for toolchain in ('1.97.0', 'nightly-2026-02-07'):
        sysroot = Path(subprocess.check_output(['rustc', '+' + toolchain, '--print', 'sysroot'], text=True).strip())
        output['standard_library'][toolchain] = {}
        for relative in ('std/src/io/mod.rs', 'std/src/io/impls.rs',
                         'core/src/convert/mod.rs', 'core/src/result.rs'):
            path = sysroot / 'lib/rustlib/src/rust/library' / relative
            if path.is_symlink() or not path.is_file():
                raise ValueError('missing actual fixed standard library source')
            output['standard_library'][toolchain][relative] = sha(path)
    return output


def check(candidate, root=ROOT):
    if (candidate.get('a6_instance_adopted') is not False or
            candidate.get('new_library_refinement_theorems') != 0):
        raise ValueError('this use-site candidate cannot claim library refinement/adoption')
    for name, expected in candidate['inputs'].items():
        if sha(root / name) != expected:
            raise ValueError('bound original source/proof changed: ' + name)
    actual = capture(root)
    if actual != candidate['library_inputs']:
        raise ValueError('fixed serializer source/version/features changed')
    # raw_value is enabled in the actual production/Writer graph. The special
    # case is selected by a reserved struct name, not any ordinary object key.
    for role, packages in actual['features'].items():
        if packages['serde_json']['version'] != '1.0.142':
            raise ValueError('unreviewed JSON engine')
        features = set(packages['serde_json']['features'])
        if features - {'default', 'std', 'raw_value'}:
            raise ValueError('unreviewed JSON renderer feature')
    names = candidate['ordinary_struct_names']
    schema = json.loads((root /
        'verification/experiments/stage1-protocol/envelope-schema/inventory.json').read_text())
    if names != list(schema['structs']) + ['LifecycleOperation', 'ExpectedSession']:
        raise ValueError('ordinary name list differs from complete existing schema')
    if len(names) != len(set(names)) or any(name.startswith('$serde_json::') for name in names):
        raise ValueError('reserved/duplicate serialized struct name')
    return actual


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--candidate', type=Path, default=HERE / 'candidate.json')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    value = check(json.loads(args.candidate.read_text()))
    with args.output.open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')
    print('Fixed library use sites checked; no library refinement or shared acceptance claimed.')
