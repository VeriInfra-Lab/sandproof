"""Restore fix-git's original clone from its pinned upstream image, in the guest.

Only the two original Git pack files are imported. Image programs, Git config,
hooks, worktree files, and the image's already-initialized reflog are not used.
The unchanged setup.sh then creates the task state from the original clone tip.
"""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import urllib.parse
import urllib.request

REPOSITORY = 'alexgshaw/fix-git'
MANIFEST = 'sha256:61e431c00c58df652287aadce5457634d9f9330cfdd153ebdf2802df0d540119'
LAYER_SHA256 = '39467b0699441e2a8aad92554e7119f6f277847f43bebace4865893335dc28d1'
LAYER_BYTES = 62156524
PACK = 'pack-395fa4428d662a73175f4f2357b3603f32c56bbd'
PACK_FILES = {
    PACK + '.idx': (16024, '509c72deec67248b57347f8a2307a3e104bb009e3cbfa55d4b82b47e0b235a6d'),
    PACK + '.pack': (32224108, '282f3b2b258f813a9ff248beed32df27e547755fa780069982d2b18147c93fca'),
}
CLONE_TIP = 'b0d59cded7105fea91a4ea0008098295d8895786'
RESET_COMMIT = 'd7d3e4ba9350f634d92d39ace2b471433ee57d50'
SETUP_SHA256 = '99125cf2e362f2f4a864ba2f55c2f0e9f0aa0641fed588cef588184fcc192de2'
SOURCE_URL = 'https://github.com/TheMikeMerrill/personal-site.git'


def copy_verified(source, destination, size, expected):
    digest = hashlib.sha256()
    count = 0
    with destination.open('xb') as output:
        while chunk := source.read(min(1024**2, size + 1 - count)):
            count += len(chunk)
            if count > size:
                raise ValueError('oversized input')
            digest.update(chunk)
            output.write(chunk)
    if count != size or digest.hexdigest() != expected:
        raise ValueError('input size or SHA-256 mismatch')


def download_layer(destination):
    query = urllib.parse.urlencode({'service': 'registry.docker.io',
                                   'scope': 'repository:' + REPOSITORY + ':pull'})
    with urllib.request.urlopen('https://auth.docker.io/token?' + query, timeout=30) as response:
        raw = response.read(65537)
    if len(raw) > 65536:
        raise ValueError('oversized registry token response')
    token = json.loads(raw)['token']
    request = urllib.request.Request(
        'https://registry-1.docker.io/v2/' + REPOSITORY + '/blobs/sha256:' + LAYER_SHA256,
        headers={'Authorization': 'Bearer ' + token})
    with urllib.request.urlopen(request, timeout=30) as response:
        copy_verified(response, destination, LAYER_BYTES, LAYER_SHA256)


def extract_packs(archive, destination):
    """Read allowlisted regular members into fixed basenames, never extract a tar."""
    prefix = 'app/personal-site/.git/objects/pack/'
    selected = {prefix + name: name for name in PACK_FILES}
    seen = set()
    with tarfile.open(archive, mode='r:gz') as stream:
        for member in stream:
            name = selected.get(member.name)
            if name is None:
                continue
            if name in seen or not member.isfile() or member.size != PACK_FILES[name][0]:
                raise ValueError('invalid or duplicate Git pack member')
            size, digest = PACK_FILES[name]
            with stream.extractfile(member) as content:
                copy_verified(content, destination / name, size, digest)
            seen.add(name)
    if seen != set(PACK_FILES):
        raise ValueError('missing Git pack member')


def make_mirror(archive, mirror):
    env = {**os.environ, 'GIT_CONFIG_GLOBAL': '/dev/null', 'GIT_CONFIG_NOSYSTEM': '1'}
    subprocess.run(['git', 'init', '--bare', '--initial-branch=master', str(mirror)],
                   env=env, check=True)
    extract_packs(archive, mirror / 'objects/pack')
    git = ['git', '-c', 'core.hooksPath=/dev/null', '-c', 'core.fsmonitor=false',
           '--git-dir=' + str(mirror)]
    for commit in (CLONE_TIP, RESET_COMMIT):
        subprocess.run(git + ['cat-file', '-e', commit + '^{commit}'], env=env, check=True)
    subprocess.run(git + ['update-ref', 'refs/heads/master', CLONE_TIP], env=env, check=True)
    subprocess.run(git + ['fsck', '--strict', '--no-reflogs'], env=env, check=True)


def main():
    setup = Path('/app/setup.sh')
    if hashlib.sha256(setup.read_bytes()).hexdigest() != SETUP_SHA256:
        raise ValueError('setup.sh differs from the fixed original task')
    with tempfile.TemporaryDirectory(prefix='sandproof-fix-git-') as temp:
        root = Path(temp)
        archive = root / 'initialization.tar.gz'
        download_layer(archive)
        mirror = root / 'source.git'
        make_mirror(archive, mirror)
        # Only this setup process sees the rewrite. Its original clone/reset/
        # detached-commit/checkout sequence is executed without editing the script.
        env = {**os.environ, 'GIT_CONFIG_COUNT': '1',
               'GIT_CONFIG_KEY_0': 'url.' + mirror.as_uri() + '.insteadOf',
               'GIT_CONFIG_VALUE_0': SOURCE_URL, 'GIT_TERMINAL_PROMPT': '0'}
        subprocess.run(['bash', str(setup)], cwd='/app', env=env, check=True)
    proof = {'source_url': SOURCE_URL, 'upstream_image': REPOSITORY + ':20251031',
             'manifest_digest': MANIFEST, 'layer_sha256': LAYER_SHA256,
             'layer_bytes': LAYER_BYTES, 'pack_files': PACK_FILES,
             'original_clone_tip': CLONE_TIP, 'reset_commit': RESET_COMMIT,
             'original_setup_sha256': SETUP_SHA256,
             'scope': 'Original Git source objects only; unchanged setup, oracle, verifier and limits.'}
    Path('/logs/verifier/fix-git-source.json').write_text(json.dumps(proof, indent=2) + '\n')
    print(json.dumps(proof), flush=True)


if __name__ == '__main__':
    main()
