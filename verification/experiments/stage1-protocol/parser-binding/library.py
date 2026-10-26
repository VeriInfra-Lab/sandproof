"""Exact source/feature admission for the finite parser use-site review.

This checksum admission is not a proof of Serde, scanning or derive semantics.
"""
import hashlib
import json
from pathlib import Path
import subprocess

SOURCES = {
    'serde': ['src/private/de.rs'],
    'serde_core': ['src/de/mod.rs', 'src/de/value.rs', 'src/de/impls.rs', 'src/private/content.rs'],
    'serde_derive': ['src/de.rs', 'src/de/struct_.rs', 'src/de/enum_.rs',
                     'src/de/enum_internally.rs', 'src/de/enum_adjacently.rs',
                     'src/de/enum_untagged.rs', 'src/de/identifier.rs'],
    'serde_json': ['src/de.rs', 'src/read.rs', 'src/error.rs', 'src/lib.rs'],
}
MANIFESTS = {
    'production': 'Cargo.toml',
    'extracted_wrappers': 'verification/experiments/stage1-protocol/envelope-bytes/Cargo.toml',
    'native_parser': 'verification/experiments/stage1-protocol/parser-binding/Cargo.toml',
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def capture(root):
    result = dict(features={}, sources={})
    for role, manifest in MANIFESTS.items():
        metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--format-version', '1',
            '--locked', '--offline', '--manifest-path', str(root / manifest)], text=True))
        nodes = {p['id']: p['features'] for p in metadata['resolve']['nodes']}
        packages = {p['name']: p for p in metadata['packages'] if p['name'] in SOURCES}
        if set(packages) != set(SOURCES):
            raise ValueError('incomplete concrete parser dependency graph')
        result['features'][role] = {}
        for name, package in packages.items():
            result['features'][role][name] = dict(version=package['version'], features=sorted(nodes[package['id']]))
            for relative in SOURCES[name]:
                path = Path(package['manifest_path']).parent / relative
                key = name + '-' + package['version'] + '/' + relative
                if not path.is_file() or path.is_symlink():
                    raise ValueError('missing regular parser source: ' + key)
                value = sha(path)
                if key in result['sources'] and result['sources'][key] != value:
                    raise ValueError('compiled parser sources differ: ' + key)
                result['sources'][key] = value
    return result


def check(candidate, root):
    if (candidate.get('a6_instance_adopted') is not False or
            candidate.get('whole_parser_refinement_proved') is not False):
        raise ValueError('this composition does not certify a complete parser')
    for name, wanted in candidate['inputs'].items():
        if sha(root / name) != wanted:
            raise ValueError('source-reviewed parser input changed: ' + name)
    actual = capture(root)
    if actual != candidate['library_inputs']:
        raise ValueError('parser source/version/features changed')
    if (set(actual['features']['production']['serde_json']['features']) != {'default', 'std', 'raw_value'} or
            actual['features']['native_parser'] != actual['features']['production'] or
            set(actual['features']['extracted_wrappers']['serde_json']['features']) != {'default', 'std'}):
        raise ValueError('unexpected parser feature split')
    return actual
