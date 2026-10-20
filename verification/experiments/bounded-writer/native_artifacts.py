"""Bind native libraries and test executables to this variant's Cargo source IDs."""
from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path
import shutil

OWN = {
    'sandproof-controller-service': ('controller-service/Cargo.toml', 'controller-service/src/lib.rs'),
    'sandproof-internal': ('sandbox-internal/Cargo.toml', 'sandbox-internal/src/lib.rs'),
    'sandproof-bounded-writer-extract': ('verification/extract-bounded-writer/Cargo.toml',
                                       'verification/extract-bounded-writer/src/lib.rs'),
}
TESTS = {
    'original_writer': 'verification/extract-bounded-writer/tests/original_writer.rs',
    'candidate_writer': 'verification/extract-bounded-writer/tests/candidate_writer.rs',
    'sandproof_bounded_writer_extract': 'verification/extract-bounded-writer/src/lib.rs',
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def package_identities(root, metadata):
    identities = {}
    for name, (manifest, source) in OWN.items():
        expected_manifest = (root / manifest).resolve(strict=True)
        expected_source = (root / source).resolve(strict=True)
        found = [p for p in metadata['packages'] if p['name'] == name
                 and Path(p['manifest_path']).resolve(strict=True) == expected_manifest]
        if len(found) != 1:
            raise ValueError('variant Cargo package identity missing/ambiguous: ' + name)
        package = found[0]
        libraries = [t for t in package['targets'] if 'lib' in t['kind']]
        if len(libraries) != 1 or Path(libraries[0]['src_path']).resolve(strict=True) != expected_source:
            raise ValueError('variant library source mismatch: ' + name)
        identities[package['id']] = dict(name=name, manifest_path=str(expected_manifest),
            src_path=str(expected_source), target_name=libraries[0]['name'])
    by_name = {p['name'].replace('-', '_'): package_id for package_id, p in identities.items()}
    nodes = {n['id']: n for n in metadata['resolve']['nodes']}
    for package_id in identities:
        for dep in nodes[package_id]['deps']:
            if dep['name'] in by_name and dep['pkg'] != by_name[dep['name']]:
                raise ValueError('Cargo dependency resolves to a different source variant')
    wrapper = by_name['sandproof_bounded_writer_extract']
    linked = {d['pkg'] for d in nodes[wrapper]['deps']}
    if not {by_name['sandproof_controller_service'], by_name['sandproof_internal']} <= linked:
        raise ValueError('test wrapper does not depend on the exact original library instances')
    return identities


def build_artifacts(root, target, log, identities, required_tests):
    """All changed libraries AND test executables must come from a fresh compile."""
    libraries, tests = {}, {}
    owned_targets = {p['target_name'] for p in identities.values()} | set(TESTS)
    for line in log.read_text().splitlines():
        try:
            item = json.loads(line)
        except ValueError:
            continue
        if item.get('reason') != 'compiler-artifact':
            continue
        package = identities.get(item['package_id'])
        if package is None:
            if item.get('target', {}).get('name') in owned_targets:
                raise ValueError('same-name library/test came from another package/source ID')
            continue
        if item.get('manifest_path') != package['manifest_path']:
            raise ValueError('artifact manifest is not the current variant')
        name = item['target']['name']
        source = Path(item['target']['src_path']).resolve(strict=True)
        expected = (root / TESTS[name]).resolve(strict=True) if item.get('executable') else Path(package['src_path'])
        if source != expected:
            raise ValueError('artifact library/test src_path is not the current variant')
        if item.get('fresh') is not False:
            raise ValueError('changed library/test was cached rather than freshly compiled')
        filenames = []
        for name_on_disk in item['filenames']:
            path = Path(name_on_disk).resolve(strict=True)
            path.relative_to(target.resolve(strict=True))
            if not path.is_file():
                raise ValueError('compiler artifact missing')
            filenames.append(dict(path=str(path), sha256=sha(path)))
        row = dict(package_id=item['package_id'], manifest_path=item['manifest_path'],
                   src_path=str(source), fresh=False, filenames=filenames, target=name)
        if item.get('executable'):
            executable = Path(item['executable']).resolve(strict=True)
            executable.relative_to(target.resolve(strict=True))
            row.update(executable=str(executable), binary_sha256=sha(executable))
            tests[name] = row
        elif 'lib' in item['target']['kind']:
            libraries[package['name']] = row
    if set(libraries) != set(OWN) or set(tests) != set(required_tests):
        raise ValueError('missing/extra fresh variant libraries or test executables')
    return dict(libraries=libraries, tests=tests)


def preserve_file(source, destination):
    """Keep immutable evidence outside the target before selective cargo clean."""
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        raise ValueError('refuse to replace compiled evidence')
    # Copy the few owned artifacts; dependencies and the large target are not copied.
    shutil.copy2(source, destination)
    if sha(source) != sha(destination):
        raise ValueError('compiled evidence changed during preservation')
    return destination


def quarantine_owned_cache(target, output):
    """Only this interface's three owned packages; dependency caches stay intact."""
    target = target.resolve(strict=True)
    fingerprint = re.compile('(?:' + '|'.join(re.escape(name) for name in OWN) + r')-[0-9a-f]{16}')
    stems = tuple(name.replace('-', '_') for name in OWN)
    deps_prefixes = tuple(s + '-' for s in stems) + tuple('lib' + s + '-' for s in stems)
    deps_prefixes += ('original_writer-', 'candidate_writer-')
    candidates = []
    for profile in (target / 'debug', target / 'x86_64-unknown-linux-gnu/debug'):
        if not profile.is_dir():
            continue
        fingerprints = profile / '.fingerprint'
        if fingerprints.is_dir():
            candidates += [p for p in fingerprints.iterdir() if fingerprint.fullmatch(p.name) is not None]
        deps = profile / 'deps'
        if deps.is_dir():
            candidates += [p for p in deps.iterdir() if p.name.startswith(deps_prefixes)]
        for stem in stems:
            candidates += [p for p in profile.glob('lib' + stem + '.*') if p.is_file()]
        incremental = profile / 'incremental'
        if incremental.is_dir():
            candidates += [p for p in incremental.iterdir() if p.name.startswith(tuple(s + '-' for s in stems))]
    moved = []
    for path in sorted(set(candidates)):
        path.resolve(strict=True).relative_to(target)
        relative = path.relative_to(target)
        destination = output / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        if destination.exists():
            raise ValueError('existing cache quarantine destination')
        shutil.move(str(path), str(destination))
        moved.append(str(relative))
    return moved
